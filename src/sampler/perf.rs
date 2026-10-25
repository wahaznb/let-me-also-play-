//! Linux `perf_event_open` sampling with no external `perf` binary.
//!
//! The sampler uses one per-thread software CPU-clock event per thread. It
//! reads only user-space instruction pointers (`exclude_kernel`), so it works
//! with `kernel.perf_event_paranoid=2` and does not require root on the target
//! machine. Each sample is a *self* sample: it says "this thread was executing
//! at this IP", not "this function or its callees caused the frame time".
//!
//! The ring buffer is the standard `perf_event_mmap_page` layout. We read the
//! head with an acquire fence, copy records across the ring wrap, and publish
//! the tail with a release fence before advancing it.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io;
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use std::time::{Duration, Instant};

const PERF_TYPE_SOFTWARE: u32 = 1;
const PERF_COUNT_SW_CPU_CLOCK: u64 = 0;
const PERF_SAMPLE_IP: u64 = 1 << 0;
const PERF_FLAG_FD_CLOEXEC: libc::c_ulong = 1 << 3;
const PERF_EVENT_IOC_ENABLE: libc::c_ulong = 0x2400;
const PERF_EVENT_IOC_DISABLE: libc::c_ulong = 0x2401;
const PERF_RECORD_LOST: u32 = 2;
const PERF_RECORD_SAMPLE: u32 = 9;

// Bit positions in perf_event_attr's first u64 bitfield.
const ATTR_DISABLED: u64 = 1 << 0;
const ATTR_EXCLUDE_KERNEL: u64 = 1 << 5;
const ATTR_EXCLUDE_HV: u64 = 1 << 6;

const MMAP_DATA_HEAD_OFFSET: usize = 1024;
const MMAP_DATA_TAIL_OFFSET: usize = 1032;
const MMAP_DATA_OFFSET_OFFSET: usize = 1040;
const MMAP_DATA_SIZE_OFFSET: usize = 1048;
const RING_DATA_PAGES: usize = 8;
const RECORD_HEADER_SIZE: u64 = 8;
const SAMPLE_IP_SIZE: u64 = 8;
const LOST_RECORD_SIZE: u64 = 16;

/// Raw IP samples and event-loss accounting from one sampling run.
#[derive(Debug, Default)]
pub struct RawSamples {
    pub total_samples: u64,
    pub lost_samples: u64,
    pub overwritten_bytes: u64,
    pub ip_counts: HashMap<u64, u64>,
    pub threads_started: usize,
    pub thread_open_failures: usize,
    pub process_exited: bool,
    pub elapsed_secs: f64,
}

/// The kernel's `perf_event_attr` up to `sig_data` (128 bytes).
///
/// We define it locally instead of relying on `libc`'s bitfield representation
/// so the layout is explicit and identical across libc versions.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    config2: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    sample_max_stack: u16,
    __reserved_2: u16,
    aux_sample_size: u32,
    __reserved_3: u32,
    sig_data: u64,
}

impl PerfEventAttr {
    fn cpu_clock(frequency: u32) -> Self {
        let period_ns = 1_000_000_000u64 / u64::from(frequency.max(1));
        Self {
            type_: PERF_TYPE_SOFTWARE,
            size: size_of::<Self>() as u32,
            config: PERF_COUNT_SW_CPU_CLOCK,
            sample_period: period_ns.max(1),
            sample_type: PERF_SAMPLE_IP,
            read_format: 0,
            flags: ATTR_DISABLED | ATTR_EXCLUDE_KERNEL | ATTR_EXCLUDE_HV,
            wakeup_events: 1,
            bp_type: 0,
            config1: 0,
            config2: 0,
            branch_sample_type: 0,
            sample_regs_user: 0,
            sample_stack_user: 0,
            clockid: 0,
            sample_regs_intr: 0,
            aux_watermark: 0,
            sample_max_stack: 0,
            __reserved_2: 0,
            aux_sample_size: 0,
            __reserved_3: 0,
            sig_data: 0,
        }
    }
}

/// One open perf event and its mmap'd ring buffer.
pub struct PerfEvent {
    fd: libc::c_int,
    base: *mut u8,
    map_len: usize,
    data_ptr: *const u8,
    data_size: usize,
    tail: u64,
}

