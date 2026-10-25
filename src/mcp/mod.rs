//! MCP server — exposes game-opt-tool to AI agents (OpenCode, Claude Code, etc.)
//!
//! Implements JSON-RPC 2.0 over stdio (newline-delimited) and an HTTP POST
//! transport. Every tool call maps to the same code paths as the CLI:
//!
//! * `decompile`   → Ghidra headless via [`crate::decompiler`]
//! * `game_info`   → binary/game detection + entry point
//! * `sample`      → real per-function CPU samples from a running game
//! * `profile`     → MangoHud CSV frame-time stats
//! * `compare`     → baseline vs patched frame-time comparison
//! * `patch`       → validated, manifest-backed PE patch
//! * `unpatch`     → exact revert from a patch manifest
//! * `backup`      → create a SHA-256 verified backup
//! * `backup_verify` → re-check every existing backup
//! * `verify`      → compare original and patched binaries
//!
//! stdout is reserved for protocol messages. All logging goes to stderr.

use crate::backup;
use crate::decompiler::{self, Decompiler};
use crate::game::GameInfo;
use crate::patcher::{PatchSpec, Patcher};
use crate::profiler::Profiler;
use crate::verifier::Verifier;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "game-opt-tool";

/// Upper bound for any single MCP tool text, so one call cannot flood an
/// agent's context window. Override with `GAME_OPT_MCP_MAX_BYTES`.
const DEFAULT_MAX_RESPONSE_BYTES: usize = 60_000;
const MIN_MAX_RESPONSE_BYTES: usize = 4_096;
const MAX_MAX_RESPONSE_BYTES: usize = 1_048_576;
/// Default pseudocode budget for one `decompile` page (~6k tokens).
const DEFAULT_DECOMPILE_BYTES: usize = 24_000;
/// Preview kept when an unexpected oversized response is capped.
const MAX_PREVIEW_BYTES: usize = 8_192;

/// Configured response budget per tool call.
fn max_response_bytes() -> usize {
    std::env::var("GAME_OPT_MCP_MAX_BYTES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map(|value| value.clamp(MIN_MAX_RESPONSE_BYTES, MAX_MAX_RESPONSE_BYTES))
        .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
}

/// Slice `text` at char boundaries. Returns `(slice, next_offset, truncated)`.
fn slice_text(text: &str, offset: usize, max_bytes: usize) -> (String, Option<usize>, bool) {
    let total = text.len();
    let mut start = offset.min(total);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = start.saturating_add(max_bytes).min(total);
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < total;
    (
        text[start..end].to_string(),
        if truncated { Some(end) } else { None },
        truncated,
    )
}

/// Safety net for unexpectedly large JSON payloads. Keeps a bounded preview so
/// file paths and manifest handles near the start remain visible.
fn enforce_response_cap_with_limit(text: String, tool: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text;
    }

    tracing::warn!(
        "MCP response for '{}' was {} bytes (cap {}); truncating to protect agent context",
        tool,
        text.len(),
        cap
    );

    let preview_budget = (cap / 2).min(MAX_PREVIEW_BYTES);
    let (preview, _, _) = slice_text(&text, 0, preview_budget);
    json!({
        "truncated": true,
        "tool": tool,
        "total_bytes": text.len(),
        "max_bytes": cap,
        "preview": preview,
        "note": "This response exceeded the MCP context budget. Use a narrower request: one function address, a smaller pagination window, or tail frames."
    })
    .to_string()
}

fn enforce_response_cap(text: String, tool: &str) -> String {
    enforce_response_cap_with_limit(text, tool, max_response_bytes())
}

/// Run the MCP server over stdio (the transport OpenCode uses for local servers).
pub async fn run_stdio() -> Result<()> {
    info!("MCP server starting on stdio");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = stdin.lock();

    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .context("Failed to read MCP request from stdin")?;
        if read == 0 {
            info!("MCP client closed stdin; shutting down");
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                let response = jsonrpc_error(Value::Null, -32700, &format!("Parse error: {error}"));
                write_message(&mut stdout, &response)?;
                continue;
            }
        };

        let Some(response) = handle_message(&request).await else {
            continue; // notification: no response
        };
        write_message(&mut stdout, &response)?;
    }

    Ok(())
}

