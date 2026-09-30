//! Real-stdio proof for `ghostlink` on a Strata log.
//!
//! Spawns the shipped `vestige-mcp` binary on a fresh data directory and
//! checks, over line-framed JSON-RPC: the advertised surface (ghostlink in,
//! graph out, 16 tools), both propose lenses with their proofs, weave writing
//! through the gate with a receipt per write and removing the pair, inspect,
//! harden seeding once and then reporting every law as already present, a
//! laws file taking precedence, a malformed laws file naming itself, and the
//! hidden graph alias still answering.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RPC_TIMEOUT: Duration = Duration::from_secs(120);

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stderr: Receiver<String>,
    next_id: u64,
}

impl Server {
    /// A server on `data_dir` whose HOME is `home`, so a developer's own
    /// `~/.vestige/ghostlink-laws.json` can never leak into a test.
    fn spawn(data_dir: &Path, home: &Path) -> Self {
        let mut child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_vestige-mcp")))
            .env("VESTIGE_DATA_DIR", data_dir)
            .env("HOME", home)
            .env("VESTIGE_DASHBOARD_ENABLED", "false")
            .env("VESTIGE_HTTP_ENABLED", "0")
            .env("VESTIGE_AUTOPILOT_ENABLED", "0")
            .env("VESTIGE_TRACE", "0")
            .env("VESTIGE_BACKFILL_AUTOFIRE", "0")
            .env("VESTIGE_FAILURE_FEEDBACK", "0")
            .env_remove("RUST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn vestige-mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let raw_stdout = child.stdout.take().expect("child stdout");
        let raw_stderr = child.stderr.take().expect("child stderr");
        let (stdout_tx, stdout) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(raw_stdout).lines().map_while(Result::ok) {
                if stdout_tx.send(line).is_err() {
                    return;
                }
            }
        });
        let (stderr_tx, stderr) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(raw_stderr).lines().map_while(Result::ok) {
                if stderr_tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut server = Self {
            child,
            stdin: Some(stdin),
            stdout,
            stderr,
            next_id: 0,
        };
        server.result(
            "initialize",
            Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "ghostlink-stdio", "version": "1" },
            })),
        );
        server.write_line(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
        );
        server
    }

    fn stderr_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        while let Ok(line) = self.stderr.try_recv() {
            lines.push(line);
        }
        lines
    }

    fn write_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush())
            .unwrap_or_else(|error| panic!("writing to vestige-mcp failed: {error}"));
    }

    fn result(&mut self, method: &str, params: Option<Value>) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.write_line(&message.to_string());
        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = self.stdout.recv_timeout(remaining).unwrap_or_else(|_| {
                panic!("no response to {method}. stderr: {:?}", self.stderr_lines())
            });
            let value: Value = serde_json::from_str(&line)
                .unwrap_or_else(|error| panic!("stdout was not JSON ({error}): {line}"));
            if value.get("id").is_none() && value.get("method").is_some() {
                continue;
            }
            assert_eq!(value["id"], json!(id), "unexpected response: {value}");
            assert!(value.get("error").is_none(), "{method} failed: {value}");
            return value["result"].clone();
        }
    }

    /// A tool's payload; a tool-level failure comes back as `{isError, text}`.
    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.result(
            "tools/call",
            Some(json!({ "name": name, "arguments": arguments })),
        );
        if result["isError"] == json!(true) {
            return json!({ "isError": true, "text": result["content"][0]["text"] });
        }
        result
            .get("structuredContent")
            .cloned()
            .or_else(|| {
                result["content"][0]["text"]
                    .as_str()
                    .and_then(|text| serde_json::from_str(text).ok())
            })
            .unwrap_or_else(|| panic!("tool {name} returned no JSON payload: {result}"))
    }

    fn call_tool_ok(&mut self, name: &str, arguments: Value) -> Value {
        let value = self.call_tool(name, arguments);
        assert!(
            value.get("isError").is_none() && value.get("error").is_none(),
            "tool {name} failed: {value}"
        );
        value
    }

    fn shutdown(mut self) {
        self.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.child.try_wait().expect("poll vestige-mcp").is_none() {
            assert!(
                Instant::now() < deadline,
                "vestige-mcp did not exit on stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn data_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn spawn(dir: &Path, home: &Path) -> Server {
    Server::spawn(dir, home)
}

fn save(server: &mut Server, content: &str, node_type: &str) -> String {
    let saved = server.call_tool_ok(
        "smart_ingest",
        json!({ "content": content, "node_type": node_type, "tags": ["ghostlink-test"] }),
    );
    saved["nodeId"]
        .as_str()
        .unwrap_or_else(|| panic!("smart_ingest returned no nodeId: {saved}"))
        .to_string()
}

fn pairs(proposal: &Value) -> Vec<(String, String)> {
    proposal["candidates"]
        .as_array()
        .unwrap_or_else(|| panic!("no candidates array: {proposal}"))
        .iter()
        .map(|c| {
            (
                c["firstId"].as_str().unwrap().to_string(),
                c["secondId"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn ghostlink_is_the_advertised_composition_tool() {
    let dir = data_dir();
    let home = data_dir();
    let mut server = spawn(dir.path(), home.path());
    let listed = server.result("tools/list", None);
    let names: Vec<&str> = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 16, "{names:?}");
    assert!(names.contains(&"ghostlink"), "{names:?}");
    assert!(
        !names.contains(&"graph"),
        "graph must be a hidden alias: {names:?}"
    );
    let ghostlink = listed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ghostlink")
        .unwrap();
    assert_eq!(ghostlink["annotations"]["readOnlyHint"], json!(false));
    let modes = &ghostlink["inputSchema"]["properties"]["mode"]["enum"];
    for mode in [
        "propose", "bounty", "weave", "map", "inspect", "explore", "predict", "harden",
    ] {
        assert!(
            modes.as_array().unwrap().iter().any(|m| m == mode),
            "mode {mode} missing: {modes}"
        );
    }
    server.shutdown();
}

#[test]
fn propose_weave_and_inspect_carry_proofs_and_receipts() {
    let dir = data_dir();
    let home = data_dir();
    let mut server = spawn(dir.path(), home.path());
    let ids: Vec<String> = [
        ("Refunds need multi-row transactions.", "decision"),
        ("The March outage started in the retry loop.", "event"),
        ("Kettle descaling is due every six weeks.", "fact"),
        ("Invoices are rendered by the PDF worker.", "fact"),
    ]
    .iter()
    .map(|(content, kind)| save(&mut server, content, kind))
    .collect();

    // A cold log has no typed edges: the bridge lens is empty and says why.
    let bridge = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "propose", "lens": "bridge", "limit": 5 }),
    );
    assert!(pairs(&bridge).is_empty(), "{bridge}");
    assert!(
        bridge["admission"]["emptyBecause"].as_str().is_some(),
        "an empty bridge must explain itself: {bridge}"
    );

    // The divergent lens forces juxtapositions, each with its proof and no
    // claimed score.
    let divergent = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "propose", "lens": "divergent", "limit": 2 }),
    );
    let first = pairs(&divergent);
    assert!(!first.is_empty(), "{divergent}");
    for candidate in divergent["candidates"].as_array().unwrap() {
        assert_eq!(
            candidate["proof"]["noEdgeVerified"],
            json!(true),
            "{candidate}"
        );
        assert_eq!(candidate["proof"]["neverWoven"], json!(true), "{candidate}");
        assert_eq!(candidate["lane"], json!("juxtaposition"), "{candidate}");
        assert!(
            candidate["score"].is_null(),
            "unmeasured pairs claim no score: {candidate}"
        );
        assert!(
            candidate["compositionQuestion"]
                .as_str()
                .is_some_and(|q| q.contains("Force a composition")),
            "{candidate}"
        );
    }
    let members: Vec<&String> = first.iter().flat_map(|(a, b)| [a, b]).collect();
    let mut unique = members.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        members.len(),
        "each memory at most once per page"
    );
    let again = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "propose", "lens": "divergent", "limit": 2 }),
    );
    assert_eq!(pairs(&again), first, "same log head, same page");

    // Weave writes a composition record and two derived_from edges, each
    // with a receipt, and the pair leaves both lenses.
    let (a, b) = first[0].clone();
    let woven = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "weave", "first_id": a, "second_id": b,
                "outcome_type": "helpful", "lens": "divergent" }),
    );
    let receipts = woven["receipts"]
        .as_array()
        .unwrap_or_else(|| panic!("{woven}"));
    assert_eq!(receipts.len(), 3, "{woven}");
    for receipt in receipts {
        assert!(
            receipt["receiptId"]
                .as_str()
                .is_some_and(|r| r.starts_with("eff-")),
            "{receipt}"
        );
    }
    let record = woven["recordId"].as_str().unwrap().to_string();
    let proof = server.call_tool_ok(
        "receipt",
        json!({ "action": "replay", "receipt_id": receipts[0]["receiptId"] }),
    );
    assert_eq!(proof["matched"], json!(true), "{proof}");

    let after = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "propose", "lens": "divergent", "limit": 50 }),
    );
    assert!(
        !pairs(&after)
            .iter()
            .any(|(x, y)| (x == &a && y == &b) || (x == &b && y == &a)),
        "a woven pair must not be proposed again: {after}"
    );
    assert!(
        !pairs(&after)
            .iter()
            .any(|(x, y)| x == &record || y == &record),
        "composition records are bridges, not ideas"
    );

    let recent = server.call_tool_ok("ghostlink", json!({ "mode": "inspect", "view": "recent" }));
    assert_eq!(recent["events"][0]["id"], json!(record), "{recent}");
    let neighbors = server.call_tool_ok(
        "ghostlink",
        json!({ "mode": "inspect", "view": "neighbors", "memory_id": a }),
    );
    assert_eq!(
        neighbors["neighbors"][0]["memoryId"],
        json!(b),
        "{neighbors}"
    );
    assert!(ids.contains(&a) && ids.contains(&b));

    // The hidden graph alias still dispatches, to the same engine.
    let alias = server.call_tool_ok("graph", json!({ "action": "never_composed", "limit": 3 }));
    assert_eq!(alias["lens"], json!("bridge"), "{alias}");
    server.shutdown();
}