// The pointer is owned by this struct and only used on the sampling thread.
unsafe impl Send for PerfEvent {}

impl PerfEvent {
    fn open(tid: u32, frequency: u32) -> io::Result<Self> {
        let attr = PerfEventAttr::cpu_clock(frequency);
        let fd = unsafe {
            libc::syscall(
                libc::SYS_perf_event_open,
                &attr as *const PerfEventAttr,
                tid as libc::pid_t,
                -1i32,
                -1i32,
                PERF_FLAG_FD_CLOEXEC,
            )
        } as libc::c_int;

        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = if page_size > 0 {
            page_size as usize
        } else {
            4096
        };
        let map_len = (1 + RING_DATA_PAGES) * page_size;

        let base = unsafe {
            libc::mmap(
                ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            let error = io::Error::last_os_error();
            unsafe {
                libc::close(fd);
            }
            return Err(error);
        }
        let base = base as *mut u8;

        let data_offset = unsafe { read_u64(base, MMAP_DATA_OFFSET_OFFSET) } as usize;
        let data_size = unsafe { read_u64(base, MMAP_DATA_SIZE_OFFSET) } as usize;

        // Old kernels predate data_offset/data_size and always use one metadata
        // page followed by the data pages.
        let data_offset = if data_offset == 0 {
            page_size
        } else {
            data_offset
        };
        let data_size = if data_size == 0 {
            map_len.saturating_sub(data_offset)
        } else {
            data_size
        };

        if data_offset < page_size
            || data_size == 0
            || data_offset.saturating_add(data_size) > map_len
        {
            unsafe {
                libc::munmap(base as *mut libc::c_void, map_len);
                libc::close(fd);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "perf ring buffer metadata is outside the mmap'd region",
            ));
        }

        if unsafe { libc::ioctl(fd, PERF_EVENT_IOC_ENABLE, 0) } < 0 {
            let error = io::Error::last_os_error();
            unsafe {
                libc::munmap(base as *mut libc::c_void, map_len);
                libc::close(fd);
            }
            return Err(error);
        }

        let tail = unsafe { read_u64(base, MMAP_DATA_TAIL_OFFSET) };
        Ok(Self {
            fd,
            base,
            map_len,
            data_ptr: unsafe { base.add(data_offset) },
            data_size,
            tail,
        })
    }

    /// Drain all records currently in the ring buffer.
    fn drain(&mut self, samples: &mut RawSamples) {
        let head = unsafe { read_u64(self.base, MMAP_DATA_HEAD_OFFSET) };
        fence(Ordering::Acquire);

        if head < self.tail {
            // A new event should never move backwards. Treat it as a reset so
            // a bad kernel value cannot make us spin forever.
            self.tail = head;
        }

        // If the consumer fell behind, the kernel has already overwritten the
        // oldest records. Account for the lost bytes and skip to the valid
        // region rather than parsing a torn record.
        let data_size = self.data_size as u64;
        if head.saturating_sub(self.tail) > data_size {
            samples.overwritten_bytes = samples
                .overwritten_bytes
                .saturating_add(head - self.tail - data_size);
            self.tail = head - data_size;
        }

        while self.tail < head {
            let mut header = [0u8; RECORD_HEADER_SIZE as usize];
            if !copy_ring(self.data_ptr, self.data_size, self.tail, &mut header) {
                break;
            }
            let record_type = u32::from_ne_bytes([header[0], header[1], header[2], header[3]]);
            let record_size = u16::from_ne_bytes([header[6], header[7]]) as u64;

            if record_size < RECORD_HEADER_SIZE
                || record_size > data_size
                || self.tail.saturating_add(record_size) > head
            {
                break;
            }

            match record_type {
                PERF_RECORD_SAMPLE => {
                    if record_size >= RECORD_HEADER_SIZE + SAMPLE_IP_SIZE {
                        let mut ip_bytes = [0u8; SAMPLE_IP_SIZE as usize];
                        if copy_ring(
                            self.data_ptr,
                            self.data_size,
                            self.tail + RECORD_HEADER_SIZE,
                            &mut ip_bytes,
                        ) {
                            let ip = u64::from_ne_bytes(ip_bytes);
                            samples.total_samples = samples.total_samples.saturating_add(1);
                            *samples.ip_counts.entry(ip).or_insert(0) += 1;
                        }
                    }
                }
                PERF_RECORD_LOST => {
                    if record_size >= RECORD_HEADER_SIZE + LOST_RECORD_SIZE {
                        let mut lost_bytes = [0u8; 8];
                        if copy_ring(
                            self.data_ptr,
                            self.data_size,
                            self.tail + RECORD_HEADER_SIZE + 8,
                            &mut lost_bytes,
                        ) {
                            let lost = u64::from_ne_bytes(lost_bytes);
                            samples.lost_samples = samples.lost_samples.saturating_add(lost);
                        }
                    }
                }
                _ => {}
            }

            self.tail = self.tail.saturating_add(record_size);
        }

        fence(Ordering::Release);
        unsafe {
            write_u64(self.base, MMAP_DATA_TAIL_OFFSET, self.tail);
        }
    }
}

