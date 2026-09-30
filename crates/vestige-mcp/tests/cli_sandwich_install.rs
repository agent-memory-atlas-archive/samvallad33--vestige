//! Real `vestige sandwich install` against a throwaway home directory: a
//! settings file that cannot be parsed is never rewritten, and every rewrite
//! of a parseable one is preceded by a fresh backup of its current contents.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

const BACKUP_FIRST: &str = "settings.json.bak.pre-sandwich";
const BACKUP_LATEST: &str = "settings.json.bak.last-sandwich";

fn repo_source_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn install(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vestige"))
        .args(["sandwich", "install", "--src"])
        .arg(repo_source_root())
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env_remove("VESTIGE_DATA_DIR")
        .output()
        .expect("spawn vestige")
}

fn claude_dir(home: &Path) -> std::path::PathBuf {
    let dir = home.join(".claude");
    fs::create_dir_all(&dir).expect("create .claude");
    dir
}

#[test]
fn unparseable_settings_are_left_untouched_and_the_install_fails() {
    let home = TempDir::new().unwrap();
    let dir = claude_dir(home.path());
    let broken = "{ \"permissions\": { \"allow\": [\"Bash(ls)\"] ,, } \n// hand edited\n";
    fs::write(dir.join("settings.json"), broken).unwrap();

    let out = install(home.path());

    assert!(
        !out.status.success(),
        "install must fail on an unparseable settings file: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        fs::read_to_string(dir.join("settings.json")).unwrap(),
        broken,
        "the unparseable settings file must not be rewritten"
    );
    let message = String::from_utf8_lossy(&out.stderr);
    assert!(
        message.contains("settings.json"),
        "the error names the file: {message}"
    );
}

#[test]
fn non_object_settings_are_left_untouched_and_the_install_fails() {
    let home = TempDir::new().unwrap();
    let dir = claude_dir(home.path());
    fs::write(dir.join("settings.json"), "[1, 2, 3]\n").unwrap();

    let out = install(home.path());

    assert!(!out.status.success());
    assert_eq!(
        fs::read_to_string(dir.join("settings.json")).unwrap(),
        "[1, 2, 3]\n"
    );
}

#[test]
fn every_rewrite_keeps_a_fresh_backup_and_the_first_one_survives() {
    let home = TempDir::new().unwrap();
    let dir = claude_dir(home.path());
    let original = "{\"model\":\"one\",\"permissions\":{\"allow\":[\"Bash(ls)\"]}}\n";
    fs::write(dir.join("settings.json"), original).unwrap();

    let first = install(home.path());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.join(BACKUP_FIRST)).unwrap(),
        original
    );

    // The user edits the file between installs.
    let edited = "{\"model\":\"two\",\"permissions\":{\"allow\":[\"Bash(ls)\",\"Bash(pwd)\"]}}\n";
    fs::write(dir.join("settings.json"), edited).unwrap();

    let second = install(home.path());
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        fs::read_to_string(dir.join(BACKUP_FIRST)).unwrap(),
        original,
        "the first backup is kept"
    );
    assert_eq!(
        fs::read_to_string(dir.join(BACKUP_LATEST)).unwrap(),
        edited,
        "a fresh backup of the contents about to be rewritten exists"
    );
}