#[test]
fn harden_seeds_once_then_reports_every_law_present() {
    let dir = data_dir();
    let home = data_dir();
    let mut server = spawn(dir.path(), home.path());
    let first = server.call_tool_ok("ghostlink", json!({ "mode": "harden" }));
    assert_eq!(first["lawsSource"]["kind"], json!("embedded"), "{first}");
    assert_eq!(first["laws"], json!(6), "{first}");
    assert_eq!(first["seeded"], json!(6), "{first}");
    assert_eq!(first["failed"], json!(0), "{first}");
    for result in first["results"].as_array().unwrap() {
        assert!(
            result["receiptId"]
                .as_str()
                .is_some_and(|r| r.starts_with("eff-")),
            "a seeded law carries its receipt: {result}"
        );
    }
    let second = server.call_tool_ok("ghostlink", json!({ "mode": "harden" }));
    assert_eq!(second["seeded"], json!(0), "{second}");
    assert_eq!(second["already_present"], json!(6), "{second}");
    let laws = server.call_tool_ok("recall", json!({ "handle": "invariant-law" }));
    assert_eq!(
        laws["nodes"].as_array().map(Vec::len),
        Some(6),
        "no duplicate law memories: {laws}"
    );
    server.shutdown();
}

#[test]
fn a_laws_file_wins_and_a_malformed_one_names_itself() {
    let dir = data_dir();
    let home = data_dir();
    std::fs::write(
        dir.path().join("ghostlink-laws.json"),
        json!({ "ghostlink_invariant_laws": [{
            "id": "LAW-TEST", "name": "Test law", "law": "Every retry carries its attempt id.",
            "signals": ["retry"], "severity_if_absent": "High"
        }]})
        .to_string(),
    )
    .unwrap();
    let mut server = spawn(dir.path(), home.path());
    let seeded = server.call_tool_ok("ghostlink", json!({ "mode": "harden" }));
    assert_eq!(seeded["lawsSource"]["kind"], json!("file"), "{seeded}");
    assert_eq!(seeded["laws"], json!(1), "{seeded}");
    assert_eq!(seeded["results"][0]["lawId"], json!("LAW-TEST"), "{seeded}");
    server.shutdown();

    let broken = data_dir();
    std::fs::write(broken.path().join("ghostlink-laws.json"), "{ not json").unwrap();
    let mut server = spawn(broken.path(), home.path());
    let refused = server.call_tool("ghostlink", json!({ "mode": "harden" }));
    let text = refused.to_string();
    assert!(
        text.contains("ghostlink-laws.json") && text.contains("malformed"),
        "a malformed laws file must be named, not skipped: {refused}"
    );
    server.shutdown();
}