/// Run the MCP server over HTTP (`POST /mcp`).
pub async fn run_http(host: &str, port: u16) -> Result<()> {
    use axum::{routing::post, Router};

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind MCP HTTP server to {addr}"))?;
    let local = listener
        .local_addr()
        .context("Failed to read MCP HTTP server address")?;

    let state = Arc::new(Mutex::new(()));
    let router = Router::new()
        .route("/mcp", post(http_handler))
        .with_state(state);

    info!("MCP HTTP server listening on http://{local}/mcp");
    axum::serve(listener, router)
        .await
        .context("MCP HTTP server failed")?;
    Ok(())
}

async fn http_handler(
    axum::extract::State(_state): axum::extract::State<Arc<Mutex<()>>>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let request: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(jsonrpc_error(
                    Value::Null,
                    -32700,
                    &format!("Parse error: {error}"),
                )),
            )
                .into_response();
        }
    };

    match handle_message(&request).await {
        Some(response) => (StatusCode::OK, axum::Json(response)).into_response(),
        // Notifications are acknowledged with 202 and no body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn write_message(stdout: &mut impl Write, message: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, message).context("Failed to serialize MCP response")?;
    stdout
        .write_all(b"\n")
        .context("Failed to write MCP response")?;
    stdout.flush().context("Failed to flush MCP response")?;
    Ok(())
}

