//! Markdown link parsing and graph building.
//!
//! This module provides functionality for parsing different types of links in markdown
//! files and building a graph representation of the connections between files.

use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

/// The type of link found in markdown content.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    /// Wiki-style link: [[Page]]
    Wiki,
    /// Markdown-style link: [text](url)
    Markdown,
}

/// A parsed link from markdown content.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    /// The line number where the link was found (1-indexed).
    pub source_line: usize,
    /// The raw target string as it appears in the link.
    pub target_raw: String,
    /// The parsed path component (without anchor).
    pub target_path: Option<String>,
    /// The parsed anchor component (after #).
    pub target_anchor: Option<String>,
    /// The type of link.
    pub link_type: LinkKind,
    /// The display text for the link (wiki links only).
    pub display_text: Option<String>,
}

/// A node in the link graph representing a file.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    /// The file path of this node.
    pub path: String,
    /// List of files this node links to.
    pub outgoing: Vec<String>,
    /// List of files that link to this node.
    pub incoming: Vec<String>,
}

/// Statistics computed from the link graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    /// Total number of nodes (files) in the graph.
    pub nodes: usize,
    /// Total number of edges (links) in the graph.
    pub edges: usize,
    /// Number of nodes with no incoming or outgoing links.
    pub orphans: usize,
    /// The file with the most incoming links.
    pub most_linked: Option<(String, usize)>,
    /// The file with the most outgoing links.
    pub most_linking: Option<(String, usize)>,
}

/// An edge in the link graph with metadata.
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    /// The source file path.
    pub from: String,
    /// The target file path.
    pub to: String,
    /// The type of link.
    pub link_type: LinkKind,
    /// The line number where the link occurs.
    pub line: usize,
}

/// A node in the frontmatter graph with field data.
#[derive(Debug, Clone, Serialize)]
pub struct FrontmatterNode {
    /// The file path of this node.
    pub id: String,
    /// Frontmatter fields to include in output.
    pub fields: serde_json::Value,
}

/// An edge in the frontmatter graph.
#[derive(Debug, Clone, Serialize)]
pub struct FrontmatterEdge {
    /// The source file path.
    pub source: String,
    /// The target file path.
    pub target: String,
    /// The shared value (for shared relations) or null (for ref relations).
    pub value: Option<String>,
}

/// Result of building a frontmatter graph.
#[derive(Debug, Clone, Serialize)]
pub struct FrontmatterGraph {
    /// Metadata about the graph.
    pub meta: FrontmatterGraphMeta,
    /// Nodes in the graph.
    pub nodes: Vec<FrontmatterNode>,
    /// Edges between nodes.
    pub edges: Vec<FrontmatterEdge>,
}

/// Metadata about a frontmatter graph.
#[derive(Debug, Clone, Serialize)]
pub struct FrontmatterGraphMeta {
    /// Total number of nodes.
    pub nodes: usize,
    /// Total number of edges.
    pub edges: usize,
    /// The field used to build relations.
    pub field: String,
    /// The relation type used.
    pub relation: String,
}

/// Extract all links from markdown content.
///
/// Parses both wiki-style links (`[[Page]]`) and markdown-style links (`[text](url)`).
/// Image links (starting with `!`) are skipped.
///
/// # Arguments
/// * `content` - The markdown content to parse
///
/// # Returns
/// A vector of all links found in the content, with line numbers.
pub fn parse_links(content: &str) -> Vec<Link> {
    let mut links = Vec::new();

    // Regex patterns for wiki and markdown links
    let wiki_regex = Regex::new(r"\[\[([^\]]+)\]\]").unwrap();
    // Match [text](url) - we'll filter out image links (![...]) manually
    let md_regex = Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").unwrap();

    let lines: Vec<&str> = content.lines().collect();

    for (line_idx, line) in lines.iter().enumerate() {
        let line_num = line_idx + 1;

        // Parse wiki links
        for cap in wiki_regex.captures_iter(line) {
            let target = cap.get(1).unwrap().as_str();
            let (target_path, target_anchor, display_text) = parse_wiki_link_target(target);

            links.push(Link {
                source_line: line_num,
                target_raw: target.to_string(),
                target_path,
                target_anchor,
                link_type: LinkKind::Wiki,
                display_text,
            });
        }

        // Parse markdown links, skipping image links
        for cap in md_regex.captures_iter(line) {
            let match_start = cap.get(0).unwrap().start();
            // Skip if preceded by '!' (image link)
            if match_start > 0 && line.as_bytes()[match_start - 1] == b'!' {
                continue;
            }

            let display = cap.get(1).unwrap().as_str();
            let target = cap.get(2).unwrap().as_str();
            let (target_path, target_anchor) = parse_markdown_link_target(target);

            links.push(Link {
                source_line: line_num,
                target_raw: target.to_string(),
                target_path,
                target_anchor,
                link_type: LinkKind::Markdown,
                display_text: Some(display.to_string()),
            });
        }
    }

    links
}

