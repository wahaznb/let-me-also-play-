//! End-to-end tests for the MCP server (`mcp-server`).
//!
//! These drive the real compiled binary over both transports, exactly like an
//! AI agent would: JSON-RPC over stdio and JSON-RPC over HTTP.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_game-opt-tool");
const ENTRY_VA: u64 = 0x0040_1000;

/// Run one stdio session: write every request, close stdin, collect responses
/// keyed by numeric id. Notifications produce no response and are skipped.
fn stdio_session(requests: &[Value]) -> HashMap<i64, Value> {
    stdio_session_with_env(requests, &[])
}

fn stdio_session_with_env(requests: &[Value], env: &[(&str, &str)]) -> HashMap<i64, Value> {
    let mut command = Command::new(BIN);
    command.arg("mcp-server");
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start mcp-server");

    {
        let mut stdin = child.stdin.take().expect("stdin is piped");
        for request in requests {
            writeln!(stdin, "{}", serde_json::to_string(request).unwrap()).unwrap();
        }
        stdin.flush().unwrap();
        // stdin is dropped here, which closes the pipe and ends the server loop.
    }

    let output = child.wait_with_output().expect("mcp-server did not exit");
    assert!(
        output.status.success(),
        "mcp-server failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut responses = HashMap::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value =
            serde_json::from_str(line).unwrap_or_else(|error| panic!("bad JSON line: {error}: {line}"));
        if let Some(id) = value["id"].as_i64() {
            responses.insert(id, value);
        }
    }
    responses
}

fn tool_text(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing tool text: {response}"))
        .to_string()
}

fn tool_json(response: &Value) -> Value {
    let text = tool_text(response);
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("tool text is not JSON: {error}: {text}"))
}

fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap_or(false)
}

/// Minimal PE32 image: .text at RVA 0x1000, raw 0x400, entry bytes 55 8B EC.
fn pe32() -> Vec<u8> {
    const FILE_SIZE: usize = 0x600;
    let mut data = vec![0u8; FILE_SIZE];
    data[0] = b'M';
    data[1] = b'Z';
    data[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    data[0x80..0x84].copy_from_slice(b"PE\0\0");

    let coff = 0x84;
    data[coff..coff + 2].copy_from_slice(&0x014Cu16.to_le_bytes());
    data[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    data[coff + 16..coff + 18].copy_from_slice(&0xE0u16.to_le_bytes());
    data[coff + 18..coff + 20].copy_from_slice(&0x0102u16.to_le_bytes());

    let optional = 0x98;
    data[optional..optional + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
    data[optional + 4..optional + 8].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 16..optional + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 20..optional + 24].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 28..optional + 32].copy_from_slice(&0x0040_0000u32.to_le_bytes());
    data[optional + 32..optional + 36].copy_from_slice(&0x1000u32.to_le_bytes());
    data[optional + 36..optional + 40].copy_from_slice(&0x200u32.to_le_bytes());
    data[optional + 56..optional + 60].copy_from_slice(&0x2000u32.to_le_bytes());
    data[optional + 60..optional + 64].copy_from_slice(&0x400u32.to_le_bytes());
    data[optional + 68..optional + 70].copy_from_slice(&3u16.to_le_bytes());
    data[optional + 92..optional + 96].copy_from_slice(&16u32.to_le_bytes());

    let section = 0x178;
    data[section..section + 5].copy_from_slice(b".text");
    data[section + 8..section + 12].copy_from_slice(&0x100u32.to_le_bytes());
    data[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    data[section + 16..section + 20].copy_from_slice(&0x200u32.to_le_bytes());
    data[section + 20..section + 24].copy_from_slice(&0x400u32.to_le_bytes());
    data[section + 36..section + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

    data[0x400..0x408].copy_from_slice(&[0x55, 0x8B, 0xEC, 0x83, 0xEC, 0x10, 0x90, 0xC3]);
    data
}

fn write_spec(dir: &Path, name: &str, value: Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    path
}

#[test]
fn stdio_initializes_and_lists_tools() {
    let responses = stdio_session(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }),
    ]);

    let init = &responses[&1]["result"];
    assert_eq!(init["serverInfo"]["name"], "game-opt-tool");
    assert!(init["capabilities"]["tools"].is_object());

    let tools = responses[&2]["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|tool| tool["name"].as_str()).collect();
    for expected in [
        "game_info",
        "decompile",
        "list_functions",
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
    assert_eq!(responses[&3]["result"], json!({}));
    assert!(
        !responses.values().any(|response| response["id"] == json!(null)),
        "notifications must not produce responses"
    );
}

#[test]
fn stdio_patches_then_unpatches_exact_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    let original = pe32();
    std::fs::write(&input, &original).unwrap();

    let spec = write_spec(
        dir.path(),
        "patch.json",
        json!({
            "schema_version": 1,
            "name": "mcp-roundtrip",
            "patches": [{
                "address": format!("0x{ENTRY_VA:X}"),
                "expected": "55",
                "replace": "90",
                "note": "NOP the first entry byte"
            }]
        }),
    );

    let patch_responses = stdio_session(&[json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "patch",
            "arguments": {
                "game": input.to_str().unwrap(),
                "output": output.to_str().unwrap(),
                "spec": spec.to_str().unwrap()
            }
        }
    })]);

    assert!(
        !is_error(&patch_responses[&1]),
        "patch failed: {}",
        tool_text(&patch_responses[&1])
    );
    let patch_report = tool_json(&patch_responses[&1]);
    assert_eq!(patch_report["patches_applied"], 1);
    assert_eq!(std::fs::read(&output).unwrap()[0x400], 0x90);
    let manifest = patch_report["manifest_path"].as_str().unwrap().to_string();
    assert!(Path::new(&manifest).is_file());

    let unpatch_responses = stdio_session(&[json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "unpatch",
            "arguments": {
                "manifest": manifest,
                "game": output.to_str().unwrap()
            }
        }
    })]);

    assert!(
        !is_error(&unpatch_responses[&1]),
        "unpatch failed: {}",
        tool_text(&unpatch_responses[&1])
    );
    let unpatch_report = tool_json(&unpatch_responses[&1]);
    assert_eq!(unpatch_report["patches_reverted"], 1);
    assert_eq!(std::fs::read(&output).unwrap(), original);
}