impl Drop for PerfEvent {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, PERF_EVENT_IOC_DISABLE, 0);
            libc::munmap(self.base as *mut libc::c_void, self.map_len);
            libc::close(self.fd);
        }
    }
}

unsafe fn read_u64(base: *const u8, offset: usize) -> u64 {
    ptr::read_volatile(base.add(offset) as *const u64)
}

unsafe fn write_u64(base: *mut u8, offset: usize, value: u64) {
    ptr::write_volatile(base.add(offset) as *mut u64, value);
}

/// Copy `out.len()` bytes starting at ring position `position`.
///
/// Returns `false` if the request is larger than the ring or would overflow
/// the ring position arithmetic.
fn copy_ring(base: *const u8, ring_size: usize, position: u64, out: &mut [u8]) -> bool {
    if ring_size == 0 || out.len() > ring_size {
        return false;
    }
    let start = (position as usize) % ring_size;
    let first = (ring_size - start).min(out.len());
    unsafe {
        ptr::copy_nonoverlapping(base.add(start), out.as_mut_ptr(), first);
        if first < out.len() {
            ptr::copy_nonoverlapping(base, out.as_mut_ptr().add(first), out.len() - first);
        }
    }
    true
}

/// Read the current thread IDs for a process.
pub fn list_tids(pid: u32) -> io::Result<Vec<u32>> {
    let path = format!("/proc/{pid}/task");
    let mut tids = Vec::new();
    for entry in std::fs::read_dir(&path)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(tid) = name.parse::<u32>() {
                tids.push(tid);
            }
        }
    }
    tids.sort_unstable();
    tids.dedup();
    Ok(tids)
}

