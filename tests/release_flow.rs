//! Интеграционный прогон на временном git-репозитории.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

use releaser_rs::release::{self, Options};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(dir: &Path, message: &str) {
    let file = dir.join("work.txt");
    let mut content = std::fs::read_to_string(&file).unwrap_or_default();
    content.push_str(message);
    content.push('\n');
    std::fs::write(&file, content).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test"]);
    git(dir.path(), &["checkout", "-q", "-b", "main"]);
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("work.txt"), "init\n").unwrap();
    git(dir.path(), &["add", "."]);
    // Release-коммит исключается из changelog/bump.
    git(
        dir.path(),
        &["commit", "-q", "-m", "chore(release): v0.1.0"],
    );
    dir
}

fn options(dir: &Path) -> Options {
    Options {
        root: dir.to_path_buf(),
        ..Options::default()
    }
}

#[test]
fn releases_feat_and_is_idempotent() {
    let dir = setup();
    commit(dir.path(), "feat(core): add feature");
    commit(dir.path(), "fix: repair bug");

    let preview = release::preview(&options(dir.path())).unwrap();
    assert_eq!(preview.next.as_deref(), Some("0.2.0"));
    assert_eq!(preview.commits, 2);
    assert!(!preview.already_released);

    let outcome = release::apply(&options(dir.path())).unwrap();
    assert!(outcome.committed);
    assert!(outcome.tagged);
    assert!(outcome.changelog_written);
    assert_eq!(outcome.tag.as_deref(), Some("v0.2.0"));

    let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(manifest.contains("version = \"0.2.0\""));
    let changelog = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## [0.2.0]"));
    assert!(changelog.contains("### Features"));
    assert!(changelog.contains("#### core"));
    assert!(changelog.contains("- add feature"));
    assert!(changelog.contains("### Fixes"));
    assert!(changelog.contains("- repair bug"));

    let tags = Command::new("git")
        .current_dir(dir.path())
        .args(["tag", "--list"])
        .output()
        .unwrap();
    let tags = String::from_utf8_lossy(&tags.stdout);
    assert!(tags.lines().any(|tag| tag == "v0.2.0"));

    let second = release::apply(&options(dir.path())).unwrap();
    assert!(second.skipped);
    assert_eq!(second.previous, "0.2.0");
}

#[test]
fn dry_run_does_not_write_anything() {
    let dir = setup();
    commit(dir.path(), "feat: new api");
    let mut dry = options(dir.path());
    dry.dry_run = true;

    let outcome = release::apply(&dry).unwrap();
    assert!(outcome.dry_run);
    assert_eq!(outcome.next.as_deref(), Some("0.2.0"));
    assert!(!dir.path().join("CHANGELOG.md").exists());
    let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
    assert!(manifest.contains("version = \"0.1.0\""));
    let tags = Command::new("git")
        .current_dir(dir.path())
        .args(["tag", "--list"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&tags.stdout).trim().is_empty());
}

#[test]
fn check_reports_non_conventional_commits() {
    let dir = setup();
    commit(dir.path(), "feat: valid change");
    commit(dir.path(), "just a message");

    let report = release::check_commits(&options(dir.path())).unwrap();
    assert_eq!(report.conventional, 1);
    assert_eq!(report.invalid, vec!["just a message".to_string()]);
    assert!(report.config_ok);
}

#[test]
fn check_accepts_an_explicit_commit_range() {
    let dir = setup();
    commit(dir.path(), "feat: valid change");
    commit(dir.path(), "just a message");

    let mut ranged = options(dir.path());
    ranged.commits = Some("HEAD~1..HEAD".into());
    let report = release::check_commits(&ranged).unwrap();
    assert_eq!(report.total, 1);
    assert_eq!(report.invalid, vec!["just a message".to_string()]);
}

#[test]
fn external_template_replaces_the_changelog_body() {
    let dir = setup();
    commit(dir.path(), "feat: template feature");
    std::fs::write(
        dir.path().join(".releaserc.toml"),
        "template = \"release/notes.hbs\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("release")).unwrap();
    std::fs::write(
        dir.path().join("release/notes.hbs"),
        "# Notes {version} ({date})\n{repo_url}\n{sections}\n",
    )
    .unwrap();

    let mut opts = options(dir.path());
    opts.no_git_checks = true;
    let preview = release::preview(&opts).unwrap();
    let changelog = preview.changelog.unwrap();
    assert!(changelog.contains("# Notes 0.2.0"));
    assert!(changelog.contains("template feature"));
}

#[test]
fn prerelease_channel_produces_numbered_version() {
    let dir = setup();
    commit(dir.path(), "feat: next");
    let mut prerelease = options(dir.path());
    prerelease.prerelease = Some("beta".into());

    let preview = release::preview(&prerelease).unwrap();
    assert_eq!(preview.next.as_deref(), Some("0.2.0-beta.1"));
    assert_eq!(preview.tag.as_deref(), Some("v0.2.0-beta.1"));
}