/// Parse a wiki link target into its components.
///
/// Wiki links can have the format:
/// - `[[Page]]`
/// - `[[Page|display text]]`
/// - `[[Page#anchor]]`
/// - `[[Page#anchor|display text]]`
fn parse_wiki_link_target(target: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut parts = target.split('|');

    let link_part = parts.next().unwrap_or("");
    let display_text = parts.next().map(|s| s.to_string());

    // Split anchor from path
    if let Some(anchor_pos) = link_part.find('#') {
        let path = link_part[..anchor_pos].to_string();
        let anchor = link_part[anchor_pos + 1..].to_string();
        (
            if path.is_empty() { None } else { Some(path) },
            if anchor.is_empty() { None } else { Some(anchor) },
            display_text,
        )
    } else {
        let path = link_part.to_string();
        (
            if path.is_empty() { None } else { Some(path) },
            None,
            display_text,
        )
    }
}

/// Parse a markdown link target into its components.
///
/// Markdown links can have the format:
/// - `[text](path)`
/// - `[text](path#anchor)`
fn parse_markdown_link_target(target: &str) -> (Option<String>, Option<String>) {
    if let Some(anchor_pos) = target.find('#') {
        let path = target[..anchor_pos].to_string();
        let anchor = target[anchor_pos + 1..].to_string();
        (
            if path.is_empty() { None } else { Some(path) },
            if anchor.is_empty() { None } else { Some(anchor) },
        )
    } else {
        let path = target.to_string();
        (
            if path.is_empty() { None } else { Some(path) },
            None,
        )
    }
}

/// Resolve a relative link path to an absolute file path.
///
/// # Arguments
/// * `link` - The link to resolve
/// * `source_file` - The path of the file containing the link
/// * `known_files` - List of known file paths in the project
///
/// # Returns
/// The resolved absolute path if found, None otherwise.
pub fn resolve_link_path(
    link: &Link,
    source_file: &str,
    known_files: &[String],
) -> Option<String> {
    let target_path = link.target_path.as_ref()?;

    // If it's already an absolute path (starts with /), use it directly
    if target_path.starts_with('/') {
        if known_files.contains(&target_path.to_string()) {
            return Some(target_path.clone());
        }
        return None;
    }

    // Handle external URLs (http://, https://, etc.)
    if target_path.contains("://") {
        return None;
    }

    // Resolve relative path
    let source_dir = source_file.rfind('/')?;
    let base_path = &source_file[..source_dir];

    // Build the full path
    let full_path = if target_path.starts_with("./") || target_path.starts_with("../") {
        // Handle relative path navigation
        normalize_path(&format!("{}/{}", base_path, target_path))
    } else {
        // Same directory or subdirectory
        format!("{}/{}", base_path, target_path)
    };

    // Try exact match first
    if known_files.contains(&full_path) {
        return Some(full_path);
    }

    // Try without .md extension
    let without_ext = full_path.strip_suffix(".md").unwrap_or(&full_path);
    if without_ext != full_path && known_files.contains(&without_ext.to_string()) {
        return Some(without_ext.to_string());
    }

    // Try adding .md extension
    let with_ext = format!("{}.md", full_path);
    if known_files.contains(&with_ext) {
        return Some(with_ext);
    }

    // Try as filename only (search in all directories)
    let filename = target_path.split('/').last()?;
    for known_file in known_files {
        if known_file.ends_with(filename) || known_file.ends_with(&format!("{}/", filename)) {
            return Some(known_file.clone());
        }
        if known_file.ends_with(&format!("{}.md", filename)) {
            return Some(known_file.clone());
        }
    }

    None
}

