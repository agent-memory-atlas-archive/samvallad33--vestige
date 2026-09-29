//! Thin CLI for the standalone build: `strata-verify <dir>`. The vestige
//! binary exposes the same report as `vestige strata-verify <dir>`; this bin
//! exists so the crate is runnable without the vestige workspace.

use std::path::PathBuf;

fn main() {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: strata-verify <store-dir>");
        std::process::exit(2);
    };
    let report = strata_verify::verify_store(&dir);
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("report serializes")
    );
    if report.ok() {
        println!("OK");
    } else {
        println!("FAILED");
        for failure in &report.failures {
            eprintln!("  {failure}");
        }
        std::process::exit(1);
    }
}
