//! Thin CLI for the standalone build: `strata-verify <dir>`. The vestige
//! binary exposes the same report as `vestige strata-verify <dir>`.
//!
//! The check is strictly read-only: it never creates `strata.key`, a
//! segment, a lock, or any other file in the directory it is given.
//!
//! The key fingerprint is the lowercase blake3 hex of the 32-byte ed25519
//! verifying key. `--expect-key` requires that fingerprint.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut expect_key: Option<String> = None;
    let mut dir: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            eprintln!(
                "usage: strata-verify [--expect-key <fingerprint>] <store-dir>\n\
                 fingerprint: lowercase blake3 hex of the 32-byte ed25519 verifying key\n\
                 a migration receipt is trusted only when it matches receipt-signing.key"
            );
            return;
        }
        if arg == "--expect-key" {
            let Some(value) = args.next() else {
                eprintln!("--expect-key needs a fingerprint");
                std::process::exit(2);
            };
            expect_key = Some(value);
            continue;
        }
        if dir.is_some() {
            eprintln!("usage: strata-verify [--expect-key <fingerprint>] <store-dir>");
            std::process::exit(2);
        }
        dir = Some(PathBuf::from(arg));
    }
    let Some(dir) = dir else {
        eprintln!("usage: strata-verify [--expect-key <fingerprint>] <store-dir>");
        std::process::exit(2);
    };
    let report = strata_verify::verify_path(&dir);
    println!("{}", report.json);
    if !report.key_fingerprint.is_empty() {
        println!("key fingerprint: {}", report.key_fingerprint);
    }
    let mut failed = !report.ok;
    if let Some(expected) = expect_key.as_deref() {
        let actual = report.key_fingerprint.to_ascii_lowercase();
        let expected = expected.trim().to_ascii_lowercase();
        if actual != expected {
            eprintln!(
                "signing key fingerprint {actual} does not match --expect-key {expected}"
            );
            failed = true;
        }
    }
    if !failed {
        println!("OK");
        return;
    }
    println!("FAILED");
    for failure in &report.failures {
        eprintln!("  {failure}");
    }
    std::process::exit(1);
}
