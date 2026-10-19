//! Explore connections tool — Graph exploration, chain building, bridge discovery.
//!
//! On Strata, chain, associations, and bridges walk recorded typed edges only.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cognitive::CognitiveEngine;
use vestige_core::Storage;

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["chain", "associations", "bridges"],
                "description": "Type of exploration: 'chain' builds reasoning path, 'associations' finds related memories, 'bridges' finds connecting memories"
            },
            "from": {
                "type": "string",
                "description": "Source memory ID"
            },
            "to": {
                "type": "string",
                "description": "Target memory ID (required for 'chain' and 'bridges')"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum results (default: 10)",
                "default": 10
            }
        },
        "required": ["action", "from"]
    })
}

pub async fn execute(
    storage: &Arc<Storage>,
    _cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let args = args.ok_or("Missing arguments")?;
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'action'")?;
    let from = args
        .get("from")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'from'")?;
    let to = args.get("to").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    crate::strata_memory::ghostlink::explore(
        storage.as_ref(),
        action,
        from,
        to,
        limit.clamp(1, 100),
    )
}
