//! Memory Timeline Tool
//!
//! Browse memories chronologically. Returns memories in a time range,
//! grouped by day. Defaults to last 7 days.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

use vestige_core::{OutputConfig, Storage};

use super::search_unified::{apply_output_masks, format_node};

/// Input schema for memory_timeline tool
pub fn schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "start": {
                "type": "string",
                "description": "Start of time range (ISO 8601 date or datetime). Default: 7 days ago."
            },
            "end": {
                "type": "string",
                "description": "End of time range (ISO 8601 date or datetime). Default: now."
            },
            "node_type": {
                "type": "string",
                "description": "Filter by node type (e.g. 'fact', 'concept', 'decision')"
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Filter by tags (ANY match)"
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of memories to return (default: 50, max: 200)",
                "default": 50,
                "minimum": 1,
                "maximum": 200
            },
            "detail_level": {
                "type": "string",
                "description": "Level of detail: 'brief', 'summary' (default), or 'full'",
                "enum": ["brief", "summary", "full"],
                "default": "summary"
            }
        }
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimelineArgs {
    start: Option<String>,
    end: Option<String>,
    #[serde(alias = "node_type")]
    node_type: Option<String>,
    tags: Option<Vec<String>>,
    limit: Option<i32>,
    #[serde(alias = "detail_level")]
    detail_level: Option<String>,
}

/// Parse an ISO 8601 date or datetime string into a DateTime<Utc>.
/// Supports both `2026-02-01` and `2026-02-01T00:00:00Z` formats.
fn parse_datetime(s: &str) -> Result<DateTime<Utc>, String> {
    // Try full datetime first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Try date-only (YYYY-MM-DD)
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| format!("Invalid date: {}", s))?
            .and_utc();
        return Ok(dt);
    }
    Err(format!(
        "Invalid date/datetime '{}'. Use ISO 8601 format: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ",
        s
    ))
}

/// Execute memory_timeline tool
pub async fn execute(
    storage: &Arc<Storage>,
    output_config: &OutputConfig,
    args: Option<Value>,
) -> Result<Value, String> {
    let args: TimelineArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => TimelineArgs {
            start: None,
            end: None,
            node_type: None,
            tags: None,
            limit: None,
            detail_level: None,
        },
    };

    // Validate detail_level. Precedence: explicit MCP param > config > default.
    let detail_level_owned = output_config.resolve_detail_level(args.detail_level.as_deref());
    let detail_level = match detail_level_owned.as_str() {
        "brief" => "brief",
        "full" => "full",
        "summary" => "summary",
        invalid => {
            return Err(format!(
                "Invalid detail_level '{}'. Must be 'brief', 'summary', or 'full'.",
                invalid
            ));
        }
    };

    // Parse time range
    let now = Utc::now();
    let start = match &args.start {
        Some(s) => Some(parse_datetime(s)?),
        None => Some(now - chrono::Duration::days(7)),
    };
    let end = match &args.end {
        Some(e) => Some(parse_datetime(e)?),
        None => Some(now),
    };

    // Precedence: explicit MCP param > config limit > built-in default (50).
    let limit = output_config.resolve_limit(args.limit, 50).clamp(1, 200);

    // Query memories in time range with filters pushed into SQL. Rust-side
    // `retain` after `LIMIT` was unsafe for sparse types/tags — a dominant
    // set could crowd the sparse matches out of the limit window and leave
    // the retain with 0 rows to keep.
    let results = storage
        .query_time_range(
            start,
            end,
            limit,
            args.node_type.as_deref(),
            args.tags.as_deref(),
        )
        .map_err(|e| e.to_string())?;

    // Group by day, applying the active profile's field masks (e.g. `lean`
    // drops timestamps) to each formatted node.
    let mut by_day: BTreeMap<NaiveDate, Vec<Value>> = BTreeMap::new();
    for node in &results {
        let date = node.created_at.date_naive();
        let mut formatted = [format_node(node, detail_level)];
        apply_output_masks(&mut formatted, output_config);
        let [formatted] = formatted;
        by_day.entry(date).or_default().push(formatted);
    }

    // Build timeline (newest first)
    let timeline: Vec<Value> = by_day
        .into_iter()
        .rev()
        .map(|(date, memories)| {
            serde_json::json!({
                "date": date.to_string(),
                "count": memories.len(),
                "memories": memories,
            })
        })
        .collect();

    let total = results.len();
    let days = timeline.len();

    Ok(serde_json::json!({
        "tool": "memory_timeline",
        "range": {
            "start": start.map(|dt| dt.to_rfc3339()),
            "end": end.map(|dt| dt.to_rfc3339()),
        },
        "detailLevel": detail_level,
        "profile": output_config.profile.as_str(),
        // `totalMemories` is the RETURNED count (bounded by limit), kept for
        // compatibility. `returned`/`truncated` make the bound explicit so
        // callers do not mistake it for the range's full population.
        "totalMemories": total,
        "returned": total,
        "truncated": total >= limit as usize,
        "days": days,
        "timeline": timeline,
    }))
}
