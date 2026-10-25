/// MCP Server — Entry point for AI agent integration
use axum::{
    extract::{Extension, State},
    http::{Request, Response},
    middleware::{self, from_fn},
    Router,
};
use hyper::body::Bytes;
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::mcp::{parse_mcp_request, McpRequest, McpResponse};

/// Application state shared across handlers
struct AppState {
    patcher: src::patcher::Patcher,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            patcher: src::patcher::Patcher::new(),
        })
    }
}

async fn handle_mcp(
    State(state): State<AppState>,
    request: hyper::Request<Bytes>,
) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri;

    // Try to parse as MCP request
    let result = match parse_mcp_request(&body) {
        Ok(req) => handle_request(&state, &req).await,
        Err(e) => {
            McpResponse::Error {
                success: false,
                message: format!("Invalid MCP request: {}", e),
            }
        }
    };

    // Build JSON-RPC 2.0 response
    let id = "1"; // Extract from real request in production
    format!(
        r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#,
        // Simplified - real implementation would serialize the McpResponse
        serde_json::to_string(&McpResponse::Error {
            success: false,
            message: "MCP handler placeholder".into(),
        }).unwrap_or_else(|_| r#"{"jsonrpc":"2.0","error":{"code":-32600","message":"Invalid request"}}id":1}"#
    )
}

pub async fn run_server(addr: std::net::SocketAddr) -> anyhow::Result<()> {
    // Simple TCP-based MCP for now
    // Real implementation would use proper JSON-RPC over stdin/stdout or HTTP
    
    let listener = std::net::TcpListener::bind(addr)?;
    info!("MCP server listening on {}", addr);
    
    loop {
        let (stream, _) = listener.accept().await?;
        // Handle connection - simplified for now
        let _ = stream;
    }
    
    Ok(())
}

/// Start the MCP server with the given address
pub async fn start(address: &str) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = addr.parse()?;
    run_server(addr).await
}
