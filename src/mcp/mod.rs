//! MCP server — exposes game-opt-tool to AI agents (OpenCode, Claude Code, etc.)
//!
//! Implements JSON-RPC 2.0 over stdio (newline-delimited) and an HTTP POST
//! transport. Every tool call maps to the same code paths as the CLI:
//!
//! * `decompile`   → Ghidra headless via [`crate::decompiler`]
//! * `game_info`   → binary/game detection + entry point
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
                    "instructions": "Reverse-engineering and binary-patching tools for owned, single-player games. Start with game_info, then decompile, profile, and patch."
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
            "Decompile a function with Ghidra headless and return C pseudocode plus assembly. Defaults to the executable entry point.",
            json!({
                "type": "object",
                "properties": {
                    "game": { "type": "string", "description": "Path to the game executable." },
                    "address": { "type": "string", "description": "Function address in hex (e.g. 0x109214b6). Optional." }
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
        Ok(value) => Ok((serde_json::to_string_pretty(&value)?, false)),
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

    Ok(json!(function))
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
}
