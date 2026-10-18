//! `gather`: the receipt factory for a GitHub issue hunt.
//!
//! Given an issue URL, a checkout path and an optional version window, gather
//! records every artifact the hunt needs as its own labeled memory — the
//! issue, its comments, every cross-referenced pull request, and the repo's
//! commit window through the ingest_repo machinery — then runs ghostlink and
//! causal_walk over the recorded receipts. Gathering is dynamic (it talks to
//! the `gh` CLI); judging is deterministic (the walk follows recorded edges
//! only, and never reads the fetched prose to pick a cause).
//!
//! Order matters and is the point: commits are recorded BEFORE the issue
//! memory, so the auto-connect `touched` edges point from the earlier commit
//! to the later failure and the backward walk can traverse them. Everything
//! lands in ONE scope, so a walk can never come back empty from a scope
//! mismatch. All writes go through the server's own `Storage` handle: Strata
//! is single-writer and a subprocess store would collide with the running
//! server's lock.
//!
//! If the walk returns nothing, the report says which receipt class was
//! missing (no version window, no linked PRs, no commits in scope) instead of
//! inventing a cause. A hunt that fails this way defines the next thing
//! gather learns to fetch.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use vestige_core::Storage;

use super::repo_ingest;
use crate::cognitive::CognitiveEngine;

pub struct GatherRequest {
    /// GitHub issue URL (…/issues/N or …/pull/N).
    pub url: String,
    /// Optional checkout of the same repository; when present its commit
    /// window is ingested before the failure record is written.
    pub repo_path: Option<PathBuf>,
    /// Worked-in tag for the version window (e.g. `v0.23.3`).
    pub worked_in: Option<String>,
    /// Broke-in tag for the version window.
    pub broke_in: Option<String>,
    /// Scope for every record. Defaults to `<owner>-<repo>-<number>`.
    pub scope: Option<String>,
    /// Maximum comments recorded (default 50).
    pub max_comments: usize,
}

struct IssueRef {
    owner: String,
    name: String,
    number: u64,
    scope: String,
}

fn parse_issue_url(url: &str) -> Result<IssueRef, String> {
    let trimmed = url.trim();
    let trimmed = trimmed
        .split(['#', '?'])
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').collect();
    let (owner, name, num) = match parts.as_slice() {
        [.., o, n, "issues" | "pull" | "pulls", num] => (*o, *n, *num),
        _ => {
            return Err(
                "url must be a GitHub issue or pull request URL like https://github.com/owner/repo/issues/123"
                    .into(),
            );
        }
    };
    let number: u64 = num
        .parse()
        .map_err(|_| format!("issue number {num:?} is not a number"))?;
    Ok(IssueRef {
        owner: owner.to_string(),
        name: name.to_string(),
        number,
        scope: format!("{owner}-{name}-{number}"),
    })
}

