//! Dream tool — Explicit dream trigger that replays recorded edges and folds FSRS.
//!
//! The Strata path does not score content. It consolidates one id page by
//! replaying recorded edges whose strength meets the floor, then folds one
//! FSRS review per endpoint from that card's state alone.

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cognitive::CognitiveEngine;
use vestige_core::Storage;

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "scope":{"type":"string","default":"user"},
            "after":{"type":"string","description":"ID cursor from previous dream page. Cross-page pairs are outside this page's analysis."},
            "max_pairs":{"type":"integer","minimum":10,"maximum":124750,"default":1225,"description":"Upper bound for within-page memory pairs; reduces selected memory count."},
            "memory_count": {
                "type": "integer",
                "description": "Maximum memories in a scoped ID page (default: 50)",
                "default": 50, "minimum":5, "maximum":500
            },
            "min_similarity": {
                "type": "number",
                "description": "Strata: the minimum strength a recorded edge needs to be replayed, echoed back as edgeStrengthFloor. 0.0-1.0, default 0.5.",
                "minimum": 0.0,
                "maximum": 1.0,
                "default": 0.5
            }
        }
    })
}

pub async fn execute(
    storage: &Arc<Storage>,
    _cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        memory_count: Option<usize>,
        min_similarity: Option<f64>,
        max_pairs: Option<usize>,
        after: Option<String>,
        scope: Option<String>,
    }
    let parsed: Args = serde_json::from_value(args.unwrap_or_else(|| serde_json::json!({})))
        .map_err(|e| e.to_string())?;
    let requested = parsed.memory_count.unwrap_or(50);
    let max_pairs = parsed.max_pairs.unwrap_or(1225);
    if !(5..=500).contains(&requested)
        || !(10..=124750).contains(&max_pairs)
        || parsed
            .min_similarity
            .is_some_and(|n| !n.is_finite() || !(0.0..=1.0).contains(&n))
    {
        return Err("invalid memory_count, max_pairs or min_similarity".into());
    }
    let mut memory_count = requested;
    while memory_count * (memory_count - 1) / 2 > max_pairs {
        memory_count -= 1;
    }
    let scope = parsed.scope.unwrap_or_else(|| "user".into());
    if scope.trim().is_empty() {
        return Err("scope must not be empty".into());
    }
    execute_strata(
        storage,
        StrataDreamArgs {
            scope,
            memory_count,
            max_pairs,
            after: parsed.after,
            edge_strength_floor: parsed.min_similarity.unwrap_or(0.5),
        },
    )
    .await
}

struct StrataDreamArgs {
    scope: String,
    memory_count: usize,
    max_pairs: usize,
    after: Option<String>,
    edge_strength_floor: f64,
}

