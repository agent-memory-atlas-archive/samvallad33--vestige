//! MCP Protocol Implementation
//!
//! JSON-RPC 2.0 over stdio for the Model Context Protocol.

pub mod auth;
// http/stdio transports host the McpServer, which holds `Arc<Storage>`:
// behind `legacy-sqlite` (build/t5-legacy-isolation).
pub mod http;
pub mod messages;
pub mod stdio;
pub mod types;
