//! Predict tool — Proactive memory prediction from exact code anchors.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cognitive::CognitiveEngine;
use vestige_core::Storage;

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "context": {
                "type": "object",
                "description": "Current context for prediction",
                "properties": {
                    "current_file": { "type": "string" },
                    "current_topics": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "codebase": { "type": "string" }
                }
            }
        }
    })
}

pub async fn execute(
    storage: &Arc<Storage>,
    _cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let context = args.as_ref().and_then(|a| a.get("context"));
    crate::strata_memory::ghostlink::predict(storage.as_ref(), context)
}