/// Dispatch one JSON-RPC message. Returns `None` for notifications.
async fn handle_message(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");

    // Notifications never get a response, regardless of method.
    if id.is_none() && !method.is_empty() {
        return None;
    }

    let Some(id) = id else {
        return Some(jsonrpc_error(
            Value::Null,
            -32600,
            "Invalid request: missing id",
        ));
    };

    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let response = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            jsonrpc_result(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": "Reverse-engineering and binary-patching tools for owned, single-player games. Start with game_info, use sample for real per-function CPU hotspots while the game runs, decompile one function at a time, then patch and compare."
                }),
            )
        }
        "ping" => jsonrpc_result(id, json!({})),
        "tools/list" => jsonrpc_result(id, json!({ "tools": tool_definitions() })),
        "tools/call" => match call_tool(&params).await {
            Ok((text, is_error)) => jsonrpc_result(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                }),
            ),
            Err(error) => jsonrpc_result(
                id,
                json!({
                    "content": [{ "type": "text", "text": format!("{error:#}") }],
                    "isError": true,
                }),
            ),
        },
        other => jsonrpc_error(id, -32601, &format!("Method not found: {other}")),
    };

    Some(response)
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn tool_definitions() -> Value {
    fn tool(name: &str, description: &str, schema: Value) -> Value {
        json!({
            "name": name,
            "description": description,
            "inputSchema": schema,
        })
    }

    json!([
        tool(
            "game_info",
            "Detect a game executable, its format, architecture, and PE entry point.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Path to the game executable or install directory." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "decompile",
            "Decompile one function with Ghidra and return C pseudocode. Results are paginated: pass next_offset to continue. Assembly is omitted unless include_assembly is true.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Path to the game executable." },
                    "address": { "type": "string", "description": "Function address in hex (e.g. 0x109214b6). Optional; defaults to the entry point." },
                    "offset": { "type": "integer", "minimum": 0, "description": "Byte offset into the pseudocode. Use next_offset from a previous call." },
                    "max_bytes": { "type": "integer", "minimum": 1024, "description": "Maximum pseudocode bytes to return (default 24000)." },
                    "include_assembly": { "type": "boolean", "description": "Also return disassembly (counts against the same budget). Default false." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "list_functions",
            "List function handles (address, name, size) without decompiling them. Use this to choose one function to decompile next.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Path to the game executable." },
                    "filter": { "type": "string", "description": "Case-insensitive substring match on the function name." },
                    "offset": { "type": "integer", "minimum": 0, "description": "Skip this many matching functions." },
                    "limit": { "type": "integer", "minimum": 1, "description": "Maximum functions to return (default 200, max 1000)." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "sample",
            "Sample a running game's CPU time per function with Linux perf_event_open, without sudo. Launch the game first; this finds the process that maps `game` unless pid/process is given. Resolves function names with cached Ghidra analysis. Returns self CPU samples, not GPU time or call-graph attribution.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Path to the game executable. Also used to find the running process and resolve function names." },
                    "pid": { "type": "integer", "minimum": 1, "description": "Running game PID. Optional; omit to find the process mapping the game." },
                    "process": { "type": "string", "description": "Process name substring (e.g. DeusEx.exe) instead of pid." },
                    "duration_secs": { "type": "integer", "minimum": 1, "maximum": 120, "description": "Sampling window in seconds (default 10)." },
                    "frequency": { "type": "integer", "minimum": 1, "maximum": 999, "description": "Samples per second per thread (default 99)." },
                    "top": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Functions to return (default 20)." },
                    "refresh_functions": { "type": "boolean", "description": "Rebuild the cached Ghidra function list." },
                    "resolve_functions": { "type": "boolean", "description": "Resolve names with Ghidra (default true). False returns raw PE RVAs and skips Ghidra." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "profile",
            "Compute frame-time statistics (1% low, 0.1% low, p99, stutter ratio) from a MangoHud CSV log.",
            json!({
                "type": "object",
                "properties": {
                    "log": { "type": "string", "description": "Path to the MangoHud CSV log." },
                    "frames": { "type": "integer", "description": "Analyze only the last N frames (0 = all).", "minimum": 0 }
                },
                "required": ["log"]
            })
        ),
        tool(
            "compare",
            "Compare frame-time metrics between a baseline and a patched MangoHud log.",
            json!({
                "type": "object",
                "properties": {
                    "baseline": { "type": "string" },
                    "patched": { "type": "string" },
                    "frames": { "type": "integer", "description": "Compare only the last N frames (0 = all).", "minimum": 0 }
                },
                "required": ["baseline", "patched"]
            })
        ),
        tool(
            "patch",
            "Apply a validated, expected-byte-verified JSON patch spec to a PE binary and write a reversible manifest.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Input executable." },
                    "output": { "type": "string", "description": "Output executable path." },
                    "spec": { "type": "string", "description": "Path to the JSON patch spec." },
                    "manifest": { "type": "string", "description": "Optional manifest output path." },
                    "dry_run": { "type": "boolean", "description": "Validate without writing." }
                },
                "required": ["game", "output", "spec"]
            })
        ),
        tool(
            "unpatch",
            "Revert exactly the changes recorded in a patch manifest.",
            json!({
                "type": "object",
                "properties": {
                    "manifest": { "type": "string", "description": "Patch manifest path." },
                    "game": { "type": "string", "description": "Patched executable override." },
                    "dry_run": { "type": "boolean", "description": "Validate without writing." }
                },
                "required": ["manifest"]
            })
        ),
        tool(
            "backup",
            "Create a SHA-256 verified backup of a binary next to it.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "File to back up." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "backup_verify",
            "Re-check the checksum of every existing backup for a binary.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "File whose backups should be verified." }
                },
                "required": ["game"]
            })
        ),
        tool(
            "verify",
            "Compare an original and a patched binary. Reports a hash/section diff; behavioral verification is not implemented.",
            json!({
                "type": "object",
                "properties": {
                    "original": { "type": "string" },
                    "patched": { "type": "string" }
                },
                "required": ["original", "patched"]
            })
        )
    ])
}

