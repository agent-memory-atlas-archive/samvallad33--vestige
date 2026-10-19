//! memory_graph tool — Subgraph export with force-directed layout for visualization.
//! v1.9.0: Computes Fruchterman-Reingold layout server-side.

use std::sync::Arc;
use vestige_core::Storage;

pub fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "center_id": {
                "type": "string",
                "description": "Memory ID to center the graph on. Required if no query."
            },
            "query": {
                "type": "string",
                "description": "Search query to find center node. Used if center_id not provided."
            },
            "depth": {
                "type": "integer",
                "description": "How many hops from center to include (1-3, default: 2)",
                "default": 2,
                "minimum": 1,
                "maximum": 3
            },
            "max_nodes": {
                "type": "integer",
                "description": "Maximum number of nodes to include (default: 50)",
                "default": 50,
                "maximum": 200
            }
        }
    })
}

/// Simple Fruchterman-Reingold force-directed layout
fn fruchterman_reingold(
    node_count: usize,
    edges: &[(usize, usize, f64)],
    width: f64,
    height: f64,
    iterations: usize,
) -> Vec<(f64, f64)> {
    if node_count == 0 {
        return Vec::new();
    }
    if node_count == 1 {
        return vec![(width / 2.0, height / 2.0)];
    }

    let area = width * height;
    let k = (area / node_count as f64).sqrt();

    // Initialize positions in a circle
    let mut positions: Vec<(f64, f64)> = (0..node_count)
        .map(|i| {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / node_count as f64;
            (
                width / 2.0 + (width / 3.0) * angle.cos(),
                height / 2.0 + (height / 3.0) * angle.sin(),
            )
        })
        .collect();

    let mut temperature = width / 10.0;
    let cooling = temperature / iterations as f64;

    for _ in 0..iterations {
        let mut displacements = vec![(0.0f64, 0.0f64); node_count];

        // Repulsive forces between all pairs
        for i in 0..node_count {
            for j in (i + 1)..node_count {
                let dx = positions[i].0 - positions[j].0;
                let dy = positions[i].1 - positions[j].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = k * k / dist;
                let fx = dx / dist * force;
                let fy = dy / dist * force;
                displacements[i].0 += fx;
                displacements[i].1 += fy;
                displacements[j].0 -= fx;
                displacements[j].1 -= fy;
            }
        }

        // Attractive forces along edges
        for &(u, v, weight) in edges {
            let dx = positions[u].0 - positions[v].0;
            let dy = positions[u].1 - positions[v].1;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let force = dist * dist / k * weight;
            let fx = dx / dist * force;
            let fy = dy / dist * force;
            displacements[u].0 -= fx;
            displacements[u].1 -= fy;
            displacements[v].0 += fx;
            displacements[v].1 += fy;
        }

        // Apply displacements with temperature limiting
        for i in 0..node_count {
            let dx = displacements[i].0;
            let dy = displacements[i].1;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let capped = dist.min(temperature);
            positions[i].0 += dx / dist * capped;
            positions[i].1 += dy / dist * capped;

            // Clamp to bounds
            positions[i].0 = positions[i].0.clamp(10.0, width - 10.0);
            positions[i].1 = positions[i].1.clamp(10.0, height - 10.0);
        }

        temperature -= cooling;
        if temperature < 0.1 {
            break;
        }
    }

    positions
}