fn gh_json(args: &[&str]) -> Result<Value, String> {
    let out = std::process::Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| format!("gh is not runnable: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("gh {} returned invalid JSON: {e}", args.join(" ")))
}

/// The issue's comments, oldest first. `repos/O/R/issues/N` is the issue
/// itself (one object, never a list), so its comments come only from here.
/// GitHub serves at most 100 per page.
fn comments_endpoint(issue: &IssueRef, max_comments: usize) -> String {
    format!(
        "repos/{}/{}/issues/{}/comments?per_page={}",
        issue.owner,
        issue.name,
        issue.number,
        max_comments.clamp(1, 100)
    )
}

/// `(author, body)` for every comment with a non-blank body. Anything other
/// than a list of comments yields none.
fn comment_bodies(comments: &Value) -> Vec<(String, String)> {
    comments
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let who = c["user"]["login"].as_str()?.to_string();
                    let body = c["body"].as_str()?.trim().to_string();
                    (!body.is_empty()).then_some((who, body))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Join each record gather wrote to the scope's memories it shares an exact
/// identity with, through the same pass smart_ingest runs at ingest: hub tags
/// are skipped, and each record writes at most
/// [`crate::auto_connect::MAX_AUTO_EDGES`] edges in one write. Returns
/// `(edges written, candidates the budget left unlinked)`.
///
/// Records are joined one at a time, never every pair of the scope: commits
/// that only share their commit tag stay unjoined, so a commit window of n
/// commits costs nothing here rather than n * (n - 1) / 2 edges on a log that
/// cannot drop them.
fn bridge(
    storage: &Storage,
    scope: &str,
    records: &[(String, String, Vec<String>)],
) -> (usize, usize) {
    let mut edges = 0usize;
    let mut not_linked = 0usize;
    for (id, content, tags) in records {
        match crate::auto_connect::auto_connect_new_memory(storage, id, scope, content, tags) {
            Ok(report) => {
                edges += report.edges;
                not_linked += report.not_linked;
            }
            Err(err) => tracing::warn!("gather: {err}"),
        }
    }
    (edges, not_linked)
}

/// Run one ingest_repo page and return `(summary, pageBackRev)`.
async fn ingest_page(
    storage: &Arc<Storage>,
    repo_path: &Path,
    scope: &str,
    worked_in: &str,
    broke_in: &str,
    page_back: Option<String>,
) -> Result<(Value, Option<String>), String> {
    let request = repo_ingest::Request {
        repo_path: repo_path.to_path_buf(),
        codebase: None,
        scope: Some(scope.to_string()),
        rev: page_back,
        since: None,
        until: None,
        limit: Some(200),
        dry_run: false,
        budget: None,
    };
    let summary = repo_ingest::execute(storage, request).await?;
    // The version window bounds the walk's structural ranking; the tags make
    // both ends recallable by handle.
    let _ = (worked_in, broke_in);
    let back = summary["pageBackWith"]["rev"].as_str().map(String::from);
    Ok((summary, back))
}

/// The full input schema (`memory_status` view='tools', tool='gather'); the
/// `tools/list` wire carries its compact form.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "description": "Record a GitHub issue hunt's evidence as labeled receipts in one scope (the issue, its comments, cross-referenced PRs, and the commit window of a local checkout through ingest_repo paging), then run ghostlink leads and causal_walk over the recorded edges. Causes are hypotheses; missingReceipts names the next receipt class to fetch.",
        "properties": {
            "url": {"type": "string", "description": "GitHub issue or pull request URL (…/issues/N or …/pull/N)"},
            "repoPath": {"type": "string", "description": "local checkout of the same repository; its commit window is ingested before the failure record"},
            "workedIn": {"type": "string", "description": "good version tag for the window"},
            "brokeIn": {"type": "string", "description": "bad version tag for the window"},
            "scope": {"type": "string", "description": "memory namespace (default owner-repo-number)"},
            "maxComments": {"type": "integer", "description": "stop after this many comments (default 50)"}
        },
        "required": ["url"]
    })
}

/// A hunt writes a commit window, its receipts, and a bounded batch of edges
/// per receipt, each holding the store for a gated write. It runs as
/// background store work so dashboard reads and other agents' calls take the
/// store first; what it records, and in what order, is unchanged.
pub async fn execute(
    storage: &Arc<Storage>,
    cognitive: Option<&Arc<Mutex<CognitiveEngine>>>,
    args: Option<Value>,
) -> Result<Value, String> {
    crate::strata_memory::as_background(run(storage, cognitive, args)).await
}