#[test]
fn stdio_backs_up_and_verifies_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.bin");
    std::fs::write(&game, b"owned single-player binary").unwrap();

    let responses = stdio_session(&[
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "backup", "arguments": { "game": game.to_str().unwrap() } }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "backup_verify", "arguments": { "game": game.to_str().unwrap() } }
        }),
    ]);

    assert!(!is_error(&responses[&1]), "backup failed: {}", tool_text(&responses[&1]));
    let record = tool_json(&responses[&1]);
    assert!(record["sha256"].as_str().unwrap().len() == 64);

    assert!(
        !is_error(&responses[&2]),
        "verify failed: {}",
        tool_text(&responses[&2])
    );
    let verify = tool_json(&responses[&2]);
    assert_eq!(verify["backups"], 1);
    assert_eq!(verify["verified"], 1);
    assert_eq!(verify["failures"].as_array().unwrap().len(), 0);
}

#[test]
fn stdio_caps_oversized_tool_output_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("game.exe");
    let output = dir.path().join("patched.exe");
    let original = pe32();
    std::fs::write(&input, &original).unwrap();

    // Enough distinct byte patches to make the JSON report exceed the cap.
    let mut patches = Vec::new();
    for index in 0..64u64 {
        let file_offset = 0x400 + index as usize;
        let before = original[file_offset];
        patches.push(json!({
            "address": format!("0x{:X}", ENTRY_VA + index),
            "expected": format!("{before:02X}"),
            "replace": format!("{:02X}", before ^ 0xFF),
        }));
    }
    let spec = write_spec(
        dir.path(),
        "big-patch.json",
        json!({ "schema_version": 1, "patches": patches }),
    );

    let responses = stdio_session_with_env(
        &[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "patch",
                "arguments": {
                    "game": input.to_str().unwrap(),
                    "output": output.to_str().unwrap(),
                    "spec": spec.to_str().unwrap()
                }
            }
        })],
        &[("GAME_OPT_MCP_MAX_BYTES", "4096")],
    );

    assert!(
        !is_error(&responses[&1]),
        "patch failed: {}",
        tool_text(&responses[&1])
    );
    let text = tool_text(&responses[&1]);
    assert!(
        text.len() <= 4_096,
        "capped response was {} bytes",
        text.len()
    );
    let capped: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["tool"], "patch");
    assert!(capped["total_bytes"].as_u64().unwrap() > 4_096);
    assert!(
        capped["preview"].as_str().unwrap().contains("manifest_path"),
        "preview should keep the manifest handle: {}",
        capped["preview"]
    );
}

#[test]
fn stdio_reports_unknown_tools_as_tool_errors() {
    let responses = stdio_session(&[json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "nope", "arguments": {} }
    })]);

    assert!(is_error(&responses[&1]));
    assert!(tool_text(&responses[&1]).contains("Unknown tool: nope"));
}

#[test]
fn stdio_rejects_missing_required_arguments() {
    let responses = stdio_session(&[json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "game_info", "arguments": {} }
    })]);

    assert!(is_error(&responses[&1]));
    assert!(tool_text(&responses[&1]).contains("Missing required argument: game"));
}

fn wait_for_port(port: u16) -> bool {
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

struct HttpServer {
    child: Child,
    port: u16,
}

impl HttpServer {
    fn start() -> Self {
        let port = free_port();
        let child = Command::new(BIN)
            .args(["mcp-server", "--transport", "http", "--port", &port.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start HTTP MCP server");

        assert!(wait_for_port(port), "HTTP MCP server never listened on {port}");
        Self { child, port }
    }

    fn post(&self, body: &Value) -> Value {
        let payload = serde_json::to_string(body).unwrap();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        )
        .unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .expect("malformed HTTP response");
        assert!(
            headers.starts_with("HTTP/1.1 200"),
            "unexpected status: {headers}"
        );
        serde_json::from_str(body).unwrap_or_else(|error| panic!("bad JSON body: {error}: {body}"))
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn http_transport_answers_initialize_and_tool_calls() {
    let server = HttpServer::start();

    let init = server.post(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05" }
    }));
    assert_eq!(init["result"]["serverInfo"]["name"], "game-opt-tool");

    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game.bin");
    std::fs::write(&game, b"http transport test").unwrap();

    let call = server.post(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": "backup", "arguments": { "game": game.to_str().unwrap() } }
    }));
    assert_eq!(call["result"]["isError"], false);
}