/// Normalize a path by resolving . and .. components.
fn normalize_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let mut result = Vec::new();

    for part in parts {
        match part {
            "" | "." => continue,
            ".." => {
                result.pop();
            }
            _ => {
                result.push(part);
            }
        }
    }

    if result.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", result.join("/"))
    }
}

/// Build a link graph from files and their links.
///
/// # Arguments
/// * `files_with_links` - A slice of tuples containing (file_path, links)
///
/// # Returns
/// A vector of graph nodes with their connections.
pub fn build_graph(files_with_links: &[(String, Vec<Link>)]) -> Vec<GraphNode> {
    let mut nodes: HashMap<String, GraphNode> = HashMap::new();
    let known_files: Vec<String> = files_with_links
        .iter()
        .map(|(path, _)| path.clone())
        .collect();

    // Initialize all nodes
    for (path, _) in files_with_links {
        nodes.insert(
            path.clone(),
            GraphNode {
                path: path.clone(),
                outgoing: Vec::new(),
                incoming: Vec::new(),
            },
        );
    }

    // Collect all resolved connections first, then apply them
    let mut connections: Vec<(String, String)> = Vec::new();
    for (source_path, links) in files_with_links {
        for link in links {
            if let Some(resolved_target) = resolve_link_path(link, source_path, &known_files) {
                connections.push((source_path.clone(), resolved_target));
            }
        }
    }

    // Apply connections
    for (source, target) in connections {
        if let Some(source_node) = nodes.get_mut(&source) {
            if !source_node.outgoing.contains(&target) {
                source_node.outgoing.push(target.clone());
            }
        }
        if let Some(target_node) = nodes.get_mut(&target) {
            if !target_node.incoming.contains(&source) {
                target_node.incoming.push(source.clone());
            }
        }
    }

    // Convert to sorted vector
    let mut result: Vec<GraphNode> = nodes.into_values().collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));

    result
}

/// Compute statistics for the link graph.
///
/// # Arguments
/// * `nodes` - The graph nodes to analyze
///
/// # Returns
/// Graph statistics including node count, edge count, orphans, and most linked/linking files.
pub fn compute_graph_stats(nodes: &[GraphNode]) -> GraphStats {
    let node_count = nodes.len();
    let edge_count: usize = nodes.iter().map(|n| n.outgoing.len()).sum();

    let orphans = nodes
        .iter()
        .filter(|n| n.incoming.is_empty() && n.outgoing.is_empty())
        .count();

    let most_linked = nodes
        .iter()
        .filter(|n| !n.incoming.is_empty())
        .max_by_key(|n| n.incoming.len())
        .map(|n| (n.path.clone(), n.incoming.len()));

    let most_linking = nodes
        .iter()
        .filter(|n| !n.outgoing.is_empty())
        .max_by_key(|n| n.outgoing.len())
        .map(|n| (n.path.clone(), n.outgoing.len()));

    GraphStats {
        nodes: node_count,
        edges: edge_count,
        orphans,
        most_linked,
        most_linking,
    }
}

/// Collect all edges from files with their links.
///
/// Creates a flat list of edges with metadata including the link type and line number.
///
/// # Arguments
/// * `files_with_links` - A slice of tuples containing (file_path, links)
///
/// # Returns
/// A vector of graph edges with metadata.
pub fn collect_edges(files_with_links: &[(String, Vec<Link>)]) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    let known_files: Vec<String> = files_with_links
        .iter()
        .map(|(path, _)| path.clone())
        .collect();

    for (source_path, links) in files_with_links {
        for link in links {
            if let Some(resolved_target) = resolve_link_path(link, source_path, &known_files) {
                edges.push(GraphEdge {
                    from: source_path.clone(),
                    to: resolved_target,
                    link_type: link.link_type.clone(),
                    line: link.source_line,
                });
            }
        }
    }

    edges
}