/// Execute one tool call. Returns `(text, is_error)`.
async fn call_tool(params: &Value) -> Result<(String, bool)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .context("Missing tool name")?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let result = match name {
        "game_info" => tool_game_info(&args),
        "decompile" => tool_decompile(&args).await,
        "list_functions" => tool_list_functions(&args).await,
        "sample" => tool_sample(&args).await,
        "profile" => tool_profile(&args),
        "compare" => tool_compare(&args),
        "patch" => tool_patch(&args),
        "unpatch" => tool_unpatch(&args),
        "backup" => tool_backup(&args),
        "backup_verify" => tool_backup_verify(&args),
        "verify" => tool_verify(&args).await,
        other => Err(anyhow::anyhow!("Unknown tool: {other}")),
    };

    match result {
        Ok(value) => {
            // Compact JSON keeps tool output small; pretty-printing wastes
            // context on whitespace in the agent loop.
            let text = serde_json::to_string(&value)?;
            Ok((enforce_response_cap(text, name), false))
        }
        Err(error) => Ok((format!("{error:#}"), true)),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("Missing required argument: {key}"))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn opt_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn opt_frames(args: &Value) -> usize {
    args.get("frames")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

fn tool_game_info(args: &Value) -> Result<Value> {
    let game_path = PathBuf::from(arg_str(args, "game")?);
    let game = GameInfo::from_path(&game_path)?;
    let entry = decompiler::entry_point(&game).ok();

    Ok(json!({
        "name": game.name,
        "version": game.version,
        "path": game.path,
        "format": game.binary_format,
        "architecture": game.architecture,
        "entry_point": entry.map(|address| format!("0x{address:X}")),
    }))
}

async fn tool_decompile(args: &Value) -> Result<Value> {
    let game_path = PathBuf::from(arg_str(args, "game")?);
    if !game_path.exists() {
        anyhow::bail!("Game path does not exist: {}", game_path.display());
    }
    let address = match opt_str(args, "address") {
        Some(value) => Some(decompiler::parse_hex_address(&value)?),
        None => None,
    };

    let game = GameInfo::from_path(&game_path)?;
    let decompiler = Decompiler::new()?;
    let function = match address {
        Some(address) => decompiler.decompile_function(&game, address).await?,
        None => decompiler.decompile_entry(&game).await?,
    };

    let include_assembly = opt_bool(args, "include_assembly");
    let cap = max_response_bytes();
    let requested_bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let budget = requested_bytes
        .unwrap_or(DEFAULT_DECOMPILE_BYTES)
        .clamp(1_024, cap);
    let pseudocode_budget = if include_assembly { budget / 2 } else { budget };

    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let pseudocode_total = function.pseudocode.len();
    let (pseudocode, next_offset, truncated) =
        slice_text(&function.pseudocode, offset, pseudocode_budget);

    let mut result = json!({
        "name": function.name,
        "address": function.address,
        "signature": function.signature,
        "return_type": function.return_type,
        "parameters": function.parameters,
        "pseudocode": pseudocode,
        "pseudocode_total_bytes": pseudocode_total,
        "offset": offset,
        "next_offset": next_offset,
        "truncated": truncated,
        "max_bytes": budget,
    });

    if include_assembly {
        let assembly_total = function.assembly.len();
        let (assembly, _, assembly_truncated) =
            slice_text(&function.assembly, 0, budget - pseudocode_budget);
        result["assembly"] = json!(assembly);
        result["assembly_total_bytes"] = json!(assembly_total);
        result["assembly_truncated"] = json!(assembly_truncated);
    }

    Ok(result)
}

async fn tool_list_functions(args: &Value) -> Result<Value> {
    use crate::decompiler::FunctionSummary;

    let game_path = PathBuf::from(arg_str(args, "game")?);
    if !game_path.exists() {
        anyhow::bail!("Game path does not exist: {}", game_path.display());
    }

    let filter = opt_str(args, "filter").map(|value| value.to_lowercase());
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    // Keep the page safely inside the response budget; ~80 bytes per handle
    // leaves room for long function names and JSON overhead.
    let rows_by_budget = (max_response_bytes().saturating_sub(512) / 80).max(1);
    let requested_limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 1_000) as usize;
    let limit = requested_limit.min(rows_by_budget);

    let game = GameInfo::from_path(&game_path)?;
    let decompiler = Decompiler::new()?;
    let mut functions = decompiler.list_functions(&game).await?;
    functions.sort_by_key(|function| function.address);

    if let Some(filter) = filter.as_deref() {
        functions.retain(|function| function.name.to_lowercase().contains(filter));
    }

    let total = functions.len();
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    let returned: Vec<FunctionSummary> = functions[start..end].to_vec();

    Ok(json!({
        "total_functions": total,
        "returned": returned.len(),
        "offset": start,
        "limit": limit,
        "next_offset": if end < total { Some(end) } else { None },
        "truncated": end < total,
        "functions": returned.iter().map(|function| json!({
            "address": format!("0x{:X}", function.address),
            "name": function.name,
            "size": function.size,
        })).collect::<Vec<_>>(),
    }))
}