/// Exact center for a Strata `memory_graph`. `query` is refused: picking a
/// center by text would be keyword or semantic search, which this log does
/// not admit.
fn strata_center(
    storage: &Arc<Storage>,
    args: Option<&serde_json::Value>,
) -> Result<String, String> {
    if let Some(id) = args
        .and_then(|a| a.get("center_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return Ok(id.to_string());
    }
    if args
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
        .is_some_and(|query| !query.trim().is_empty())
    {
        return Err(
            "similarity_disabled: memory_graph: embeddings, cosine, BM25, FTS, Jaccard, and keyword or name matching are not Strata operations; pass center_id"
                .into(),
        );
    }
    let superseded = storage.superseded_node_ids().map_err(|e| e.to_string())?;
    storage
        .get_all_nodes(i32::MAX, 0)
        .map_err(|e| format!("Failed to get recent node: {e}"))?
        .into_iter()
        .find(|node| !superseded.contains(&node.id))
        .map(|node| node.id)
        .ok_or_else(|| "No memories in database".to_string())
}

pub async fn execute(
    storage: &Arc<Storage>,
    args: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let depth = args
        .as_ref()
        .and_then(|a| a.get("depth"))
        .and_then(|v| v.as_u64())
        .unwrap_or(2)
        .min(3) as u32;

    let max_nodes = args
        .as_ref()
        .and_then(|a| a.get("max_nodes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(200) as usize;

    // Determine center node. On Strata the center is an exact handle or the
    // newest live record — a query is not a search.
    let center_id = if crate::strata_memory::is_strata_backend(storage.as_ref()) {
        strata_center(storage, args.as_ref())?
    } else if let Some(id) = args
        .as_ref()
        .and_then(|a| a.get("center_id"))
        .and_then(|v| v.as_str())
    {
        id.to_string()
    } else if let Some(query) = args
        .as_ref()
        .and_then(|a| a.get("query"))
        .and_then(|v| v.as_str())
    {
        // Search for center node
        let results = storage
            .search(query, 1)
            .map_err(|e| format!("Search failed: {}", e))?;
        results
            .first()
            .map(|n| n.id.clone())
            .ok_or_else(|| "No memories found matching query".to_string())?
    } else {
        // Default: use the most recent memory
        let recent = storage
            .get_all_nodes(1, 0)
            .map_err(|e| format!("Failed to get recent node: {}", e))?;
        recent
            .first()
            .map(|n| n.id.clone())
            .ok_or_else(|| "No memories in database".to_string())?
    };

    // Get subgraph
    let (nodes, edges) = storage
        .get_memory_subgraph(&center_id, depth, max_nodes)
        .map_err(|e| format!("Failed to get subgraph: {}", e))?;

    if nodes.is_empty() || !nodes.iter().any(|n| n.id == center_id) {
        return Err(format!(
            "Memory '{}' not found or has no accessible data",
            center_id
        ));
    }

    // Build index map for FR layout
    let id_to_idx: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let layout_edges: Vec<(usize, usize, f64)> = edges
        .iter()
        .filter_map(|e| {
            let u = id_to_idx.get(e.source_id.as_str())?;
            let v = id_to_idx.get(e.target_id.as_str())?;
            Some((*u, *v, e.strength))
        })
        .collect();

    // Compute force-directed layout
    let positions = fruchterman_reingold(nodes.len(), &layout_edges, 800.0, 600.0, 50);

    // Build response
    let nodes_json: Vec<serde_json::Value> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let (x, y) = positions.get(i).copied().unwrap_or((400.0, 300.0));
            serde_json::json!({
                "id": n.id,
                "label": if n.content.chars().count() > 60 {
                    format!("{}...", n.content.chars().take(57).collect::<String>())
                } else {
                    n.content.clone()
                },
                "type": n.node_type,
                "retention": n.retention_strength,
                "tags": n.tags,
                "x": (x * 100.0).round() / 100.0,
                "y": (y * 100.0).round() / 100.0,
                "isCenter": n.id == center_id,
                // v2.0.5 Active Forgetting — dashboard uses these to dim suppressed nodes
                "suppression_count": n.suppression_count,
                "suppressed_at": n.suppressed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    let edges_json: Vec<serde_json::Value> = edges
        .iter()
        .map(|e| {
            serde_json::json!({
                "source": e.source_id,
                "target": e.target_id,
                "weight": e.strength,
                "type": e.link_type,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "nodes": nodes_json,
        "edges": edges_json,
        "center_id": center_id,
        "depth": depth,
        "nodeCount": nodes.len(),
        "edgeCount": edges.len(),
    }))
}
