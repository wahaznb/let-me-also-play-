# Performance metrics that actually matter

This document is the measurement policy for `game-opt-tool`. It answers three
questions:

1. Which numbers describe how a game *feels*?
2. Which ones are misleading on their own?
3. How should this tool collect and compare them so a patch is judged fairly?

The short version: **frame-time distribution and frame pacing come first**.
Average FPS is context, not a target. A patch is only interesting if it improves
the slow tail (1% / 0.1% lows, p99 frame time) without adding stutter or changing
behavior.

## 1. The raw measurement is frame time, not FPS

Every statistic below starts from the interval between two presented frames:

```text
frame_time_ms[i] = present_time[i] - present_time[i - 1]
instant_fps[i]   = 1000 / frame_time_ms[i]
```

FPS is derived. Average FPS over a run is therefore:

```text
average_fps = 1000 / mean(frame_time_ms)
```

It is **not** the arithmetic mean of the per-frame FPS values. Averaging
instantaneous FPS over-weights fast frames and produces an optimistic number.
Frame-time logs are the source of truth; convert to FPS only for display.
CapFrameX makes the same point: the tools ultimately show FPS, but what is
measured is the time between images, and all metrics are calculated from those
raw frame times ([CapFrameX, "Explanation of different performance
metrics"](https://www.capframex.com/blog/post/Explanation%20of%20different%20performance%20metrics)).

This matters for old games because a single 200 ms hitch is one frame with
5 FPS, but it contributes only 200 ms to a 60-second run. It barely moves the
average and dominates what the player remembers.

## 2. Metric definitions and what each one hides

| Metric | Definition | What it captures | Main weakness |
|---|---|---|---|
| Average FPS | `1000 / mean(frame_time_ms)` | Throughput over the whole run | Hides spikes and inconsistent pacing; easy to improve by ignoring the tail |
| Median frame time / median FPS | 50th percentile of frame times, then invert for FPS | Typical frame | Says nothing about the worst frames |
| p95 / p99 frame time | 95th / 99th percentile of the frame-time distribution | Tail boundary: 95% / 99% of frames were faster | A percentile is a boundary, not the severity of frames beyond it |
| p1 / p0.1 FPS | `1000 / p99 / p99.9 frame time` | FPS at the tail boundary | Same boundary limitation; p0.1 needs a lot of frames |
| 1% low FPS (slowest-average) | `1000 / mean(slowest 1% of frame times)` | Average cost of the worst 1% of frames | More outlier-sensitive than p99; definition varies by tool |
| 0.1% low FPS (slowest-average) | `1000 / mean(slowest 0.1% of frame times)` | Rare, severe hitches | Needs thousands of frames; a short run may reduce to one frame |
| Minimum FPS | `1000 / max(frame_time_ms)` | Single worst frame | Not repeatable; the worst frame is often a loading/alt-tab artifact |
| Stutter ratio | `p99 frame time / median frame time` | How far the tail is above the typical frame | Relative, not absolute; a stable 30 FPS run can have a low ratio |
| Slow-frame percentage | Share of frames above a budget (e.g. 2x median or 16.7 ms) | How often the player sees a miss | Budget must be chosen per target frame rate |
| Coefficient of variation (CV) | `stddev(frame_time) / mean(frame_time)` | Overall relative variability | Sensitive to outliers; not a substitute for tail metrics |
| CPU frame time / GPU busy time | Time the CPU spends producing a frame; time the GPU spends executing it | Which side is limiting the frame | Requires API/ETW-level instrumentation; PresentMon exposes these |
| Display / input latency | Click-to-photon or present-to-display delay | Responsiveness, not smoothness | Different axis from frame time; needs Reflex/display instrumentation |

### The two meanings of "1% low"

There are two common calculations, and they are not interchangeable:

- **Percentile low:** take the 99th percentile frame time and invert it. This is
  a boundary: 99% of frames were faster. CapFrameX calls this P1 and notes that
  the correct description depends on whether you are talking about frame times
  or FPS: the 99th percentile of frame times is the 1st percentile of FPS.
- **Slowest-average low:** sort the frame times and average the slowest 1%.
  This weights the very worst frames more heavily. The current `game-opt-tool`
  profiler uses this definition for `low_1pct_fps` and `low_0_1pct_fps`, and
  reports p99 separately.

Both are useful, but they must be labelled. A benchmark that says "1% low" and
quietly changes definitions can make a patch look better or worse without any
real change.

### Percentiles are about frames, not time

A 1% low of 45 FPS does **not** mean the game was above 45 FPS for 99% of the
elapsed time. It means 99% of *frames* were above 45 FPS. If the slowest 1% of
frames are a handful of 500 ms freezes, those frames can account for far more
than 1% of wall-clock time. CapFrameX illustrates exactly this case and offers a
threshold view that shows total time spent below a target
([CapFrameX](https://www.capframex.com/blog/post/Explanation%20of%20different%20performance%20metrics)).
For a player, the time-based view is often closer to the felt experience.

### How many frames are enough?

- **1% low:** at least a few hundred frames; 1,000+ is better.
- **0.1% low:** CapFrameX recommends more than 1,000 frames so that a few frames
  remain after trimming. With exactly 1,000 frames, the 0.1% bucket is one
  frame. For a stable 0.1% result, collect 10,000+ frames (about 3 minutes at
  60 FPS, or 70 seconds at 144 FPS).
- Report the frame count and run duration next to the metric. A 10-second run
  is useful for spotting a regression, not for publishing a 0.1% low.

### Minimum FPS is usually noise

The single worst frame is highly sensitive to loading, shader compilation,
window focus changes, background tasks, and measurement overhead. It is not a
stable comparison metric. Use the tail percentiles and the slowest-average lows
instead.

## 3. What actually makes a game feel smooth

### Frame pacing, not throughput

Two runs can have the same average FPS and feel completely different. A run at
a steady 60 FPS has 16.7 ms frames; a run at 70 FPS average that alternates
8 ms and 25 ms frames has visible judder even though its average is higher. The
perceived quality is in the *distribution and ordering* of frame times, not the
mean. This is why a 1% low or p99 frame time is a better primary signal than
average FPS.

### The tail is usually a CPU or a GPU bottleneck — and you need to know which

- **CPU-bound:** game-thread work (AI, physics, visibility, draw-call
  submission, asset streaming, script logic), driver overhead, scheduling
  contention, or a blocking wait. The frame time spikes when that work exceeds
  the frame budget or when the OS deschedules the thread.
- **GPU-bound:** resolution, shading, shadows, anti-aliasing, VRAM pressure,
  shader compilation, or present/vsync behavior. The CPU may be idle while the
  GPU is late.

CPU sampling answers "which code is executing on the CPU?" It does **not**
answer "which code is waiting?" or "what is the GPU doing?" If the frame-time
spikes are GPU-bound, a CPU hotspot list can be completely irrelevant. Pair CPU
samples with frame-time logs and, on Windows, GPU/CPU frame duration data from
PresentMon
([PresentMon](https://github.com/GameTechDev/PresentMon)).

### Present mode, vsync, VRR, and frame caps

Presentation policy changes the shape of the frame-time distribution before any
game code is optimized:

- Vsync/fifo can quantize frame delivery and add latency when a frame misses the
  refresh deadline.
- VRR (G-Sync/FreeSync) can reduce the visual penalty of variable frame times,
  but it does not make a CPU hitch disappear; it only avoids some tearing and
  judder.
- A frame cap below the achievable maximum often improves consistency by
  preventing the GPU/CPU from racing to an unattainable rate. The cap itself
  must be paced correctly or it becomes a new source of stutter.
- Fullscreen exclusive, borderless, and flip-model presentation have different
  latency and tearing behavior. `DXGI_PRESENT_ALLOW_TEARING` requires a swap
  chain created with `DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING`, a sync interval of 0,
  and windowed/borderless mode; it cannot be used in fullscreen-exclusive mode
  ([Microsoft DXGI_PRESENT](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present)).

For an old DirectDraw/OpenGL game running under Wine, the translation layer is
part of the presentation path. Samples in `winex11.drv`, `wined3d`, or the GL
driver are not "fake" — they are real CPU time spent delivering the frame.

### Input latency is a separate axis

NVIDIA Reflex targets the latency from input sampling to display, not frame rate
or frame-time consistency ([NVIDIA Reflex](https://www.nvidia.com/en-us/geforce/technologies/reflex/)).
A game can have excellent 1% lows and still feel delayed because of a deep render
queue or vsync. Do not use a frame-time win as proof of a responsiveness win, or
vice versa.

### CPU sampling has a blind spot

The sampler in this tool uses Linux `perf_event_open` with user-space CPU-clock
samples. Each sample records the instruction pointer of the thread that is
running. That gives real self-time per function, but:

- blocked time (locks, I/O, waits, vsync) is not sampled;
- kernel time is excluded;
- GPU time is invisible;
- a function can be "hot" without being on the frame-time critical path;
- the function that causes a hitch may never appear because the hitch is a wait
  or an external driver call.

Treat the top functions as **hypotheses to test**, not as an optimization order.
The only proof is a matched frame-time comparison before and after a
behavior-preserving patch.

## 4. Measurement protocol used by this project

### Frame-time capture (`profile`, `compare`)

1. Fix the scene, route, difficulty, settings, resolution, frame cap, vsync/VRR
   state, and background workload.
2. Discard warm-up frames (shader cache, asset loading, first-time initialization).
3. Capture a run long enough for the metric:
   - 60 seconds or 2,000+ frames for 1% lows;
   - 10,000+ frames for a 0.1% low.
4. Repeat at least 3 times. Report the run-to-run spread, not just the best run.
5. Compare matched runs with `compare`; it reports average FPS, 1% low, 0.1%
   low, p99 frame time, and stutter ratio, and warns when the run is too short or
   the durations differ.
6. Accept a patch only if the primary metrics improve or hold within the observed
   noise, the tail does not get worse, and behavior is unchanged.

The current profiler uses MangoHud CSV frame-time logs. Its 1% and 0.1% lows are
slowest-average values, and it also reports p95/p99 frame times. A single-log
comparison is an observed difference, not a statistical significance test; the
tool says so explicitly.

### CPU sampling (`sample`)

1. Start the game and reach the scene to be measured.
2. Run `game-opt-tool sample --game <exe> --duration 10..30 --top 20`.
3. Let Ghidra resolve addresses to function names; without Ghidra the tool
   reports raw PE RVAs and Ghidra-space VAs.
4. Use the sample to form one hypothesis at a time. Verify the disassembly and
   the patch semantics before writing bytes.
5. Patch a copy, capture matched frame-time runs, and compare. Revert if the
   tail regresses.

Sampling frequency is a trade-off: 199 Hz is enough to rank multi-millisecond
functions over 10–30 seconds, while 999 Hz gives finer resolution at a higher
overhead. The sampler reports lost samples and distinguishes game, external
(Wine/driver), and unknown-in-game samples so an all-external profile is not
mistaken for a game hotspot list.

## 5. What this tool does *not* claim

- It does not claim that a CPU hotspot is the cause of a frame-time spike.
- It does not claim that a patch is faster until two matched frame-time runs show
  it.
- It does not currently measure GPU busy time, display latency, or input
  latency. Use PresentMon, MangoHud, or Reflex tooling for those axes.
- It has no measured speedup for a real game yet. The `optimize` command is a
  stub; real changes go through an explicit patch spec, backup, patch, and
  matched comparison.

## 6. Recommended primary metrics

For this project, in order:

1. **1% low FPS** (slowest-average, with the definition stated) and/or **p99
   frame time** as the primary tail metric.
2. **0.1% low FPS** when the run has enough frames; otherwise label it
   provisional.
3. **Stutter ratio** (`p99 / median`) and **slow-frame percentage** for pacing.
4. **Median frame time / average FPS** as context only.
5. **CPU vs GPU attribution** from frame-time tooling before choosing a CPU
   optimization.

A patch that raises average FPS while lowering the 1% low is not a win. A patch
that leaves the average unchanged but removes a 100 ms recurring hitch is.

## Sources

- CapFrameX, *Explanation of different performance metrics*:
  <https://www.capframex.com/blog/post/Explanation%20of%20different%20performance%20metrics>
- CapFrameX, *The challenge of displaying performance metrics as FPS*:
  <https://www.capframex.com/blog/post/The%20challenge%20of%20displaying%20performance%20metrics%20as%20FPS>
- Intel, *PresentMon*:
  <https://github.com/GameTechDev/PresentMon>
- Microsoft, *DXGI_PRESENT*:
  <https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-present>
- NVIDIA, *Reflex*:
  <https://www.nvidia.com/en-us/geforce/technologies/reflex/>
- GamersNexus, *Testing Methodology Explanations: 1% & 0.1% Lows*:
  <https://gamersnexus.net/site-news/2513-testing-methodology-explained-1percent-lows-and-delta-t>
- MangoHud:
  <https://github.com/flightlessmango/MangoHud>