/// Replay recorded edges and fold FSRS. Content, tags, and names are not read.
async fn execute_strata(
    storage: &Arc<Storage>,
    args: StrataDreamArgs,
) -> Result<serde_json::Value, String> {
    let started = std::time::Instant::now();
    let (page, has_more) = storage
        .maintenance_memory_page(args.memory_count, args.after.as_deref(), &args.scope)
        .map_err(|e| e.to_string())?;
    let next_cursor = page.last().map(|node| node.id.clone());
    let mut live: Vec<vestige_core::KnowledgeNode> = page
        .into_iter()
        .filter(|node| node.suppression_count == 0 && node.is_currently_valid())
        .collect();
    live.sort_by(|left, right| left.id.cmp(&right.id));
    if live.len() < 5 {
        return Ok(serde_json::json!({
            "hasMore": has_more,
            "nextCursor": next_cursor,
            "scope": args.scope,
            "maxPairs": args.max_pairs,
            "status": "insufficient_memories",
            "message": format!("Need at least 5 memories to dream. Current count: {}", live.len()),
            "count": live.len()
        }));
    }

    let page_ids: std::collections::BTreeSet<&str> =
        live.iter().map(|node| node.id.as_str()).collect();
    let mut qualifying = Vec::new();
    for edge in storage.get_all_connections().map_err(|e| e.to_string())? {
        if edge.strength < args.edge_strength_floor {
            continue;
        }
        if page_ids.contains(edge.source_id.as_str()) && page_ids.contains(edge.target_id.as_str())
        {
            qualifying.push(edge);
        }
    }
    qualifying.sort_by(|left, right| {
        (&left.source_id, &left.target_id, &left.link_type).cmp(&(
            &right.source_id,
            &right.target_id,
            &right.link_type,
        ))
    });

    let mut pairs = std::collections::BTreeSet::new();
    let mut endpoints = std::collections::BTreeSet::new();
    for edge in &qualifying {
        let (first, second) = if edge.source_id <= edge.target_id {
            (edge.source_id.clone(), edge.target_id.clone())
        } else {
            (edge.target_id.clone(), edge.source_id.clone())
        };
        pairs.insert((first, second));
        endpoints.insert(edge.source_id.clone());
        endpoints.insert(edge.target_id.clone());
    }

    let by_id: std::collections::HashMap<&str, &vestige_core::KnowledgeNode> =
        live.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut reviews = Vec::with_capacity(endpoints.len());
    let mut memories_strengthened = 0u64;
    for id in &endpoints {
        let node = by_id
            .get(id.as_str())
            .ok_or_else(|| format!("recorded edge endpoint {id} left the page"))?;
        let rating = fsrs_replay_rating(node);
        if rating.as_i32() >= 3 {
            memories_strengthened += 1;
        }
        reviews.push(serde_json::json!({
            "id": id,
            "rating": rating.as_i32(),
            "retention": node.retention_strength,
            "stability": node.stability,
            "repsBefore": node.reps,
            "lapses": node.lapses,
        }));
        storage
            .mark_reviewed(id, rating)
            .map_err(|e| e.to_string())?;
    }

    let dream_record = vestige_core::DreamHistoryRecord {
        dreamed_at: chrono::Utc::now(),
        duration_ms: started.elapsed().as_millis() as i64,
        memories_replayed: live.len() as i32,
        connections_found: pairs.len() as i32,
        insights_generated: 0,
        memories_strengthened: reviews.len() as i32,
        memories_compressed: 0,
        phase_nrem1_ms: None,
        phase_nrem3_ms: None,
        phase_rem_ms: None,
        phase_integration_ms: None,
        summaries_generated: None,
        emotional_memories_processed: None,
        creative_connections_found: None,
    };
    let dream_recorded = storage.save_dream_history(&dream_record).is_ok();

    Ok(serde_json::json!({
        "hasMore": has_more,
        "nextCursor": next_cursor,
        "scope": args.scope,
        "maxPairs": args.max_pairs,
        "status": "completed",
        "dreamRecorded": dream_recorded,
        "basis": "recorded_edges_fsrs",
        "selection": "recorded_edges_fsrs",
        "edgeStrengthFloor": args.edge_strength_floor,
        "memoriesReplayed": live.len(),
        "edgesConsidered": qualifying.len(),
        "edgesConsolidated": pairs.len(),
        "memoriesReviewed": reviews.len(),
        "memoriesStrengthened": memories_strengthened,
        "connectionsFound": pairs.len(),
        "discovery": super::unavailable::part(
            super::unavailable::EMBEDDINGS_UNAVAILABLE,
            "Finding new connections between memories needs embeddings, which this build does not ship. A dream here replays and re-weights edges the log already recorded.",
        ),
        "emptyBecause": if qualifying.is_empty() {
            Some(format!(
                "no recorded edge with strength >= {} joins two memories on this page, so there was nothing to replay; record typed edges, or lower min_similarity (the edge-strength floor)",
                args.edge_strength_floor
            ))
        } else {
            None
        },
        "insights": [],
        "connectionsPersisted": 0,
        "insightsPersisted": 0,
        "wakingTagsProcessed": 0,
        "wakingTagsCleared": 0,
        "reviews": reviews,
        "durationMs": started.elapsed().as_millis() as u64,
        "stats": {
            "new_connections_found": 0,
            "connections_persisted": 0,
            "insights_persisted": 0,
            "memories_strengthened": memories_strengthened,
            "memories_compressed": 0,
            "insights_generated": 0,
            "duration_ms": started.elapsed().as_millis() as u64,
        }
    }))
}

fn fsrs_replay_rating(node: &vestige_core::KnowledgeNode) -> vestige_core::Rating {
    let score = if node.lapses > node.reps {
        1
    } else if node.retention_strength >= 0.9 && node.stability >= 1.0 {
        4
    } else if node.retention_strength >= 0.5 {
        3
    } else if node.retention_strength >= 0.2 {
        2
    } else {
        1
    };
    vestige_core::Rating::from_i32(score).expect("replay rating is 1..=4")
}

#[cfg(test)]
mod strata_dream {
    use super::*;
    use crate::cognitive::CognitiveEngine;
    use vestige_core::{ConnectionRecord, IngestInput};

    fn engine() -> Arc<Mutex<CognitiveEngine>> {
        Arc::new(Mutex::new(CognitiveEngine::new()))
    }