async fn run(
    storage: &Arc<Storage>,
    cognitive: Option<&Arc<Mutex<CognitiveEngine>>>,
    args: Option<Value>,
) -> Result<Value, String> {
    let args = args.ok_or("gather requires arguments: url, repoPath (optional), workedIn/brokeIn (optional), scope (optional)")?;
    let url = args["url"]
        .as_str()
        .ok_or("gather requires `url`: the GitHub issue URL")?
        .to_string();
    let repo_path = args["repoPath"].as_str().map(PathBuf::from);
    let worked_in = args["workedIn"].as_str().map(String::from);
    let broke_in = args["brokeIn"].as_str().map(String::from);
    let scope = args["scope"].as_str().map(String::from);
    let max_comments = args["maxComments"].as_u64().unwrap_or(50) as usize;

    let issue = parse_issue_url(&url)?;
    let scope = scope.unwrap_or_else(|| issue.scope.clone());

    // ---- Fetch (dynamic, through gh) ------------------------------------
    let issue_json = gh_json(&[
        "api",
        &format!(
            "repos/{}/{}/issues/{}",
            issue.owner, issue.name, issue.number
        ),
    ])?;
    let title = issue_json["title"]
        .as_str()
        .unwrap_or("untitled")
        .to_string();
    let body = issue_json["body"].as_str().unwrap_or("").to_string();
    let state = issue_json["state"].as_str().unwrap_or("open").to_string();
    let mut labels: Vec<String> = Vec::new();
    if let Some(arr) = issue_json["labels"].as_array() {
        for l in arr {
            if let Some(n) = l["name"].as_str() {
                labels.push(n.to_string());
            }
        }
    }

    let comment_bodies = if max_comments == 0 {
        Vec::new()
    } else {
        let comments = gh_json(&["api", &comments_endpoint(&issue, max_comments)])?;
        let mut bodies = comment_bodies(&comments);
        bodies.truncate(max_comments);
        bodies
    };

    let mut linked_prs: Vec<(u64, String)> = Vec::new();
    if let Ok(timeline) = gh_json(&[
        "api",
        &format!(
            "repos/{}/{}/issues/{}/timeline?per_page=100",
            issue.owner, issue.name, issue.number
        ),
    ]) && let Some(events) = timeline.as_array()
    {
        for ev in events {
            let cross = ev["event"].as_str() == Some("cross-referenced");
            let is_pr = ev["source"]["issue"]["pull_request"].is_object();
            if cross
                && is_pr
                && let (Some(n), Some(t)) = (
                    ev["source"]["issue"]["number"].as_u64(),
                    ev["source"]["issue"]["title"].as_str(),
                )
            {
                linked_prs.push((n, t.to_string()));
            }
        }
    }

    // ---- Record (through the server's own storage handle) ---------------
    // Commits FIRST so auto-connect edges point failure-ward.
    let mut commits_created = 0u64;
    let mut commits_covered = 0u64;
    let mut commit_pages = 0;
    if let Some(repo_path) = &repo_path {
        let mut page_back: Option<String> = None;
        loop {
            let (summary, back) = ingest_page(
                storage,
                repo_path,
                &scope,
                worked_in.as_deref().unwrap_or(""),
                broke_in.as_deref().unwrap_or(""),
                page_back.clone(),
            )
            .await?;
            commit_pages += 1;
            // True coverage: new writes AND commits the scope already held
            // (a re-run counts the window it verified, not just new writes).
            commits_created += summary["commits"]["created"].as_u64().unwrap_or(0);
            commits_covered += summary["commits"]["created"].as_u64().unwrap_or(0)
                + summary["commits"]["alreadyIngested"].as_u64().unwrap_or(0);
            match back {
                Some(rev) if commit_pages < 10 => page_back = Some(rev),
                _ => break,
            }
        }
    }

    let write = |content: String, tags: Vec<String>, source: String| {
        let input = vestige_core::IngestInput {
            content,
            node_type: "event".to_string(),
            source: Some(source),
            sentiment_score: 0.0,
            sentiment_magnitude: 0.0,
            tags,
            valid_from: None,
            valid_until: None,
            validity_inferred: false,
            source_envelope: None,
        };
        // The scoped write path, the same one smart_ingest uses: receipts
        // MUST land in the gather scope or the walk (scoped) never sees them.
        storage.ingest_in_scope_with_secret_policy(
            input,
            &scope,
            vestige_core::SecretPolicy::Reject,
        )
    };

    let mut bridge_records: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut linked_pr_ids: Vec<String> = Vec::new();
    for (n, t) in &linked_prs {
        let node = write(
            format!(
                "Linked pull request #{n} on issue {}/{}/#{}: {t}",
                issue.owner, issue.name, issue.number
            ),
            vec![issue.name.clone(), "gather".into(), "linked-pr".into()],
            format!("github:{}/{}/#{}", issue.owner, issue.name, n),
        )
        .map_err(|e| format!("linked PR #{n} was not admitted: {e}"))?;
        bridge_records.push((node.id.clone(), node.content.clone(), node.tags.clone()));
        linked_pr_ids.push(node.id);
    }

    let failure_node = write(
        format!(
            "Issue {}/{}/#{} [{}]: {title}. {}",
            issue.owner,
            issue.name,
            issue.number,
            state,
            body.trim()
        ),
        {
            let mut t = vec![issue.name.clone(), "failure".into(), "gather".into()];
            t.extend(labels);
            t
        },
        format!("github:{}/{}/#{}", issue.owner, issue.name, issue.number),
    )
    .map_err(|e| format!("the failure record was not admitted: {e}"))?;
    let failure_id = failure_node.id.clone();
    bridge_records.push((
        failure_node.id.clone(),
        failure_node.content.clone(),
        failure_node.tags.clone(),
    ));

    let mut comment_ids: Vec<String> = Vec::new();
    for (who, body) in &comment_bodies {
        let node = write(
            format!(
                "Comment on issue {}/{}/#{} by {}: {}",
                issue.owner, issue.name, issue.number, who, body
            ),
            vec![issue.name.clone(), "gather".into(), "comment".into()],
            format!("github:{}/{}/#{}", issue.owner, issue.name, issue.number),
        )
        .map_err(|e| format!("a comment record was not admitted: {e}"))?;
        bridge_records.push((node.id.clone(), node.content.clone(), node.tags.clone()));
        comment_ids.push(node.id);
    }

    // ---- Bridge (deterministic, exact entities only) ----------------------
    // The commit records were written by ingest_repo and the receipts by the
    // scoped write above, without auto-connect. Join each receipt here on the
    // exact identities it shares with the scope (a tag, a file path, a commit
    // sha, an issue or a URL), so the walk can follow failure-to-commit
    // `touched` edges. No similarity, no guesses, and a bounded number of
    // edges per receipt.
    let (bridge_edges, bridge_not_linked) = bridge(storage.as_ref(), &scope, &bridge_records);

    // ---- Judge (deterministic, recorded edges only) ----------------------
    // GhostLink proposes never-composed pairs among the new receipts (leads
    // only), then the walk ranks causes over recorded edges.
    let mut ghostlink_note = "skipped: ghostlink proposal over this scope returned no pairs".into();
    let mut ghostlink_pairs = 0;
    if let Some(cognitive) = cognitive
        && let Ok(pairs) = super::ghostlink::execute(
            storage,
            cognitive,
            Some(json!({"mode": "propose", "lens": "bridge", "scope": scope})),
        )
        .await
        && let Some(list) = pairs["candidates"].as_array()
    {
        ghostlink_pairs = list.len();
        if ghostlink_pairs > 0 {
            ghostlink_note = format!("{ghostlink_pairs} never-composed pair(s) proposed as leads");
        }
    }

    let walk = super::causal_walk::execute(
        storage,
        Some(json!({
            "start_points": [{"kind": "logged_write", "node_id": failure_id}],
            "scope": scope
        })),
    )
    .await?;
    let walk_causes = walk["causes"].as_array().map(|a| a.len()).unwrap_or(0);
    let empty_because = walk["emptyBecause"].as_str().map(String::from);

    // The honesty clause: name what was missing instead of inventing a cause.
    let missing_receipts: Vec<String> = {
        let mut m = Vec::new();
        if repo_path.is_none() {
            m.push(
                "repoPath: no commits were ingested, so the walk cannot reach any commit record"
                    .into(),
            );
        }
        if worked_in.is_none() || broke_in.is_none() {
            m.push(
                "workedIn/brokeIn: no version window, so structural ranking could not fire".into(),
            );
        }
        if !storage
            .node_is_in_scope(&failure_id, &scope)
            .unwrap_or(false)
        {
            m.push("scope mismatch: the failure record is not in the gather scope".into());
        }
        if walk_causes == 0 && commits_covered == 0 && repo_path.is_some() {
            m.push("commits ingested but the walk still returned nothing: page further back or check the scope".into());
        }
        m
    };

    Ok(json!({
        "tool": "gather",
        "issue": format!("{}/{}/#{}", issue.owner, issue.name, issue.number),
        "scope": scope,
        "recorded": {
            "failureId": failure_id,
            "failureScope": storage
                .node_is_in_scope(&failure_id, &scope)
                .unwrap_or(false)
                .then(|| scope.clone()),
            "linkedPrIds": linked_pr_ids,
            "commentIds": comment_ids,
            "commitPages": commit_pages,
            "commitsCreated": commits_created,
            "commitsCovered": commits_covered,
            "bridgeEdges": bridge_edges,
            "bridgeNotLinked": bridge_not_linked,
        },
        "leads": {
            "ghostlinkPairs": ghostlink_pairs,
            "note": ghostlink_note,
        },
        "walk": {
            "causes": walk_causes,
            "top": walk["causes"].as_array().and_then(|a| a.first().cloned()),
            "emptyBecause": empty_because,
        },
        "missingReceipts": missing_receipts,
        "note": "gather records receipts and runs the deterministic walk; causes are hypotheses, not proven causes. A missingReceipts entry names what to fetch next time."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped_write(
        storage: &Storage,
        scope: &str,
        content: &str,
        tags: &[&str],
    ) -> (String, String, Vec<String>) {
        let input = vestige_core::IngestInput {
            content: content.into(),
            node_type: "event".to_string(),
            source: None,
            sentiment_score: 0.0,
            sentiment_magnitude: 0.0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            valid_from: None,
            valid_until: None,
            validity_inferred: false,
            source_envelope: None,
        };
        let node = storage
            .ingest_in_scope_with_secret_policy(input, scope, vestige_core::SecretPolicy::Reject)
            .expect("scoped write");
        (node.id, node.content, node.tags)
    }

    #[test]
    fn issue_and_pull_request_urls_parse() {
        for url in [
            "https://github.com/acme/app/issues/12",
            "https://github.com/acme/app/pull/12",
            "https://github.com/acme/app/pulls/12/",
            "https://github.com/acme/app/issues/12#issuecomment-99",
            "https://github.com/acme/app/pull/12?w=1",
        ] {
            let issue = parse_issue_url(url).unwrap_or_else(|e| panic!("{url}: {e}"));
            assert_eq!(
                (issue.owner.as_str(), issue.name.as_str(), issue.number),
                ("acme", "app", 12),
                "{url}"
            );
            assert_eq!(issue.scope, "acme-app-12");
        }
        assert!(parse_issue_url("https://github.com/acme/app/commit/12").is_err());
        assert!(parse_issue_url("https://github.com/acme/app/pull/12/files").is_err());
    }

    /// The issue endpoint returns the issue, so comments must come from the
    /// comments endpoint, and an issue object must never read as comments.
    #[test]
    fn comments_come_from_the_comments_endpoint() {
        let issue = parse_issue_url("https://github.com/acme/app/issues/7").unwrap();
        assert_eq!(
            comments_endpoint(&issue, 50),
            "repos/acme/app/issues/7/comments?per_page=50"
        );
        assert_eq!(
            comments_endpoint(&issue, 500),
            "repos/acme/app/issues/7/comments?per_page=100"
        );
        let issue_object = json!({"title": "t", "body": "b", "user": {"login": "x"}});
        assert!(comment_bodies(&issue_object).is_empty());
        let list = json!([
            {"user": {"login": "ann"}, "body": " first "},
            {"user": {"login": "bo"}, "body": "   "},
            {"user": {"login": "cy"}, "body": "third"}
        ]);
        assert_eq!(
            comment_bodies(&list),
            vec![
                ("ann".to_string(), "first".to_string()),
                ("cy".to_string(), "third".to_string())
            ]
        );
    }

    /// 40 commit records share their commit tag. The old bridge joined every
    /// pair of them (780 edges, each its own log write). The bridge joins
    /// only gather's own records, on identities that are not hubs: the
    /// failure reaches the four commits (0, 13, 26, 39) that touched the
    /// file it names.
    #[test]
    fn the_bridge_joins_receipts_not_every_commit_pair() {
        let dir = tempfile::TempDir::new().expect("temp store dir");
        let store = crate::strata_memory::open(dir.path()).expect("test store");
        let storage: &Storage = &*store;
        let scope = "acme-app-1";
        for i in 0..40 {
            let path = if i % 13 == 0 {
                "src/parser.rs".to_string()
            } else {
                format!("src/mod_{i}.rs")
            };
            scoped_write(
                storage,
                scope,
                &format!("commit {i}: change {path}"),
                &["git-commit", "app"],
            );
        }
        let failure = scoped_write(
            storage,
            scope,
            "Issue acme/app#1 [open]: crash in src/parser.rs",
            &["app", "failure", "gather"],
        );
        let (edges, not_linked) = bridge(storage, scope, std::slice::from_ref(&failure));
        assert_eq!((edges, not_linked), (4, 0));
        let all = storage.get_all_connections().expect("edges");
        assert_eq!(all.len(), 4, "only the failure's edges: {all:?}");
        assert!(
            all.iter()
                .all(|e| e.source_id == failure.0 || e.target_id == failure.0)
        );
    }

    /// The scope contract, as a test: every receipt gather writes (failure,
    /// comment) must land in the gather scope. If the writer ever drifts
    /// back to the default scope, the scoped walk goes blind and this fails.
    #[test]
    fn receipts_land_in_the_gather_scope_not_user() {
        let dir = tempfile::TempDir::new().expect("temp store dir");
        let storage = crate::strata_memory::open(dir.path()).expect("test store");
        let scope = "gather-scope-test";
        let input = vestige_core::IngestInput {
            content: "Issue acme/app#1 [open]: it broke. ".into(),
            node_type: "event".to_string(),
            source: Some("github:acme/app#1".into()),
            sentiment_score: 0.0,
            sentiment_magnitude: 0.0,
            tags: vec!["failure".into(), "gather".into()],
            valid_from: None,
            valid_until: None,
            validity_inferred: false,
            source_envelope: None,
        };
        let node = storage
            .ingest_in_scope_with_secret_policy(input, scope, vestige_core::SecretPolicy::Reject)
            .expect("scoped write");
        let in_gather_scope = storage.node_is_in_scope(&node.id, scope).unwrap_or(false);
        let in_user_scope = storage
            .node_is_in_scope(&node.id, vestige_core::DEFAULT_MEMORY_SCOPE)
            .unwrap_or(false);
        assert!(in_gather_scope, "receipt must be in the gather scope");
        assert!(
            !in_user_scope,
            "receipt must NOT fall back to the user scope"
        );
    }
}
