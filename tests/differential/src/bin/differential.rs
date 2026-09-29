//! Manual runner for the differential harness:
//!
//! ```text
//! cargo run -p vestige-differential-tests --bin differential -- --seed 42 [--out DIR]
//! ```
//!
//! Generates a seeded script (or loads one from `--script FILE`), replays it
//! against BOTH engines, writes each engine's canonical snapshot + the
//! materialized strata store, and prints the parity verdict.

use std::path::PathBuf;

use vestige_differential_tests::{Script, gen_script, run_sqlite, run_strata};

fn main() {
    let mut seed = 42u64;
    let mut out = std::env::temp_dir().join("vestige-differential");
    let mut script_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|v| v.parse().ok()).unwrap_or(42),
            "--out" => {
                if let Some(v) = args.next() {
                    out = PathBuf::from(v);
                }
            }
            "--script" => script_path = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown flag {other}");
                std::process::exit(2);
            }
        }
    }

    let script = match &script_path {
        Some(path) => {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("reading {}: {e}", path.display());
                std::process::exit(2);
            });
            Script::from_json(&text).unwrap_or_else(|e| {
                eprintln!("parsing {}: {e}", path.display());
                std::process::exit(2);
            })
        }
        None => gen_script(seed, 20),
    };

    std::fs::create_dir_all(&out).expect("create out dir");
    let sqlite_dir = out.join("sqlite");
    let strata_dir = out.join("strata");
    std::fs::create_dir_all(&sqlite_dir).unwrap();
    std::fs::create_dir_all(&strata_dir).unwrap();

    println!("script: {} ops (seed {})", script.ops.len(), script.seed);
    std::fs::write(
        out.join("script.json"),
        serde_json::to_string_pretty(&script).unwrap(),
    )
    .unwrap();

    let sqlite = run_sqlite(&script, &sqlite_dir);
    std::fs::write(out.join("snapshot-sqlite.txt"), sqlite.canonical_bytes()).unwrap();
    let (strata, report) = run_strata(&script, &strata_dir);
    std::fs::write(out.join("snapshot-strata.txt"), strata.canonical_bytes()).unwrap();

    sqlite.assert_parity(&strata);
    println!("parity: OK (counts, digests, edges, suppressed, review/lapse trajectories)");
    println!(
        "strata verify: {} (log_tail {}, chain {}, roots {}, verdicts {}, gaps {:?}, {}ms)",
        report.ok(),
        report.log_tail_ok,
        report.checkpoint_chain_ok,
        report.state_root_matches,
        report.gate_verdicts_rederived,
        report.gaps,
        report.duration_ms
    );
    println!("artifacts: {}", out.display());
    if !report.ok() {
        for failure in &report.failures {
            eprintln!("  {failure}");
        }
        std::process::exit(1);
    }
}