async fn tool_sample(args: &Value) -> Result<Value> {
    use crate::sampler::{run, SampleConfig};

    let game = PathBuf::from(arg_str(args, "game")?);
    if !game.is_file() {
        anyhow::bail!("Game executable not found: {}", game.display());
    }

    let pid = match args.get("pid").and_then(Value::as_u64) {
        Some(value) => Some(
            u32::try_from(value)
                .map_err(|_| anyhow::anyhow!("pid is outside the valid range"))?,
        ),
        None => None,
    };
    let process = opt_str(args, "process");
    if pid.is_some() && process.is_some() {
        anyhow::bail!("Pass either pid or process, not both");
    }

    let duration_secs = args
        .get("duration_secs")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .clamp(1, 120);
    let frequency = args
        .get("frequency")
        .and_then(Value::as_u64)
        .unwrap_or(99)
        .clamp(1, 999) as u32;
    let top = args
        .get("top")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let resolve_functions = args
        .get("resolve_functions")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let report = run(SampleConfig {
        game,
        pid,
        process,
        duration: std::time::Duration::from_secs(duration_secs),
        frequency,
        top,
        refresh_functions: opt_bool(args, "refresh_functions"),
        resolve_functions,
    })
    .await?;

    Ok(serde_json::to_value(report)?)
}

fn tool_profile(args: &Value) -> Result<Value> {
    let log = PathBuf::from(arg_str(args, "log")?);
    if !log.is_file() {
        anyhow::bail!("Log file not found: {}", log.display());
    }
    let profiler = Profiler::new();
    let results = profiler.profile_csv(&log, opt_frames(args))?;
    Ok(serde_json::to_value(results)?)
}

fn tool_compare(args: &Value) -> Result<Value> {
    let baseline = PathBuf::from(arg_str(args, "baseline")?);
    let patched = PathBuf::from(arg_str(args, "patched")?);
    for path in [&baseline, &patched] {
        if !path.is_file() {
            anyhow::bail!("Log file not found: {}", path.display());
        }
    }
    let profiler = Profiler::new();
    let comparison = profiler.compare_logs(&baseline, &patched, opt_frames(args))?;
    Ok(serde_json::to_value(comparison)?)
}

fn tool_patch(args: &Value) -> Result<Value> {
    let game = PathBuf::from(arg_str(args, "game")?);
    let output = PathBuf::from(arg_str(args, "output")?);
    let spec_path = PathBuf::from(arg_str(args, "spec")?);
    let manifest = opt_str(args, "manifest").map(PathBuf::from);
    let dry_run = opt_bool(args, "dry_run");

    if !game.exists() {
        anyhow::bail!("Game path does not exist: {}", game.display());
    }
    if !spec_path.is_file() {
        anyhow::bail!("Patch spec not found: {}", spec_path.display());
    }
    let spec = PatchSpec::load(&spec_path)?;
    let patcher = Patcher::new();
    let report = patcher.apply_spec(&game, &output, &spec, manifest.as_deref(), dry_run)?;
    Ok(serde_json::to_value(report)?)
}

