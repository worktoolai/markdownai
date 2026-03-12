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

#[test]
fn graph_frontmatter_shared_relations() {
    let dir = tempdir().unwrap();

    // Create files with tags field
    fs::write(
        dir.path().join("a.md"),
        "---\ntags: [rust, cli]\n---\n# A\n",
    ).unwrap();

    fs::write(
        dir.path().join("b.md"),
        "---\ntags: [rust, testing]\n---\n# B\n",
    ).unwrap();

    fs::write(
        dir.path().join("c.md"),
        "---\ntags: [cli, docs]\n---\n# C\n",
    ).unwrap();

    let output = cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--field",
            "tags",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["field"], "tags");
    assert_eq!(json["meta"]["relation"], "shared");
    assert_eq!(json["meta"]["nodes"], 3);

    // Should have edges between files sharing tags
    // a.md and b.md share "rust"
    // a.md and c.md share "cli"
    let edges = json["edges"].as_array().unwrap();
    assert!(edges.len() >= 2);
}

#[test]
fn graph_frontmatter_ref_relations() {
    let dir = tempdir().unwrap();

    // Create files with references field
    fs::write(
        dir.path().join("index.md"),
        "---\nreferences: [guide.md, api.md]\n---\n# Index\n",
    ).unwrap();

    fs::write(
        dir.path().join("guide.md"),
        "---\nreferences: [api.md]\n---\n# Guide\n",
    ).unwrap();

    fs::write(
        dir.path().join("api.md"),
        "---\nreferences: []\n---\n# API\n",
    ).unwrap();

    let output = cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--field",
            "references",
            "--relation",
            "ref",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["field"], "references");
    assert_eq!(json["meta"]["relation"], "ref");
    assert_eq!(json["meta"]["nodes"], 3);

    // index.md -> guide.md, api.md
    // guide.md -> api.md
    let edges = json["edges"].as_array().unwrap();
    assert!(edges.len() >= 2);

    // Check that ref edges have null value
    for edge in edges {
        assert_eq!(edge["value"], serde_json::Value::Null);
    }
}

#[test]
fn graph_frontmatter_include_fields() {
    let dir = tempdir().unwrap();

    fs::write(
        dir.path().join("doc.md"),
        "---\ntags: [rust]\ntitle: My Doc\nstatus: published\n---\n# Doc\n",
    ).unwrap();

    let output = cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--field",
            "tags",
            "--include",
            "title,status",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    let nodes = json["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 1);

    // Node should include the requested fields
    let node = &nodes[0];
    assert_eq!(node["fields"]["title"], "My Doc");
    assert_eq!(node["fields"]["status"], "published");
}

#[test]
fn graph_frontmatter_requires_field() {
    let dir = tempdir().unwrap();

    fs::write(
        dir.path().join("doc.md"),
        "# Doc\n",
    ).unwrap();

    cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--json",
        ])
        .assert()
        .failure();
}

#[test]
fn graph_frontmatter_requires_json() {
    let dir = tempdir().unwrap();

    fs::write(
        dir.path().join("doc.md"),
        "---\ntags: [rust]\n---\n# Doc\n",
    ).unwrap();

    cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--field",
            "tags",
        ])
        .assert()
        .failure();
}

#[test]
fn graph_frontmatter_empty_files() {
    let dir = tempdir().unwrap();

    let output = cmd()
        .args([
            "graph",
            dir.path().to_str().unwrap(),
            "--format",
            "frontmatter",
            "--field",
            "tags",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["meta"]["nodes"], 0);
    assert_eq!(json["meta"]["edges"], 0);
}
