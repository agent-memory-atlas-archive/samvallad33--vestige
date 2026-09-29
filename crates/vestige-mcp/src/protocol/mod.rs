//! MCP Protocol Implementation
//!
//! JSON-RPC 2.0 over stdio for the Model Context Protocol.

pub mod auth;
// http/stdio transports host the McpServer, which holds `Arc<Storage>`:
// behind `legacy-sqlite` (build/t5-legacy-isolation).
#[cfg(feature = "legacy-sqlite")]
pub mod http;
pub mod messages;
#[cfg(feature = "legacy-sqlite")]
pub mod stdio;
pub mod types;