/// Sample every thread in `pid` for `duration`, using the given frequency.
///
/// The function opens one software CPU-clock event per thread, rescans the
/// thread list every 500 ms, and aggregates self instruction-pointer samples.
/// It does not require root when `perf_event_paranoid <= 2`.
pub fn sample_process(pid: u32, duration: Duration, frequency: u32) -> Result<RawSamples> {
    if pid == 0 {
        bail!("Refusing to sample PID 0");
    }
    if duration.is_zero() {
        bail!("Sampling duration must be greater than zero");
    }

    let frequency = frequency.clamp(1, 999);
    let mut samples = RawSamples::default();
    let mut events: HashMap<u32, PerfEvent> = HashMap::new();
    let mut open_errors: Vec<String> = Vec::new();

    let initial_tids = list_tids(pid)
        .with_context(|| format!("Failed to list threads for PID {pid} (is the game running?)"))?;
    if initial_tids.is_empty() {
        bail!("PID {pid} has no threads");
    }

    for tid in &initial_tids {
        match PerfEvent::open(*tid, frequency) {
            Ok(event) => {
                events.insert(*tid, event);
                samples.threads_started += 1;
            }
            Err(error) if is_thread_gone(&error) => {}
            Err(error) => {
                samples.thread_open_failures += 1;
                open_errors.push(format!("tid {tid}: {error}"));
            }
        }
    }

    if events.is_empty() {
        let detail = open_errors
            .first()
            .cloned()
            .unwrap_or_else(|| "no threads could be opened".to_string());
        bail!(
            "Could not open a perf event for PID {pid}: {detail}. \
             User-space sampling requires kernel.perf_event_paranoid <= 2."
        );
    }

    let started = Instant::now();
    let mut last_scan = Instant::now();

    while started.elapsed() < duration {
        let tids = match list_tids(pid) {
            Ok(tids) => Some(tids),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                samples.process_exited = true;
                break;
            }
            Err(_) => {
                // A transient /proc read failure should not end an otherwise
                // healthy sampling run; we will retry on the next scan.
                None
            }
        };

        for event in events.values_mut() {
            event.drain(&mut samples);
        }

        if last_scan.elapsed() >= Duration::from_millis(500) {
            if let Some(tids) = tids {
                let tids: std::collections::HashSet<u32> = tids.into_iter().collect();
                events.retain(|tid, _| tids.contains(tid));
                for tid in &tids {
                    if events.contains_key(tid) {
                        continue;
                    }
                    match PerfEvent::open(*tid, frequency) {
                        Ok(event) => {
                            events.insert(*tid, event);
                            samples.threads_started += 1;
                        }
                        Err(error) if is_thread_gone(&error) => {}
                        Err(error) => {
                            samples.thread_open_failures += 1;
                            open_errors.push(format!("tid {tid}: {error}"));
                        }
                    }
                }
            }
            last_scan = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    for event in events.values_mut() {
        event.drain(&mut samples);
    }

    samples.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(samples)
}

fn is_thread_gone(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ESRCH) | Some(libc::ENOENT)
    )
}

/// Check whether this process can open a user-space perf event.
pub fn probe() -> Result<(), String> {
    match PerfEvent::open(0, 99) {
        Ok(event) => {
            drop(event);
            Ok(())
        }
        Err(error) => {
            let paranoid = std::fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")
                .ok()
                .map(|value| value.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Err(format!(
                "perf_event_open failed: {error} (kernel.perf_event_paranoid={paranoid}); \
                 user-space sampling needs perf_event_paranoid <= 2"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn kill_child(child: &mut std::process::Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn copies_records_across_the_ring_wrap() {
        let ring = b"0123456789";
        let mut out = [0u8; 4];
        assert!(copy_ring(ring.as_ptr(), ring.len(), 8, &mut out));
        assert_eq!(&out, b"8901");

        let mut all = [0u8; 10];
        assert!(copy_ring(ring.as_ptr(), ring.len(), 3, &mut all));
        assert_eq!(&all, b"3456789012");

        let mut too_big = [0u8; 11];
        assert!(!copy_ring(ring.as_ptr(), ring.len(), 0, &mut too_big));
    }

    #[test]
    fn samples_a_busy_child_process() {
        // This is the real sampler end-to-end: perf_event_open, ring buffer,
        // per-thread discovery, and aggregation. Skip when the host disallows
        // user-space perf events (common in locked-down CI).
        if probe().is_err() {
            eprintln!("skipping: user-space perf_event_open is not permitted here");
            return;
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg("while :; do :; done")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn busy shell");

        let samples = sample_process(child.id(), Duration::from_millis(700), 99);
        kill_child(&mut child);

        let samples = samples.expect("sample busy child");
        assert!(
            samples.total_samples > 0,
            "expected at least one sample from a busy child"
        );
        assert!(samples.ip_counts.values().sum::<u64>() >= samples.total_samples);
    }

    #[test]
    fn rejects_zero_pid_and_zero_duration() {
        assert!(sample_process(0, Duration::from_millis(1), 99).is_err());
        assert!(sample_process(std::process::id(), Duration::ZERO, 99).is_err());
    }

    #[test]
    fn attr_layout_is_128_bytes() {
        assert_eq!(size_of::<PerfEventAttr>(), 128);
        assert_eq!(PerfEventAttr::cpu_clock(99).size, 128);
    }

    #[test]
    fn tid_list_contains_the_calling_thread() {
        let tids = list_tids(std::process::id()).unwrap();
        let current = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
        assert!(tids.contains(&current));
    }
}