fn tool_unpatch(args: &Value) -> Result<Value> {
    let manifest = PathBuf::from(arg_str(args, "manifest")?);
    let game = opt_str(args, "game").map(PathBuf::from);
    let dry_run = opt_bool(args, "dry_run");
    if !manifest.is_file() {
        anyhow::bail!("Manifest not found: {}", manifest.display());
    }
    let report = Patcher::unpatch(&manifest, game.as_deref(), dry_run)?;
    Ok(serde_json::to_value(report)?)
}

fn tool_backup(args: &Value) -> Result<Value> {
    let path = PathBuf::from(arg_str(args, "game")?);
    let record = backup::create(&path)?;
    Ok(serde_json::to_value(record)?)
}

fn tool_backup_verify(args: &Value) -> Result<Value> {
    let path = PathBuf::from(arg_str(args, "game")?);
    let records = backup::list_backups(&path)?;
    let mut verified = 0usize;
    let mut failures = Vec::new();
    for record in &records {
        match backup::verify_record(record) {
            Ok(()) => verified += 1,
            Err(error) => failures.push(format!("{}: {error:#}", record.manifest_path().display())),
        }
    }
    Ok(json!({
        "backups": records.len(),
        "verified": verified,
        "failures": failures,
    }))
}

async fn tool_verify(args: &Value) -> Result<Value> {
    let original = PathBuf::from(arg_str(args, "original")?);
    let patched = PathBuf::from(arg_str(args, "patched")?);
    for path in [&original, &patched] {
        if !path.is_file() {
            anyhow::bail!("File not found: {}", path.display());
        }
    }
    let verifier = Verifier::new();
    let report = verifier.verify(&original, &patched).await?;
    Ok(serde_json::to_value(report)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_with_tool_capability() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        });
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_message(&request))
            .expect("initialize must respond");
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
        assert_eq!(response["result"]["capabilities"]["tools"], json!({}));
    }

    #[test]
    fn lists_all_tools() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_message(&request))
            .expect("tools/list must respond");
        let tools = response["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        for expected in [
            "game_info",
            "decompile",
            "list_functions",
            "sample",
            "profile",
            "compare",
            "patch",
            "unpatch",
            "backup",
            "backup_verify",
            "verify",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
    }

    #[test]
    fn notifications_get_no_response() {
        let request = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_message(&request));
        assert!(response.is_none());
    }

    #[test]
    fn unknown_tool_is_flagged_as_error() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "nope", "arguments": {} }
        });
        let response = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(handle_message(&request))
            .expect("tools/call must respond");
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn slices_text_on_char_boundaries_with_a_continuation_offset() {
        let text = "abcdefghij";
        let (slice, next, truncated) = slice_text(text, 0, 4);
        assert_eq!(slice, "abcd");
        assert_eq!(next, Some(4));
        assert!(truncated);

        let (slice, next, truncated) = slice_text(text, 4, 100);
        assert_eq!(slice, "efghij");
        assert_eq!(next, None);
        assert!(!truncated);

        // Multi-byte characters must never be split.
        let unicode = "aé€z";
        let (slice, next, _) = slice_text(unicode, 0, 3);
        assert!(unicode.is_char_boundary(next.unwrap_or(unicode.len())));
        assert!(unicode.starts_with(&slice));
    }

    #[test]
    fn caps_an_oversized_response_and_keeps_a_bounded_preview() {
        let text = "x".repeat(50_000);
        let capped = enforce_response_cap_with_limit(text, "demo", 10_000);
        let value: Value = serde_json::from_str(&capped).unwrap();
        assert_eq!(value["truncated"], true);
        assert_eq!(value["tool"], "demo");
        assert_eq!(value["total_bytes"], 50_000);
        assert!(value["preview"].as_str().unwrap().len() <= 5_000);
    }

    #[test]
    fn leaves_a_small_response_unchanged() {
        let text = "{\"ok\":true}".to_string();
        let capped = enforce_response_cap_with_limit(text.clone(), "demo", 10_000);
        assert_eq!(capped, text);
    }
}
