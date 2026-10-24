//! Read-only, bounded salience packets for local lifecycle hooks.
//! Prompt words select explicit recorded entity handles; they never rank memories.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use vestige_core::{KnowledgeNode, Storage};

pub(crate) fn scores(storage: &Storage) -> BTreeMap<String, i128> {
    let mut scores = BTreeMap::new();
    for row in storage.salience_receipts().unwrap_or_default() {
        if let (Some(id), Some(old), Some(new)) = (
            row["memory_id"].as_str(),
            row["old_stability_q"].as_i64(),
            row["new_stability_q"].as_i64(),
        ) {
            *scores.entry(id.to_owned()).or_default() += i128::from(new) - i128::from(old);
        }
    }
    scores
}

pub(crate) fn compare(
    a: &KnowledgeNode,
    b: &KnowledgeNode,
    scores: &BTreeMap<String, i128>,
) -> std::cmp::Ordering {
    scores
        .get(&b.id)
        .copied()
        .unwrap_or_default()
        .cmp(&scores.get(&a.id).copied().unwrap_or_default())
        .then_with(|| b.stability.total_cmp(&a.stability))
        .then_with(|| b.created_at.cmp(&a.created_at))
        .then_with(|| b.id.cmp(&a.id))
}

fn phrase(text: &str, surface: &str) -> bool {
    let text = text.to_lowercase();
    let surface = surface.to_lowercase();
    if surface.is_empty() {
        return false;
    }
    text.match_indices(&surface).any(|(at, _)| {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        text[..at].chars().next_back().is_none_or(|c| !word(c))
            && text[at + surface.len()..]
                .chars()
                .next()
                .is_none_or(|c| !word(c))
    })
}

pub(crate) fn packet(storage: &Storage, scope: &str, args: &Value) -> Result<Value, String> {
    let prompt = args["prompt"].as_str().unwrap_or_default();
    if prompt.len() > 64 * 1024 {
        return Err("hook prompt exceeds 64 KiB".into());
    }
    let mut nodes = storage
        .get_all_nodes_in_scope(scope, i32::MAX, 0)
        .map_err(|e| e.to_string())?;
    let retired = storage.superseded_node_ids().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now();
    nodes.retain(|n| {
        !retired.contains(&n.id)
            && n.valid_from.is_none_or(|t| t <= now)
            && n.valid_until.is_none_or(|t| t > now)
    });
    let count = nodes.len();
    let entities: BTreeSet<String> = nodes
        .iter()
        .flat_map(|n| n.tags.iter())
        .filter(|tag| {
            ["person:", "place:", "food:", "file:", "service:", "symbol:"]
                .iter()
                .any(|p| tag.starts_with(p))
        })
        .filter(|tag| {
            phrase(prompt, tag) || tag.split_once(':').is_some_and(|(_, s)| phrase(prompt, s))
        })
        .cloned()
        .collect();
    if !entities.is_empty() {
        nodes.retain(|n| n.tags.iter().any(|t| entities.contains(t)));
    }
    let scores = scores(storage);
    nodes.sort_by(|a, b| compare(a, b, &scores));
    let limit = if !entities.is_empty() && args["trigger"]["mode"] == "light" {
        8
    } else {
        args["limit"].as_u64().unwrap_or(3).clamp(1, 8) as usize
    };
    nodes.truncate(limit);
    let ids: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let receipts: Vec<Value> = storage
        .salience_receipts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .rev()
        .filter(|r| r["memory_id"].as_str().is_some_and(|id| ids.contains(id)))
        .take(8)
        .collect();
    Ok(
        json!({"memory_count":count,"entities":entities,"memories":nodes.iter().map(|n|
        json!({"id":n.id,"content":n.content.chars().take(700).collect::<String>(),"tags":n.tags,
            "stability":n.stability,"salience_delta_q":scores.get(&n.id).copied().unwrap_or_default().to_string()})).collect::<Vec<_>>(),
        "backfill_receipts":receipts,"backfill_autofire":super::backfill::autofire_enabled(),
        "selection":if entities.is_empty(){"top_salience"}else{"exact_recorded_entities"},
        "trigger":args.get("trigger"),"ranking_signal":"receipt_salience_then_stability_then_recency"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entity_surfaces_use_case_insensitive_whole_boundaries() {
        assert!(phrase("Where should MAYA go?", "Maya"));
        assert!(!phrase("Mayakovsky", "Maya"));
        assert!(phrase("try Bangkok Garden?", "Bangkok Garden"));
        assert!(!phrase("Bangkok Gardens", "Bangkok Garden"));
    }
}
