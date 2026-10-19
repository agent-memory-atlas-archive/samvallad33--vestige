//! memory_health tool — Retention dashboard for memory quality monitoring.
//! v1.9.0: Lightweight alternative to full system_status focused on memory health.

use std::sync::Arc;
use vestige_core::Storage;

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

pub async fn execute(
    storage: &Arc<Storage>,
    _args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    // Average retention
    let avg_retention = storage
        .get_avg_retention()
        .map_err(|e| format!("Failed to get avg retention: {}", e))?;

    // Retention distribution
    let distribution = storage
        .get_retention_distribution()
        .map_err(|e| format!("Failed to get retention distribution: {}", e))?;

    let distribution_json: serde_json::Value = distribution
        .iter()
        .map(|(bucket, count)| serde_json::json!({ "bucket": bucket, "count": count }))
        .collect();

    // Retention trend
    let trend = storage
        .get_retention_trend()
        .unwrap_or_else(|_| "unknown".to_string());

    // Total memories and those below key thresholds
    let stats = storage
        .get_stats()
        .map_err(|e| format!("Failed to get stats: {}", e))?;

    let below_30 = storage.count_memories_below_retention(0.3).unwrap_or(0);
    let below_50 = storage.count_memories_below_retention(0.5).unwrap_or(0);

    // Retention target
    let retention_target: f64 = std::env::var("VESTIGE_RETENTION_TARGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.8);

    let meets_target = avg_retention >= retention_target;

    // Generate recommendation
    let recommendation = if avg_retention >= 0.8 {
        "Excellent memory health. Retention is strong across the board."
    } else if avg_retention >= 0.6 {
        "Good memory health. Consider reviewing memories in the 0-40% range."
    } else if avg_retention >= 0.4 {
        "Fair memory health. Many memories are decaying; promote the ones still needed."
    } else {
        "Poor memory health. Urgent: promote the memories still needed; review those below 0.3 retention."
    };

    Ok(serde_json::json!({
        "avgRetention": format!("{:.1}%", avg_retention * 100.0),
        "avgRetentionRaw": avg_retention,
        "retentionTarget": retention_target,
        "meetsTarget": meets_target,
        "totalMemories": stats.total_nodes,
        "distribution": distribution_json,
        "trend": trend,
        "memoriesBelow30pct": below_30,
        "memoriesBelow50pct": below_50,
        "recommendation": recommendation,
    }))
}