/// Build a frontmatter-based graph from markdown files.
///
/// # Arguments
/// * `files` - List of file paths to process
/// * `field` - Frontmatter field name to build relations from
/// * `relation` - Relation type: "shared" (group by shared values) or "ref" (direct file references)
/// * `include_fields` - Additional frontmatter fields to include in node output
///
/// # Returns
/// A FrontmatterGraph with nodes, edges, and metadata
pub fn build_frontmatter_graph(
    files: &[String],
    field: &str,
    relation: &str,
    include_fields: &[String],
) -> FrontmatterGraph {
    use crate::frontmatter;
    use std::collections::{HashMap, HashSet};

    let mut nodes: Vec<FrontmatterNode> = Vec::new();
    let mut edges: Vec<FrontmatterEdge> = Vec::new();

    // Parse frontmatter from all files
    let mut file_field_values: HashMap<String, Vec<String>> = HashMap::new();
    let mut file_frontmatter: HashMap<String, Option<frontmatter::FrontmatterData>> = HashMap::new();

    for file_path in files {
        if let Ok(content) = std::fs::read_to_string(file_path) {
            let fm_data = frontmatter::parse_frontmatter(&content);
            file_frontmatter.insert(file_path.clone(), fm_data.clone());

            if let Some(ref fm) = fm_data {
                if let Some(field_obj) = frontmatter::get_field(fm, field) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&field_obj.value_json) {
                        let values = extract_string_values(&value);
                        file_field_values.insert(file_path.clone(), values);
                    }
                }
            }
        }
    }

    // Build nodes with included fields
    let include_fields_set: HashSet<&String> = include_fields.iter().collect();
    for file_path in files {
        let mut fields_map = serde_json::Map::new();

        if let Some(Some(ref fm)) = file_frontmatter.get(file_path) {
            for f in &fm.fields {
                if include_fields_set.is_empty() || include_fields_set.contains(&f.key) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&f.value_json) {
                        fields_map.insert(f.key.clone(), value);
                    }
                }
            }
        }

        nodes.push(FrontmatterNode {
            id: file_path.clone(),
            fields: serde_json::Value::Object(fields_map),
        });
    }

    // Build edges based on relation type
    match relation {
        "shared" => {
            // Group files by shared field values
            let mut value_to_files: HashMap<String, Vec<String>> = HashMap::new();

            for (file_path, values) in &file_field_values {
                for value in values {
                    value_to_files.entry(value.clone()).or_default().push(file_path.clone());
                }
            }

            // Create edges between files sharing values
            let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();
            for (_value, files_with_value) in &value_to_files {
                if files_with_value.len() > 1 {
                    for (i, source) in files_with_value.iter().enumerate() {
                        for target in files_with_value.iter().skip(i + 1) {
                            let edge_key = (source.clone(), target.clone(), _value.clone());
                            let edge_key_rev = (target.clone(), source.clone(), _value.clone());

                            if !seen_edges.contains(&edge_key) && !seen_edges.contains(&edge_key_rev) {
                                edges.push(FrontmatterEdge {
                                    source: source.clone(),
                                    target: target.clone(),
                                    value: Some(_value.clone()),
                                });
                                seen_edges.insert(edge_key);
                            }
                        }
                    }
                }
            }
        }
        "ref" => {
            // Field values point to other files
            let known_files_set: HashSet<&String> = files.iter().collect();

            for (source_file, values) in &file_field_values {
                for target_ref in values {
                    // Try to resolve the reference to a known file
                    if let Some(resolved) = resolve_file_reference(target_ref, files) {
                        if known_files_set.contains(&resolved) && resolved != *source_file {
                            edges.push(FrontmatterEdge {
                                source: source_file.clone(),
                                target: resolved,
                                value: None,
                            });
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let meta = FrontmatterGraphMeta {
        nodes: nodes.len(),
        edges: edges.len(),
        field: field.to_string(),
        relation: relation.to_string(),
    };

    FrontmatterGraph { meta, nodes, edges }
}

/// Extract string values from a JSON value (handles strings and arrays of strings).
fn extract_string_values(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    }
}

/// Resolve a file reference to an actual file path.
fn resolve_file_reference(reference: &str, known_files: &[String]) -> Option<String> {
    // Exact match
    if let Some(found) = known_files.iter().find(|f| *f == reference) {
        return Some(found.clone());
    }

    // Try without .md extension
    let without_ext = reference.trim_end_matches(".md");
    if let Some(found) = known_files.iter().find(|f| f.trim_end_matches(".md") == without_ext) {
        return Some(found.clone());
    }

    // Try with .md extension
    let with_ext = format!("{}.md", reference.trim_end_matches(".md"));
    if let Some(found) = known_files.iter().find(|f| f.as_str() == with_ext.as_str()) {
        return Some(found.clone());
    }

    // Try basename match
    let basename = reference.rsplit('/').next().unwrap_or(reference);
    let basename_without_ext = basename.trim_end_matches(".md");
    for file in known_files {
        let file_basename = file.rsplit('/').next().unwrap_or(file);
        if file_basename.trim_end_matches(".md") == basename_without_ext {
            return Some(file.clone());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wiki_links() {
        let content = "This is a [[WikiLink]] and [[Page|display text]]";
        let links = parse_links(content);

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].link_type, LinkKind::Wiki);
        assert_eq!(links[0].target_path, Some("WikiLink".to_string()));
        assert_eq!(links[1].display_text, Some("display text".to_string()));
    }

    #[test]
    fn test_parse_markdown_links() {
        let content = "This is a [link](target.md) and [another](page.md#anchor)";
        let links = parse_links(content);

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].link_type, LinkKind::Markdown);
        assert_eq!(links[0].target_path, Some("target.md".to_string()));
        assert_eq!(links[1].target_anchor, Some("anchor".to_string()));
    }

    #[test]
    fn test_skip_image_links() {
        let content = "This is ![image](img.png) and [link](page.md)";
        let links = parse_links(content);

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_path, Some("page.md".to_string()));
    }

    #[test]
    fn test_build_graph() {
        let files = vec![
            ("/a.md".to_string(), vec![]),
            (
                "/b.md".to_string(),
                vec![Link {
                    source_line: 1,
                    target_raw: "a".to_string(),
                    target_path: Some("a.md".to_string()),
                    target_anchor: None,
                    link_type: LinkKind::Wiki,
                    display_text: None,
                }],
            ),
        ];

        let graph = build_graph(&files);

        assert_eq!(graph.len(), 2);
        let b_node = graph.iter().find(|n| n.path == "/b.md").unwrap();
        assert_eq!(b_node.outgoing, vec!["/a.md"]);
    }

    #[test]
    fn test_graph_stats() {
        let nodes = vec![
            GraphNode {
                path: "/a.md".to_string(),
                outgoing: vec![],
                incoming: vec!["/b.md".to_string()],
            },
            GraphNode {
                path: "/b.md".to_string(),
                outgoing: vec!["/a.md".to_string()],
                incoming: vec![],
            },
            GraphNode {
                path: "/c.md".to_string(),
                outgoing: vec![],
                incoming: vec![],
            },
        ];

        let stats = compute_graph_stats(&nodes);

        assert_eq!(stats.nodes, 3);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.orphans, 1);
        assert_eq!(stats.most_linked, Some(("/a.md".to_string(), 1)));
        assert_eq!(stats.most_linking, Some(("/b.md".to_string(), 1)));
    }

    #[test]
    fn test_wiki_links_with_anchors() {
        let content = "See [[Page#section]] for details";
        let links = parse_links(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_raw, "Page#section");
        assert_eq!(links[0].target_path, Some("Page".to_string()));
        assert_eq!(links[0].target_anchor, Some("section".to_string()));
        assert_eq!(links[0].link_type, LinkKind::Wiki);
    }

    #[test]
    fn test_wiki_links_with_display_and_anchors() {
        let content = "See [[Page#section|display text]] for details";
        let links = parse_links(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_raw, "Page#section|display text");
        assert_eq!(links[0].target_path, Some("Page".to_string()));
        assert_eq!(links[0].target_anchor, Some("section".to_string()));
        assert_eq!(links[0].display_text, Some("display text".to_string()));
        assert_eq!(links[0].link_type, LinkKind::Wiki);
    }

    #[test]
    fn test_anchor_only_wiki_links() {
        let content = "See [[#section]] for details";
        let links = parse_links(content);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_raw, "#section");
        assert_eq!(links[0].target_path, None);
        assert_eq!(links[0].target_anchor, Some("section".to_string()));
        assert_eq!(links[0].link_type, LinkKind::Wiki);
    }

    #[test]
    fn test_multiple_links_same_line() {
        let content = "See [[Page1]] and [[Page2]] for details";
        let links = parse_links(content);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target_raw, "Page1");
        assert_eq!(links[1].target_raw, "Page2");
    }

    #[test]
    fn test_no_links_returns_empty() {
        let content = "This is just plain text with no links";
        let links = parse_links(content);
        assert!(links.is_empty());
        assert_eq!(links.len(), 0);
    }

    #[test]
    fn test_resolve_link_path_relative() {
        let link = Link {
            source_line: 1,
            target_raw: "../other/page.md".to_string(),
            target_path: Some("../other/page.md".to_string()),
            target_anchor: None,
            link_type: LinkKind::Markdown,
            display_text: None,
        };
        let source_file = "/docs/main/index.md";
        let known_files = &["/docs/other/page.md".to_string()];
        
        let result = resolve_link_path(&link, source_file, known_files);
        assert_eq!(result, Some("/docs/other/page.md".to_string()));
    }

    #[test]
    fn test_resolve_link_path_external_url_returns_none() {
        let link = Link {
            source_line: 1,
            target_raw: "https://example.com/page".to_string(),
            target_path: Some("https://example.com/page".to_string()),
            target_anchor: None,
            link_type: LinkKind::Markdown,
            display_text: None,
        };
        let source_file = "/docs/main/index.md";
        let known_files = &[];
        
        let result = resolve_link_path(&link, source_file, known_files);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_link_path_adds_md_extension() {
        let link = Link {
            source_line: 1,
            target_raw: "otherpage".to_string(),
            target_path: Some("otherpage".to_string()),
            target_anchor: None,
            link_type: LinkKind::Wiki,
            display_text: None,
        };
        let source_file = "/docs/main/index.md";
        let known_files = &["/docs/main/otherpage.md".to_string()];
        
        let result = resolve_link_path(&link, source_file, known_files);
        assert_eq!(result, Some("/docs/main/otherpage.md".to_string()));
    }

    #[test]
    fn test_resolve_link_path_absolute() {
        let link = Link {
            source_line: 1,
            target_raw: "/absolute/path/page.md".to_string(),
            target_path: Some("/absolute/path/page.md".to_string()),
            target_anchor: None,
            link_type: LinkKind::Markdown,
            display_text: None,
        };
        let source_file = "/docs/main/index.md";
        let known_files = &["/absolute/path/page.md".to_string()];
        
        let result = resolve_link_path(&link, source_file, known_files);
        assert_eq!(result, Some("/absolute/path/page.md".to_string()));
    }

    #[test]
    fn test_empty_graph() {
        let files_with_links: &[(String, Vec<Link>)] = &[];
        let graph = build_graph(files_with_links);
        assert!(graph.is_empty());
    }

    #[test]
    fn test_graph_with_self_referencing_links() {
        let link = Link {
            source_line: 1,
            target_raw: "SamePage".to_string(),
            target_path: Some("SamePage".to_string()),
            target_anchor: None,
            link_type: LinkKind::Wiki,
            display_text: None,
        };
        let files_with_links = &[
            ("/a.md".to_string(), vec![link.clone()]),
            ("/SamePage.md".to_string(), vec![link]),
        ];
        
        let graph = build_graph(files_with_links);
        
        // /a.md links to /SamePage.md
        let a_node = graph.iter().find(|n| n.path == "/a.md").unwrap();
        assert_eq!(a_node.outgoing, vec!["/SamePage.md".to_string()]);
        
        // /SamePage.md also links to /SamePage.md (self-reference)
        let same_node = graph.iter().find(|n| n.path == "/SamePage.md").unwrap();
        assert!(same_node.outgoing.contains(&"/SamePage.md".to_string()));
    }

    #[test]
    fn test_collect_edges_basic() {
        let link1 = Link {
            source_line: 5,
            target_raw: "TargetPage".to_string(),
            target_path: Some("TargetPage".to_string()),
            target_anchor: None,
            link_type: LinkKind::Wiki,
            display_text: None,
        };
        let link2 = Link {
            source_line: 10,
            target_raw: "other.md".to_string(),
            target_path: Some("other.md".to_string()),
            target_anchor: None,
            link_type: LinkKind::Markdown,
            display_text: Some("click here".to_string()),
        };
        let files_with_links = &[
            ("/source.md".to_string(), vec![link1, link2]),
            ("/TargetPage.md".to_string(), vec![]),
            ("/other.md".to_string(), vec![]),
        ];
        
        let edges = collect_edges(files_with_links);
        assert_eq!(edges.len(), 2);
        
        assert_eq!(edges[0].from, "/source.md");
        assert_eq!(edges[0].to, "/TargetPage.md");
        assert_eq!(edges[0].link_type, LinkKind::Wiki);
        assert_eq!(edges[0].line, 5);
        
        assert_eq!(edges[1].from, "/source.md");
        assert_eq!(edges[1].to, "/other.md");
        assert_eq!(edges[1].link_type, LinkKind::Markdown);
        assert_eq!(edges[1].line, 10);
    }

    #[test]
    fn test_collect_edges_empty_links() {
        let files_with_links: &[(String, Vec<Link>); 2] = &[
            ("/source.md".to_string(), vec![]),
            ("/other.md".to_string(), vec![]),
        ];
        
        let edges = collect_edges(files_with_links);
        assert!(edges.is_empty());
    }

    #[test]
    fn test_graph_stats_all_orphans() {
        let nodes = vec![
            GraphNode {
                path: "/a.md".to_string(),
                outgoing: vec![],
                incoming: vec![],
            },
            GraphNode {
                path: "/b.md".to_string(),
                outgoing: vec![],
                incoming: vec![],
            },
            GraphNode {
                path: "/c.md".to_string(),
                outgoing: vec![],
                incoming: vec![],
            },
        ];

        let stats = compute_graph_stats(&nodes);

        assert_eq!(stats.nodes, 3);
        assert_eq!(stats.edges, 0);
        assert_eq!(stats.orphans, 3);
        assert_eq!(stats.most_linked, None);
        assert_eq!(stats.most_linking, None);
    }

    #[test]
    fn test_orphan_detection() {
        // Orphan = no incoming links (regardless of outgoing)
        let nodes = vec![
            GraphNode {
                path: "/index.md".to_string(),
                outgoing: vec!["/about.md".to_string(), "/old.md".to_string()],
                incoming: vec!["/about.md".to_string()],
            },
            GraphNode {
                path: "/about.md".to_string(),
                outgoing: vec!["/index.md".to_string()],
                incoming: vec!["/index.md".to_string()],
            },
            GraphNode {
                path: "/old.md".to_string(),
                outgoing: vec!["/about.md".to_string()],
                incoming: vec!["/index.md".to_string()],
            },
            GraphNode {
                path: "/orphan-with-links.md".to_string(),
                outgoing: vec!["/index.md".to_string(), "/about.md".to_string()],
                incoming: vec![],
            },
            GraphNode {
                path: "/isolated.md".to_string(),
                outgoing: vec![],
                incoming: vec![],
            },
        ];

        let orphans: Vec<_> = nodes.iter()
            .filter(|n| n.incoming.is_empty())
            .collect();

        assert_eq!(orphans.len(), 2);
        assert_eq!(orphans[0].path, "/orphan-with-links.md");
        assert_eq!(orphans[1].path, "/isolated.md");

        // orphan-with-links has outgoing but still qualifies as orphan
        assert_eq!(orphans[0].outgoing.len(), 2);
        assert!(orphans[0].incoming.is_empty());

        // isolated has neither incoming nor outgoing
        assert!(orphans[1].outgoing.is_empty());
        assert!(orphans[1].incoming.is_empty());
    }

    #[test]
    fn test_links_different_lines_correct_line_numbers() {
        let content = "Line 1: [[Page1]]\nLine 2: [[Page2]]\nLine 3: [[Page3]]";
        let links = parse_links(content);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].source_line, 1);
        assert_eq!(links[1].source_line, 2);
        assert_eq!(links[2].source_line, 3);
        assert_eq!(links[0].target_raw, "Page1");
        assert_eq!(links[1].target_raw, "Page2");
        assert_eq!(links[2].target_raw, "Page3");
    }
}
