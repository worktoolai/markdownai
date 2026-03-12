use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_markdownai"))
}

/// Helper: create a temp dir with markdown files and return the tempdir handle.
fn setup_query_files() -> tempfile::TempDir {
    let dir = tempdir().unwrap();

    // File with tags field
    fs::write(
        dir.path().join("rust-guide.md"),
        "---\ntags: [rust, cli]\ntitle: Rust Guide\nstatus: published\n---\n# Rust Guide\nContent here\n",
    ).unwrap();

    // File with tags field
    fs::write(
        dir.path().join("api-docs.md"),
        "---\ntags: [rust, api]\ntitle: API Docs\nstatus: draft\n---\n# API Docs\n",
    ).unwrap();

    // File with tags field
    fs::write(
        dir.path().join("tutorial.md"),
        "---\ntags: [tutorial, beginner]\ntitle: Tutorial\nstatus: published\n---\n# Tutorial\n",
    ).unwrap();

    // File without tags field
    fs::write(
        dir.path().join("no-tags.md"),
        "---\ntitle: No Tags\nstatus: published\n---\n# No Tags\n",
    ).unwrap();

    // File without frontmatter
    fs::write(
        dir.path().join("no-fm.md"),
        "# No Frontmatter\nJust content\n",
    ).unwrap();

    dir
}

#[test]
fn frontmatter_query_basic() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 3); // Only files with tags field
    assert_eq!(json["meta"]["field"], "tags");

    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);

    // Check that each result has file and fields
    for result in results {
        assert!(result["file"].is_string());
        assert!(result["fields"]["tags"].is_array());
    }
}

#[test]
fn frontmatter_query_multiple_fields() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags,status",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 3);
    assert_eq!(json["meta"]["field"], "tags,status");

    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);

    // Check that each result has both fields
    for result in results {
        assert!(result["fields"]["tags"].is_array());
        assert!(result["fields"]["status"].is_string());
    }
}

#[test]
fn frontmatter_query_with_filter() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--filter",
            "tags contains \"rust\"",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 2); // rust-guide.md and api-docs.md

    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn frontmatter_query_no_matches() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "nonexistent",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 0);
    assert_eq!(json["results"].as_array().unwrap().len(), 0);
}

#[test]
fn frontmatter_query_count_only() {
    let dir = setup_query_files();

    cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--count-only",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""total":3"#))
        .stdout(predicate::str::contains(r#""field":"tags""#));
}

#[test]
fn frontmatter_query_raw_mode() {
    let dir = setup_query_files();

    cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("tags="));
}

#[test]
fn frontmatter_query_pretty_json() {
    let dir = setup_query_files();

    cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--json",
            "--pretty",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("  \"total\": 3"));
}

#[test]
fn frontmatter_query_limit_offset() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--limit",
            "2",
            "--offset",
            "1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 3);
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2); // limit=2
}

#[test]
fn frontmatter_query_requires_field() {
    let dir = tempdir().unwrap();

    fs::write(dir.path().join("test.md"), "# Test\n").unwrap();

    cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "",
        ])
        .assert()
        .failure();
}

#[test]
fn frontmatter_query_filter_no_matches() {
    let dir = setup_query_files();

    let output = cmd()
        .args([
            "frontmatter-query",
            dir.path().to_str().unwrap(),
            "--field",
            "tags",
            "--filter",
            "status == \"archived\"",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["total"], 0);
}
