//! Review Tool (Deprecated)
//!
//! Mark memories as reviewed using FSRS-6 algorithm.

use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use vestige_core::{Rating, Storage};

/// Input schema for mark_reviewed tool
pub fn schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The ID of the memory to review"
            },
            "rating": {
                "type": "integer",
                "description": "Review rating: 1=Again (forgot), 2=Hard, 3=Good, 4=Easy",
                "minimum": 1,
                "maximum": 4,
                "default": 3
            }
        },
        "required": ["id"]
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewArgs {
    id: String,
    rating: Option<i32>,
}

pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String> {
    let args: ReviewArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => return Err("Missing arguments".to_string()),
    };

    // SQLite ids are UUIDs. Strata ids are `mem-` + 16 hex chars.
    if uuid::Uuid::parse_str(&args.id).is_err() && !is_strata_memory_id(&args.id) {
        return Err("Invalid node ID format".to_string());
    }

    let rating_value = args.rating.unwrap_or(3);
    if !(1..=4).contains(&rating_value) {
        return Err("Rating must be between 1 and 4".to_string());
    }

    let rating =
        Rating::from_i32(rating_value).ok_or_else(|| "Invalid rating value".to_string())?;

    // Get node before review for comparison
    let before = storage
        .get_node(&args.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node not found: {}", args.id))?;

    let node = storage
        .mark_reviewed(&args.id, rating)
        .map_err(|e| e.to_string())?;

    let rating_name = match rating {
        Rating::Again => "Again",
        Rating::Hard => "Hard",
        Rating::Good => "Good",
        Rating::Easy => "Easy",
    };

    Ok(serde_json::json!({
        "success": true,
        "nodeId": node.id,
        "rating": rating_name,
        "fsrs": {
            "previousRetention": before.retention_strength,
            "newRetention": node.retention_strength,
            "previousStability": before.stability,
            "newStability": node.stability,
            "difficulty": node.difficulty,
            "reps": node.reps,
            "lapses": node.lapses,
        },
        "nextReview": node.next_review.map(|d| d.to_rfc3339()),
        "message": format!("Memory reviewed with rating '{}'. Retention: {:.2} -> {:.2}",
            rating_name, before.retention_strength, node.retention_strength),
    }))
}

pub(crate) fn is_strata_memory_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("mem-") else {
        return false;
    };
    rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_hexdigit())
}

// ============================================================================
// TESTS
// ============================================================================
