//! Compaction survival. Compaction rewrites the conversation into a summary,
//! and whatever the summary drops is gone for the rest of the session. Reflex
//! makes that checkable and repairs it by handle:
//!
//! - PreCompact: the memories the main agent holds are saved as a checkpoint
//!   memory with a receipt, listing their exact ids.
//! - PostCompact: the summary is read for those exact ids. What it kept and
//!   what it dropped are saved as an audit memory derived from the checkpoint.
//! - After compaction: the dropped memories come back by handle, decisions
//!   first, each as its current version, at the first firing that can add
//!   context once SessionStart has marked the compaction. Claude Code sends
//!   SessionStart before PostCompact, so waiting lets the audit's exact
//!   dropped set be used. With no audit, every checkpointed memory comes back.
//!
//! The ids are read as exact tokens; a paraphrased or cut id counts as dropped.

use super::*;

/// Memories re-introduced after one compaction, at most.
const BACK_MAX: usize = 12;

/// What the main agent held when compaction started.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct Checkpoint {
    pub ids: Vec<String>,
    pub turn: u32,
    pub node: Option<String>,
    /// Set by the PostCompact audit: the ids the summary did not keep.
    pub dropped: Option<Vec<String>>,
    pub audit: Option<String>,
    /// Set by SessionStart after compaction: give the memories back at the
    /// next firing that can add context.
    #[serde(default)]
    pub ready: bool,
}

/// Every exact memory id written in a text, in order, once each.
pub(super) fn ids_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if is_memory_id(token) && !out.iter().any(|o| o == token) {
            out.push(token.to_string());
        }
    }
    out
}

fn checkpoint_item(inp: &Input, ids: &[String]) -> Value {
    json!({
        "node_type": "event",
        "tags": ["reflex-compaction", "compaction-checkpoint"],
        "source": format!("reflex {VERSION}, PreCompact"),
        "content": format!(
            "COMPACTION CHECKPOINT, recorded by Reflex. When: {} UTC. Session {}, trigger {}. The agent held {} \
             memories when compaction started, by exact id: {}. Manifest {}.",
            Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            or_unknown(&inp.session.chars().take(8).collect::<String>()),
            or_unknown(&inp.trigger),
            ids.len(),
            ids.join(", "),
            manifest_digest(ids),
        ),
    })
}

fn audit_item(inp: &Input, checkpoint: &str, kept: &[String], dropped: &[String]) -> Value {
    json!({
        "node_type": "event",
        "tags": ["reflex-compaction", "compaction-audit"],
        "source": format!("reflex {VERSION}, PostCompact"),
        "links": [{"kind": "derived_from", "to": checkpoint}],
        "content": format!(
            "COMPACTION AUDIT, recorded by Reflex. When: {} UTC. Session {}. Of the memories in checkpoint {checkpoint}, \
             the summary kept {} by exact id: {}. It dropped {}: {}. The dropped ones come back by handle when the \
             session starts again.",
            Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            or_unknown(&inp.session.chars().take(8).collect::<String>()),
            kept.len(),
            if kept.is_empty() { "none".to_string() } else { kept.join(", ") },
            dropped.len(),
            if dropped.is_empty() { "none".to_string() } else { dropped.join(", ") },
        ),
    })
}

/// A memory's current version: follow recorded retirements to the successor.
fn current(id: &str, retired: &[stale::Retirement]) -> Option<String> {
    let mut id = id.to_string();
    for _ in 0..8 {
        match retired.iter().rev().find(|r| r.old == id) {
            // A retirement with no successor means the memory is gone.
            Some(r) => id = r.new.clone()?,
            None => return Some(id),
        }
    }
    Some(id)
}

/// The type and opening of one memory, read by its exact id.
async fn describe(ctx: &Ctx<'_>, id: &str) -> Option<(String, String)> {
    let out = super::super::recall::execute(
        ctx.storage,
        ctx.cognitive,
        ctx.output_config,
        Some(json!({"handle": id, "detail_level": "summary", "limit": 1})),
    )
    .await
    .ok()?;
    let node = out.get("nodes").and_then(Value::as_array)?.first()?.clone();
    let kind = node
        .get("type")
        .or_else(|| node.get("nodeType"))
        .and_then(Value::as_str)
        .unwrap_or("memory")
        .to_string();
    let content = node.get("content").and_then(Value::as_str).unwrap_or("");
    let opening: String = content.chars().take(140).collect();
    Some((kind, opening))
}

fn rank(kind: &str) -> u8 {
    match kind {
        "decision" => 0,
        "correction" => 1,
        "fact" => 2,
        _ => 3,
    }
}

