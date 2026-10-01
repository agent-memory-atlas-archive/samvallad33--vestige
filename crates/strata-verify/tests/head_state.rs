//! `strata-verify` must agree with the log's own open: a directory the log
//! refuses to open because of `head.state` is not reported as OK.

use std::fs;
use std::path::{Path, PathBuf};

use strata::StrataLog;

fn segments(dir: &Path) -> Vec<PathBuf> {
    let mut segs: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "seg"))
        .collect();
    segs.sort();
    segs
}

/// Three acked frames in the active segment.
fn three_frame_log(dir: &Path) {
    let log = StrataLog::open(dir).unwrap();
    for i in 0..3u8 {
        log.append(1, format!("frame-{i}").as_bytes()).unwrap();
    }
}

fn report_text(dir: &Path) -> String {
    let report = strata_verify::verify_path(dir);
    format!("ok={} failures={:?}", report.ok, report.failures)
}

#[test]
fn healthy_log_with_a_watermark_verifies() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("log");
    three_frame_log(&dir);
    assert!(dir.join("head.state").is_file());
    let report = strata_verify::verify_path(&dir);
    assert!(report.ok, "{}", report_text(&dir));
}

#[test]
fn acked_frames_cut_off_at_a_frame_boundary_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("log");
    three_frame_log(&dir);
    let seg = segments(&dir).remove(0);
    let bytes = fs::read(&seg).unwrap();
    let (_f1, n1) = strata::parse_frame(&bytes[strata::HEADER_WIRE_SIZE..]).unwrap();
    let (_f2, n2) = strata::parse_frame(&bytes[strata::HEADER_WIRE_SIZE + n1..]).unwrap();
    fs::write(&seg, &bytes[..strata::HEADER_WIRE_SIZE + n1 + n2]).unwrap();

    assert!(
        StrataLog::open(&dir).is_err(),
        "the log itself refuses this directory"
    );
    let report = strata_verify::verify_path(&dir);
    assert!(
        !report.ok,
        "verify must refuse what open refuses: {}",
        report_text(&dir)
    );
    assert!(
        report.failures.iter().any(|f| f.contains("head.state")),
        "the failure names the watermark: {}",
        report_text(&dir)
    );
}

#[test]
fn a_missing_tail_segment_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("log");
    {
        let log = StrataLog::open(&dir).unwrap();
        log.append(1, b"one").unwrap();
        log.append(1, b"two").unwrap();
        log.seal().unwrap();
        log.append(1, b"three").unwrap();
    }
    let segs = segments(&dir);
    assert_eq!(segs.len(), 2);
    fs::remove_file(&segs[1]).unwrap();

    assert!(StrataLog::open(&dir).is_err());
    let report = strata_verify::verify_path(&dir);
    assert!(!report.ok, "{}", report_text(&dir));
}

#[test]
fn an_unreadable_head_state_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("log");
    three_frame_log(&dir);
    fs::write(dir.join("head.state"), [1u8, 2, 3]).unwrap();

    assert!(StrataLog::open(&dir).is_err());
    let report = strata_verify::verify_path(&dir);
    assert!(!report.ok, "{}", report_text(&dir));
    assert!(
        report.failures.iter().any(|f| f.contains("head.state")),
        "{}",
        report_text(&dir)
    );
}

#[test]
fn a_log_without_a_watermark_still_verifies_when_clean() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("log");
    three_frame_log(&dir);
    fs::remove_file(dir.join("head.state")).unwrap();
    assert!(StrataLog::open(&dir).is_ok(), "open accepts a clean log");
    let report = strata_verify::verify_path(&dir);
    assert!(report.ok, "{}", report_text(&dir));
}
