use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_markdownai"))
}

/// Helper: create a temp dir with md files and return the tempdir handle.
fn setup_graph_files() -> tempfile::TempDir {
    let dir = tempdir().unwrap();

    // index.md links to about.md and guide.md
    fs::write(
        dir.path().join("index.md"),
        "# Index\nSee [About](about.md) and [Guide](guide.md)\n",
    )
    .unwrap();

    // about.md links back to index.md
    fs::write(
        dir.path().join("about.md"),
        "# About\nBack to [Index](index.md)\n",
    )
    .unwrap();

    // guide.md links to about.md
    fs::write(
        dir.path().join("guide.md"),
        "# Guide\nSee [About](about.md)\n",
    )
    .unwrap();

    // orphan.md links out but nobody links to it
    fs::write(
        dir.path().join("orphan.md"),
        "# Orphan\nSee [About](about.md)\n",
    )
    .unwrap();

    // isolated.md has no links at all
    fs::write(dir.path().join("isolated.md"), "# Isolated\nNo links here.\n").unwrap();

    dir
}

#[test]
fn graph_orphans_raw() {
    let dir = setup_graph_files();

    cmd()
        .args(["graph", dir.path().to_str().unwrap(), "--format", "orphans"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5 files, 2 orphans"))
        .stdout(predicate::str::contains("(0 in,"))
        .stdout(predicate::str::contains("orphan.md"))
        .stdout(predicate::str::contains("isolated.md"));
}

#[test]
fn graph_orphans_json() {
    let dir = setup_graph_files();

    let output = cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "orphans",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["files"], 5);
    assert_eq!(json["meta"]["orphans"], 2);

    let orphans = json["orphans"].as_array().unwrap();
    assert_eq!(orphans.len(), 2);

    // orphan.md has outgoing links
    let orphan_entry = orphans.iter().find(|e| {
        e["path"].as_str().unwrap().ends_with("orphan.md")
    }).unwrap();
    assert_eq!(orphan_entry["out_degree"], 1);
}

#[test]
fn graph_orphans_json_pretty() {
    let dir = setup_graph_files();

    cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "orphans",
            "--json",
            "--pretty",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"orphans\""))
        .stdout(predicate::str::contains("\"meta\""));
}

#[test]
fn graph_orphans_all_linked() {
    let dir = tempdir().unwrap();

    // Two files that link to each other — no orphans
    fs::write(dir.path().join("a.md"), "# A\n[B](b.md)\n").unwrap();
    fs::write(dir.path().join("b.md"), "# B\n[A](a.md)\n").unwrap();

    cmd()
        .args(["graph", dir.path().to_str().unwrap(), "--format", "orphans"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 files, 0 orphans"));
}

#[test]
fn graph_orphans_all_isolated() {
    let dir = tempdir().unwrap();

    fs::write(dir.path().join("a.md"), "# A\nNo links\n").unwrap();
    fs::write(dir.path().join("b.md"), "# B\nNo links\n").unwrap();
    fs::write(dir.path().join("c.md"), "# C\nNo links\n").unwrap();

    cmd()
        .args(["graph", dir.path().to_str().unwrap(), "--format", "orphans"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3 files, 3 orphans"));
}

#[test]
fn graph_orphans_existing_formats_still_work() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.md"), "# A\n[B](b.md)\n").unwrap();
    fs::write(dir.path().join("b.md"), "# B\n").unwrap();
    let path = dir.path().to_str().unwrap();

    // stats
    cmd()
        .args(["graph", path, "--format", "stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nodes:"));

    // edges
    cmd()
        .args(["graph", path, "--format", "edges"])
        .assert()
        .success();

    // adjacency
    cmd()
        .args(["graph", path, "--format", "adjacency"])
        .assert()
        .success();
}