/// The lines that give back what compaction dropped, and how many memories.
async fn restore_lines(
    ctx: &Ctx<'_>,
    cp: &Checkpoint,
    retired: &[stale::Retirement],
) -> (Vec<String>, usize) {
    let wanted = cp.dropped.clone().unwrap_or_else(|| cp.ids.clone());
    let mut back: Vec<(u8, String)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for id in wanted.iter().rev() {
        if back.len() >= BACK_MAX {
            break;
        }
        let Some(cur) = current(id, retired) else {
            continue;
        };
        if seen.contains(&cur) {
            continue;
        }
        seen.push(cur.clone());
        if let Some((kind, opening)) = describe(ctx, &cur).await {
            let note = if cur != *id {
                format!(" (current version of {id})")
            } else {
                String::new()
            };
            back.push((rank(&kind), format!("- {cur} [{kind}]{note}: {opening}")));
        }
    }
    back.sort_by_key(|(r, _)| *r);
    let mut lines = Vec::new();
    if !back.is_empty() {
        lines.push(format!(
            "Reflex: compaction {} {} memor{} you held. Here they are by handle; recall(handle=...) reads one whole{}:",
            if cp.dropped.is_some() { "dropped" } else { "may have dropped" },
            back.len(),
            if back.len() == 1 { "y" } else { "ies" },
            cp.audit
                .as_deref()
                .map(|a| format!(", audit {a}"))
                .unwrap_or_default(),
        ));
        lines.extend(back.into_iter().map(|(_, l)| l));
    }
    let told = seen.len();
    (lines, told)
}

/// Compaction survival for one firing. Returns the ledger entries and the
/// lines to tell the agent.
pub(super) async fn on_event(
    global: &Arc<std::sync::Mutex<Global>>,
    ctx: &Ctx<'_>,
    inp: &Input,
) -> (Vec<Value>, Vec<String>) {
    let mut wrote = Vec::new();
    let mut tell = Vec::new();
    if inp.session.is_empty() {
        return (wrote, tell);
    }
    match inp.event.as_str() {
        "PreCompact" => {
            let (ids, turn) = {
                let Ok(mut g) = global.lock() else {
                    return (wrote, tell);
                };
                let s = g.session(&inp.session);
                let ids = s.contexts.get("").map(Context::ids).unwrap_or_default();
                (ids, s.turn)
            };
            if ids.is_empty() {
                return (wrote, tell);
            }
            let node = ingest(ctx, checkpoint_item(inp, &ids)).await.ok();
            if let Some(n) = &node {
                wrote.push(json!({"kind": "compaction-checkpoint", "memory": n, "held": ids.len(), "new": true}));
            }
            if let Ok(mut g) = global.lock() {
                g.session(&inp.session).checkpoint = Some(Checkpoint {
                    ids,
                    turn,
                    node,
                    ..Checkpoint::default()
                });
                g.dirty = true;
            }
        }
        "PostCompact" => {
            let cp = {
                let Ok(mut g) = global.lock() else {
                    return (wrote, tell);
                };
                g.session(&inp.session).checkpoint.clone()
            };
            let Some(cp) = cp else {
                return (wrote, tell);
            };
            if inp.summary.is_empty() {
                return (wrote, tell);
            }
            let found = ids_in(&inp.summary);
            let (kept, dropped): (Vec<String>, Vec<String>) =
                cp.ids.iter().cloned().partition(|id| found.contains(id));
            let audit = match &cp.node {
                Some(n) => ingest(ctx, audit_item(inp, n, &kept, &dropped)).await.ok(),
                None => None,
            };
            if let Some(a) = &audit {
                wrote.push(json!({"kind": "compaction-audit", "memory": a, "kept": kept.len(), "dropped": dropped.len(), "new": true}));
            }
            if let Ok(mut g) = global.lock() {
                if let Some(c) = g.session(&inp.session).checkpoint.as_mut() {
                    c.dropped = Some(dropped);
                    c.audit = audit;
                }
                g.dirty = true;
            }
        }
        "SessionStart" if inp.source == "compact" => {
            if let Ok(mut g) = global.lock()
                && let Some(c) = g.session(&inp.session).checkpoint.as_mut()
            {
                c.ready = true;
                g.dirty = true;
            }
        }
        event if event_budget(event) > 0 => {
            let (cp, retired) = {
                let Ok(mut g) = global.lock() else {
                    return (wrote, tell);
                };
                let retired = g.p.retired.clone();
                let s = g.session(&inp.session);
                if !s.checkpoint.as_ref().is_some_and(|c| c.ready) {
                    return (wrote, tell);
                }
                (s.checkpoint.take(), retired)
            };
            let Some(cp) = cp else {
                return (wrote, tell);
            };
            let (lines, told) = restore_lines(ctx, &cp, &retired).await;
            tell.extend(lines);
            wrote.push(json!({"kind": "compaction-restore", "told": told, "audit": cp.audit, "new": false}));
        }
        _ => {}
    }
    (wrote, tell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_ids_are_read_from_a_summary_and_a_cut_id_is_not_one() {
        let text = "Kept mem-00000000000000a1 and (mem-00000000000000b2). Cut: mem-00000000000000c. \
                    Again mem-00000000000000a1, uuid 68540393-203d-428b-9a1a-854da288e655.";
        assert_eq!(
            ids_in(text),
            vec![
                "mem-00000000000000a1".to_string(),
                "mem-00000000000000b2".to_string(),
                "68540393-203d-428b-9a1a-854da288e655".to_string()
            ]
        );
    }

    #[test]
    fn the_current_version_follows_recorded_successors() {
        let r = |old: &str, new: Option<&str>| stale::Retirement {
            old: old.into(),
            new: new.map(str::to_string),
            ..stale::Retirement::default()
        };
        let retired = vec![r("a", Some("b")), r("b", Some("c")), r("x", None)];
        assert_eq!(current("a", &retired).as_deref(), Some("c"));
        assert_eq!(current("z", &retired).as_deref(), Some("z"));
        assert_eq!(
            current("x", &retired),
            None,
            "a suppressed memory does not come back"
        );
    }
}
