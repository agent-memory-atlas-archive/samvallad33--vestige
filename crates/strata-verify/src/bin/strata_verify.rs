//! Thin CLI for the standalone build: `strata-verify <dir>`. The vestige
//! binary exposes the same report as `vestige strata-verify <dir>`.
//!
//! The check is strictly read-only: it never creates `strata.key`, a
//! segment, a lock, or any other file in the directory it is given.

use std::path::PathBuf;

fn main() {
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: strata-verify <store-dir>");
        std::process::exit(2);
    };
    let report = strata_verify::verify_path(&dir);
    println!("{}", report.json);
    if report.ok {
        println!("OK");
        return;
    }
    println!("FAILED");
    for failure in &report.failures {
        eprintln!("  {failure}");
    }
    std::process::exit(1);
}