    fn no_sqlite(dir: &std::path::Path) -> bool {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.ends_with(".sqlite")
                    || name.ends_with(".sqlite3")
                    || name.ends_with(".db")
                    || name.ends_with(".db-wal")
                    || name.ends_with(".db-shm")
                {
                    return false;
                }
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }
        true
    }

    fn seed(dir: &std::path::Path) -> (Arc<Storage>, Vec<String>, Vec<String>) {
        let storage = crate::strata_memory::open(dir).unwrap();
        let mut ids = Vec::new();
        for i in 0..6 {
            let node = storage
                .ingest(IngestInput {
                    content: format!("shared token salad {i} alpha beta gamma"),
                    ..IngestInput::default()
                })
                .unwrap();
            ids.push(node.id);
        }
        let now = chrono::Utc::now();
        let link = |source: &str, target: &str, kind: &str, strength: f64| {
            storage
                .save_connection(&ConnectionRecord {
                    source_id: source.to_string(),
                    target_id: target.to_string(),
                    strength,
                    link_type: kind.to_string(),
                    created_at: now,
                    last_activated: now,
                    activation_count: 1,
                })
                .unwrap();
        };
        link(&ids[0], &ids[1], "derived_from", 1.0);
        link(&ids[1], &ids[2], "evidence_of", 0.8);
        link(&ids[3], &ids[4], "touched", 0.1);
        let strong = vec![ids[0].clone(), ids[1].clone(), ids[2].clone()];
        let quiet = vec![ids[3].clone(), ids[4].clone(), ids[5].clone()];
        (storage, strong, quiet)
    }

    fn reviewed_ids(value: &serde_json::Value) -> Vec<String> {
        value["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .map(|review| review["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn strata_dream_consolidates_recorded_edges_and_fsrs_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let (storage, strong, quiet) = seed(dir.path());
        let reps_before = storage.get_node(&strong[0]).unwrap().unwrap().reps;

        let value = execute(&storage, &engine(), None).await.unwrap();
        assert_eq!(value["status"], "completed");
        assert_eq!(value["basis"], "recorded_edges_fsrs");
        assert_eq!(value["edgesConsolidated"], 2);
        assert_eq!(value["connectionsPersisted"], 0);
        assert_eq!(value["insights"].as_array().unwrap().len(), 0);
        let text = serde_json::to_string(&value).unwrap();
        assert!(!text.contains("pending_strata"), "{text}");
        assert!(!text.contains("not implemented"), "{text}");
        assert!(!text.contains("token salad"), "{text}");

        let ids = reviewed_ids(&value);
        let mut expected = strong.clone();
        expected.sort();
        assert_eq!(ids, expected, "{value}");
        for id in &strong {
            let node = storage.get_node(id).unwrap().unwrap();
            assert_eq!(node.reps, reps_before + 1, "{id}");
        }
        for id in &quiet {
            let node = storage.get_node(id).unwrap().unwrap();
            assert_eq!(node.reps, reps_before, "{id}");
        }
        assert!(no_sqlite(dir.path()));
    }

    #[tokio::test]
    async fn strata_dream_is_deterministic_and_ignores_subthreshold_edges() {
        let left_dir = tempfile::TempDir::new().unwrap();
        let right_dir = tempfile::TempDir::new().unwrap();
        let (left, _, _) = seed(left_dir.path());
        let (right, _, _) = seed(right_dir.path());
        let left_reviews = execute(&left, &engine(), None).await.unwrap()["reviews"].clone();
        let right_reviews = execute(&right, &engine(), None).await.unwrap()["reviews"].clone();
        assert_eq!(left_reviews, right_reviews);

        let dir = tempfile::TempDir::new().unwrap();
        let (storage, _, quiet) = seed(dir.path());
        let widened = execute(
            &storage,
            &engine(),
            Some(serde_json::json!({ "min_similarity": 0.0 })),
        )
        .await
        .unwrap();
        assert_eq!(widened["status"], "completed");
        assert_eq!(widened["edgesConsolidated"], 3);
        let ids = reviewed_ids(&widened);
        assert_eq!(ids.len(), 5);
        assert!(!ids.contains(&quiet[2]));
    }

    #[tokio::test]
    async fn strata_dream_below_five_is_insufficient() {
        let dir = tempfile::TempDir::new().unwrap();
        let storage = crate::strata_memory::open(dir.path()).unwrap();
        for i in 0..3 {
            storage
                .ingest(IngestInput {
                    content: format!("short dream page {i}"),
                    ..IngestInput::default()
                })
                .unwrap();
        }
        let value = execute(&storage, &engine(), None).await.unwrap();
        assert_eq!(value["status"], "insufficient_memories");
        assert_eq!(value["count"], 3);
    }
}
