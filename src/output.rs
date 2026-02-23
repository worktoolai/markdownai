#![allow(dead_code)]
//! JSON envelope output and raw markdown output formatting.
//!
//! This module follows the jsonai output.rs pattern adapted for markdown:
//! - `--json` flag switches from raw markdown to JSON envelope
//! - `--json --pretty` combination supported (default compact)
//! - Raw mode is the default
//! - Paging envelope for list commands
//! - Raw mode paging footer
//! - `truncate_to_budget` for `--max-bytes`
//! - Plan/overflow envelope support
//! - Stats output support
//! - Facets output support

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::Value;


/// Serialize a value to JSON, with optional pretty-printing.
pub fn to_json<T: Serialize>(value: &T, pretty: bool) -> String {
    if pretty {
        serde_json::to_string_pretty(value).unwrap_or_default()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Standard envelope for list commands
// ---------------------------------------------------------------------------

/// Standard JSON envelope for list command results.
#[derive(Serialize)]
pub struct Envelope<T> {
    pub meta: Meta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<T>>,
}

impl<T: Serialize> Envelope<T> {
    /// Create a new envelope with results.
    pub fn with_results(meta: Meta, results: Vec<T>) -> Self {
        Self {
            meta,
            results: Some(results),
        }
    }

    /// Create a new envelope with no results (e.g., for count-only mode).
    pub fn without_results(meta: Meta) -> Self {
        Self {
            meta,
            results: None,
        }
    }
}

/// Metadata for standard list envelopes.
#[derive(Serialize)]
pub struct Meta {
    pub total: usize,
    pub returned: usize,
    pub offset: usize,
    pub limit: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Meta {
    /// Create paging metadata for list results.
    pub fn paging(total: usize, returned: usize, offset: usize, limit: usize) -> Self {
        let has_more = offset + returned < total;
        let next_offset = if has_more { Some(offset + returned) } else { None };

        Self {
            total,
            returned,
            offset,
            limit,
            truncated: total > limit,
            has_more: Some(has_more),
            next_offset,
            file: None,
        }
    }

    /// Create metadata for a single file result.
    pub fn for_file(file: String, total: usize, returned: usize) -> Self {
        Self {
            total,
            returned,
            offset: 0,
            limit: total,
            truncated: false,
            has_more: None,
            next_offset: None,
            file: Some(file),
        }
    }

    /// Create metadata with truncation for byte budget limits.
    pub fn with_truncation(file: Option<String>, bytes_shown: usize, bytes_total: usize, _next_line: usize) -> Self {
        Self {
            total: bytes_total,
            returned: bytes_shown,
            offset: 0,
            limit: bytes_total,
            truncated: true,
            has_more: None,
            next_offset: None,
            file,
        }
    }
}

/// Format raw mode paging footer.
/// Generates: `--- N/M shown, next: --offset N ---`
pub fn format_raw_footer(returned: usize, total: usize, offset: usize) -> String {
    if offset + returned >= total {
        format!("--- {}/{} shown ---", returned, total)
    } else {
        format!("--- {}/{} shown, next: --offset {} ---", returned, total, offset + returned)
    }
}

// ---------------------------------------------------------------------------
// Byte budget truncation
// ---------------------------------------------------------------------------

/// Truncate a list of serializable items to fit within a byte budget.
/// Returns (kept_items, was_truncated).
/// Reserves ~200 bytes for the envelope/meta overhead.
pub fn truncate_to_budget<T: Serialize + Clone>(items: &[T], max_bytes: Option<usize>) -> (Vec<T>, bool) {
    let budget = match max_bytes {
        Some(b) => b,
        None => return (items.to_vec(), false),
    };

    let overhead = 200; // meta + envelope structure
    let available = budget.saturating_sub(overhead);
    let mut kept = Vec::new();
    let mut used: usize = 0;

    for item in items {
        let item_json = serde_json::to_string(item).unwrap_or_default();
        let item_bytes = item_json.len() + 2; // comma + newline
        if used + item_bytes > available && !kept.is_empty() {
            return (kept, true);
        }
        used += item_bytes;
        kept.push(item.clone());
    }

    (kept, false)
}

// ---------------------------------------------------------------------------
// Overflow plan envelope
// ---------------------------------------------------------------------------

/// Plan envelope for overflow protection.
#[derive(Serialize)]
pub struct PlanEnvelope {
    pub meta: PlanMeta,
    pub plan: Plan,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<Value>,
}

/// Metadata for plan mode.
#[derive(Serialize)]
pub struct PlanMeta {
    pub total: usize,
    pub returned: usize,
    pub overflow: bool,
    pub threshold: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_searched: Option<usize>,
}

/// Plan with suggestions for narrowing down results.
#[derive(Serialize)]
pub struct Plan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub facets: BTreeMap<String, FacetInfo>,
}

/// Facet information for a field.
pub type FacetInfo = Vec<(String, usize)>;

impl PlanEnvelope {
    /// Create a new plan envelope.
    pub fn new(
        total: usize,
        threshold: usize,
        files_searched: Option<usize>,
        suggestion: Option<String>,
        facets: BTreeMap<String, FacetInfo>,
    ) -> Self {
        Self {
            meta: PlanMeta {
                total,
                returned: 0,
                overflow: true,
                threshold,
                files_searched,
            },
            plan: Plan {
                suggestion,
                facets,
            },
            results: vec![],
        }
    }
}

// ---------------------------------------------------------------------------
// Stats output
// ---------------------------------------------------------------------------

/// File statistics for `--stats` flag.
#[derive(Serialize)]
pub struct FileStats {
    pub file: String,
    pub bytes: usize,
    pub lines: usize,
    pub sections: usize,
    pub code_blocks: usize,
    pub has_frontmatter: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<LinkStats>,
}

/// Link statistics within a file.
#[derive(Serialize)]
pub struct LinkStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
}

impl FileStats {
    /// Format file stats as raw output.
    pub fn format_raw(&self) -> String {
        let mut parts = vec![
            format!("{}: {} bytes, {} lines", self.file, self.bytes, self.lines),
            format!("{} sections", self.sections),
            format!("{} code blocks", self.code_blocks),
            format!("frontmatter: {}", if self.has_frontmatter { "yes" } else { "no" }),
        ];

        if let Some(ref links) = self.links {
            if let Some(total) = links.total {
                parts.push(format!("links: {}", total));
            } else {
                let link_count = links.wiki.unwrap_or(0) + links.markdown.unwrap_or(0);
                parts.push(format!("links: {}", link_count));
            }
        }

        parts.join(", ")
    }
}

/// Directory statistics for `--stats` on a directory.
#[derive(Serialize)]
pub struct DirStats {
    pub path: String,
    pub files: usize,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub total_sections: usize,
}

impl DirStats {
    /// Format directory stats as raw output.
    pub fn format_raw(&self) -> String {
        format!(
            "{}: {} files, {} bytes, {} lines, {} sections",
            self.path, self.files, self.total_bytes, self.total_lines, self.total_sections
        )
    }
}

// ---------------------------------------------------------------------------
// Facets output
// ---------------------------------------------------------------------------

/// Facet distribution result for `--facets` flag.
#[derive(Serialize)]
pub struct FacetsResult {
    pub field: String,
    pub total_files: usize,
    pub facets: HashMap<String, usize>,
}

impl FacetsResult {
    /// Create a new facets result.
    pub fn new(field: String, total_files: usize) -> Self {
        Self {
            field,
            total_files,
            facets: HashMap::new(),
        }
    }

    /// Add a facet value with its count.
    pub fn add(&mut self, value: String, count: usize) {
        self.facets.insert(value, count);
    }

    /// Format facets as raw output.
    /// Format: `field: value1(count1) value2(count2) ... [N files]`
    pub fn format_raw(&self) -> String {
        let mut facet_strs: Vec<String> = self
            .facets
            .iter()
            .map(|(value, count)| format!("{}({})", value, count))
            .collect();
        facet_strs.sort(); // Sort alphabetically for consistent output

        format!(
            "{}: {}  [{} files]",
            self.field,
            facet_strs.join(" "),
            self.total_files
        )
    }
}

// ---------------------------------------------------------------------------
// Overview output
// ---------------------------------------------------------------------------

/// Overview entry for a single markdown file.
#[derive(Serialize, Clone)]
pub struct OverviewEntry {
    pub file: String,
    pub lines: usize,
    pub bytes: usize,
    pub sections: usize,
    pub has_frontmatter: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub frontmatter: HashMap<String, Value>,
}

impl OverviewEntry {
    /// Format overview entry in raw mode.
    /// Line 1: file path
    /// Line 2: frontmatter fields (if any)
    /// Line 3: structural metadata
    pub fn format_raw(&self) -> String {
        let mut lines = vec![self.file.clone()];

        if !self.frontmatter.is_empty() {
            let fields: Vec<String> = self.frontmatter.iter().map(|(k, v)| {
                let val_str = match v {
                    Value::String(s) => s.clone(),
                    Value::Array(arr) => {
                        let items: Vec<String> = arr.iter().map(|item| match item {
                            Value::String(s) => format!("\"{}\"", s),
                            _ => item.to_string(),
                        }).collect();
                        format!("[{}]", items.join(", "))
                    }
                    _ => v.to_string(),
                };
                format!("{}: {}", k, val_str)
            }).collect();
            lines.push(format!("  {}", fields.join(" | ")));
        }

        lines.push(format!("  sections: {} | lines: {} | bytes: {}", self.sections, self.lines, self.bytes));
        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Chars (Unicode script) output
// ---------------------------------------------------------------------------

/// Per-script character count.
#[derive(Serialize, Clone)]
pub struct ScriptCount {
    pub script: String,
    pub count: usize,
    pub pct: f64,
}

/// Character statistics for a single file.
#[derive(Serialize, Clone)]
pub struct CharsResult {
    pub file: String,
    pub total: usize,
    pub scripts: Vec<ScriptCount>,
}

impl CharsResult {
    /// Format in raw mode.
    /// `file: total N | Hangul 98.2%(1200) Latin 1.5%(18)`
    pub fn format_raw(&self) -> String {
        let script_parts: Vec<String> = self
            .scripts
            .iter()
            .map(|s| format!("{} {:.1}%({})", s.script, s.pct, s.count))
            .collect();
        format!(
            "{}: total {} | {}",
            self.file,
            self.total,
            script_parts.join(" ")
        )
    }
}

// ---------------------------------------------------------------------------
// Search results
// ---------------------------------------------------------------------------

/// A single search result match.
#[derive(Serialize, Clone)]
pub struct SearchResult {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_index: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_title: Option<String>,
    pub line: usize,
    pub snippet: String,
    pub score: f32,
}

/// Search results envelope with multi-query support.
#[derive(Serialize)]
pub struct SearchEnvelope {
    pub meta: SearchMeta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<SearchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<QueryGroup>>,
}

/// Metadata for search results.
#[derive(Serialize)]
pub struct SearchMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queries: Option<usize>,
    pub total: usize,
    pub returned: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

/// A group of results for a single query in multi-query mode.
#[derive(Serialize)]
pub struct QueryGroup {
    pub query: String,
    pub meta: Meta,
    pub results: Vec<SearchResult>,
}

/// Count-only result for multi-query.
#[derive(Serialize)]
pub struct CountResult {
    pub query: String,
    pub total: usize,
}

/// Count-only envelope for multi-query.
#[derive(Serialize)]
pub struct CountEnvelope {
    pub meta: CountMeta,
    pub counts: Vec<CountResult>,
}

#[derive(Serialize)]
pub struct CountMeta {
    pub queries: usize,
}

impl SearchResult {
    /// Format a single search result in raw mode.
    /// Format: `file:#index ##title (Lline, score:0.XX)`
    pub fn format_raw(&self) -> String {
        let mut parts = vec![self.file.clone()];

        if let Some(ref index) = self.section_index {
            parts.push(format!("{}{}", index, self.section_title.as_ref().map(|t| format!(" {}", t)).unwrap_or_default()));
        }

        parts.push(format!("(L{}, score:{:.2})", self.line, self.score));

        parts.join(":")
    }
}

impl SearchEnvelope {
    /// Create a single-query search envelope.
    pub fn single_query(
        query: String,
        total: usize,
        returned: usize,
        offset: usize,
        limit: usize,
        truncated: bool,
        results: Vec<SearchResult>,
    ) -> Self {
        let has_more = offset + returned < total;
        let next_offset = if has_more { Some(offset + returned) } else { None };

        Self {
            meta: SearchMeta {
                query: Some(query),
                queries: None,
                total,
                returned,
                offset: Some(offset),
                limit: Some(limit),
                truncated,
                has_more: Some(has_more),
                next_offset,
            },
            results: Some(results),
            groups: None,
        }
    }

    /// Create a multi-query search envelope.
    pub fn multi_query(groups: Vec<QueryGroup>) -> Self {
        let total_queries = groups.len();
        let total: usize = groups.iter().map(|g| g.meta.total).sum();

        Self {
            meta: SearchMeta {
                query: None,
                queries: Some(total_queries),
                total,
                returned: 0,
                offset: None,
                limit: None,
                truncated: false,
                has_more: None,
                next_offset: None,
            },
            results: None,
            groups: Some(groups),
        }
    }

    /// Create a count-only envelope for multi-query.
    pub fn count_only(counts: Vec<CountResult>) -> CountEnvelope {
        let queries = counts.len();

        CountEnvelope {
            meta: CountMeta { queries },
            counts,
        }
    }

    /// Format count-only results in raw mode.
    /// Format: `query1: count1\nquery2: count2`
    pub fn format_counts_raw(counts: &[CountResult]) -> String {
        counts
            .iter()
            .map(|c| format!("{}: {}", c.query, c.total))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl QueryGroup {
    /// Create a new query group.
    pub fn new(query: String, meta: Meta, results: Vec<SearchResult>) -> Self {
        Self {
            query,
            meta,
            results,
        }
    }
}

// ---------------------------------------------------------------------------
// Index status
// ---------------------------------------------------------------------------

/// Index status information.
#[derive(Serialize)]
pub struct IndexStatus {
    pub path: String,
    pub last_sync: String,
    pub files: FilesStatus,
    pub size: SizeStatus,
}

/// File counts in index status.
#[derive(Serialize)]
pub struct FilesStatus {
    pub indexed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untracked: Option<usize>,
}

/// Size information in index status.
#[derive(Serialize)]
pub struct SizeStatus {
    #[serde(rename = "sqlite_bytes")]
    pub sqlite: usize,
    #[serde(rename = "tantivy_bytes")]
    pub tantivy: usize,
}

impl IndexStatus {
    /// Format index status as raw output.
    pub fn format_raw(&self) -> String {
        let mut lines = vec![
            format!("db: {} (last sync: {})", self.path, self.last_sync),
            format!(
                "files: {} indexed",
                self.files.indexed
            ),
        ];

        if let Some(stale) = self.files.stale {
            lines.push(format!("stale: {}", stale));
        }
        if let Some(deleted) = self.files.deleted {
            lines.push(format!("deleted: {}", deleted));
        }
        if let Some(untracked) = self.files.untracked {
            lines.push(format!("untracked: {}", untracked));
        }

        lines.push(format!(
            "size: sqlite {:.1}MB, tantivy {:.1}MB",
            self.size.sqlite as f64 / 1_048_576.0,
            self.size.tantivy as f64 / 1_048_576.0
        ));

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// TOC output
// ---------------------------------------------------------------------------

/// Table of contents entry.
#[derive(Serialize)]
pub struct TocEntry {
    pub index: String,
    pub level: u8,
    pub text: String,
    pub line: usize,
}

impl TocEntry {
    /// Format a TOC entry in raw mode.
    /// Format: `1.1.1 ### Heading              (L8)`
    pub fn format_raw(&self) -> String {
        let indent = "  ".repeat(self.level as usize - 1);
        let line_info = format!("(L{})", self.line);
        format!("{}{}{} {:<30} {}", indent, self.index, "   ", self.text, line_info)
    }
}

// ---------------------------------------------------------------------------
// Links output
// ---------------------------------------------------------------------------

/// Link information.
#[derive(Serialize)]
pub struct LinkInfo {
    pub source_file: String,
    pub source_line: usize,
    pub target_raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_anchor: Option<String>,
    pub link_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_file: Option<String>,
    pub is_broken: bool,
}

impl LinkInfo {
    /// Format a link in raw mode.
    /// Format: `source > target wiki L5`
    pub fn format_raw(&self) -> String {
        let target = self.target_path.as_ref().unwrap_or(&self.target_raw);
        format!(
            "{} > {} {} L{}",
            self.source_file,
            target,
            self.link_type,
            self.source_line
        )
    }
}

// ---------------------------------------------------------------------------
// Graph output
// ---------------------------------------------------------------------------

/// Graph adjacency representation.
#[derive(Serialize)]
pub struct GraphAdjacency {
    pub meta: GraphMeta,
    pub graph: BTreeMap<String, NodeInfo>,
}

/// Metadata for graph output.
#[derive(Serialize)]
pub struct GraphMeta {
    pub nodes: usize,
    pub edges: usize,
}

/// Node information with incoming and outgoing edges.
#[derive(Serialize)]
pub struct NodeInfo {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub out: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub r#in: Vec<String>,
}

/// Graph edge list representation.
#[derive(Serialize)]
pub struct GraphEdges {
    pub meta: GraphMeta,
    pub edges: Vec<EdgeInfo>,
}

/// Individual edge information.
#[derive(Serialize)]
pub struct EdgeInfo {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

/// Graph statistics.
#[derive(Serialize)]
pub struct GraphStats {
    pub nodes: usize,
    pub edges: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphans: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_linked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_linking: Option<String>,
}

impl GraphStats {
    /// Format graph stats as raw output.
    pub fn format_raw(&self) -> String {
        let mut lines = vec![
            format!("nodes: {}, edges: {}", self.nodes, self.edges),
        ];

        if let Some(orphans) = self.orphans {
            lines.push(format!("orphans: {}", orphans));
        }
        if let Some(ref most_linked) = self.most_linked {
            lines.push(format!("most linked: {}", most_linked));
        }
        if let Some(ref most_linking) = self.most_linking {
            lines.push(format!("most linking: {}", most_linking));
        }

        lines.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Frontmatter output
// ---------------------------------------------------------------------------

/// Frontmatter entry for a file.
#[derive(Serialize)]
pub struct FrontmatterEntry {
    pub file: String,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, Value>,
}

impl FrontmatterEntry {
    /// Format frontmatter entry in raw mode.
    pub fn format_raw(&self) -> String {
        let mut parts = vec![self.file.clone()];
        for (key, value) in &self.fields {
            let value_str = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => "null".to_string(),
                Value::Array(arr) => {
                    let items: Vec<String> = arr.iter().map(|v| {
                        match v {
                            Value::String(s) => format!("\"{}\"", s),
                            _ => v.to_string(),
                        }
                    }).collect();
                    format!("[{}]", items.join(", "))
                }
                Value::Object(obj) => {
                    let items: Vec<String> = obj.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                    format!("{{{}}}", items.join(", "))
                }
            };
            parts.push(format!("{}={}", key, value_str));
        }
        parts.join(" ")
    }
}

// ---------------------------------------------------------------------------
// Read output
// ---------------------------------------------------------------------------

/// Content read result with metadata.
#[derive(Serialize)]
pub struct ReadResult {
    pub meta: ReadMeta,
    pub content: String,
}

/// Metadata for read results.
#[derive(Serialize)]
pub struct ReadMeta {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_shown: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<usize>,
}

impl ReadResult {
    /// Create a new read result.
    pub fn new(file: String, content: String) -> Self {
        Self {
            meta: ReadMeta {
                file,
                section: None,
                truncated: None,
                bytes_shown: None,
                bytes_total: None,
                next_line: None,
                start_line: None,
                end_line: None,
            },
            content,
        }
    }

    /// Set section information.
    pub fn with_section(mut self, section: String, start_line: usize, end_line: usize) -> Self {
        self.meta.section = Some(section);
        self.meta.start_line = Some(start_line);
        self.meta.end_line = Some(end_line);
        self
    }

    /// Set truncation information.
    pub fn with_truncation(mut self, bytes_shown: usize, bytes_total: usize, next_line: usize) -> Self {
        self.meta.truncated = Some(true);
        self.meta.bytes_shown = Some(bytes_shown);
        self.meta.bytes_total = Some(bytes_total);
        self.meta.next_line = Some(next_line);
        self
    }

    /// Format truncation footer for raw mode.
    /// Format: `--- truncated at L42, 2048/8192 bytes, next: --section "L43-" ---`
    pub fn format_truncation_footer(&self) -> Option<String> {
        if self.meta.truncated.unwrap_or(false) {
            let bytes_shown = self.meta.bytes_shown.unwrap_or(0);
            let bytes_total = self.meta.bytes_total.unwrap_or(0);
            let next_line = self.meta.next_line.unwrap_or(0);

            Some(format!(
                "--- truncated at L{}, {}/{} bytes, next: --section \"L{}-\" ---",
                next_line, bytes_shown, bytes_total, next_line
            ))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Summary output (for --summary flag)
// ---------------------------------------------------------------------------

/// Section summary for `--summary` flag.
#[derive(Serialize)]
pub struct SectionSummary {
    pub index: String,
    pub title: String,
    pub line: usize,
    pub preview: String,
}

/// Summary results envelope.
#[derive(Serialize)]
pub struct SummaryEnvelope {
    pub meta: SummaryMeta,
    pub results: Vec<SectionSummary>,
}

#[derive(Serialize)]
pub struct SummaryMeta {
    pub file: String,
    pub total_sections: usize,
    pub returned: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

impl SectionSummary {
    /// Format a section summary in raw mode.
    /// Format: `## Setup (L5, #1.1)\n  Rust 1.75+ needed.`
    pub fn format_raw(&self) -> String {
        format!(
            "{} (L{}, {})\n  {}",
            self.title, self.line, self.index, self.preview
        )
    }
}

impl SummaryEnvelope {
    /// Create a new summary envelope.
    pub fn new(file: String, total_sections: usize, returned: usize, offset: usize, results: Vec<SectionSummary>) -> Self {
        let has_more = offset + returned < total_sections;
        let next_offset = if has_more { Some(offset + returned) } else { None };

        Self {
            meta: SummaryMeta {
                file,
                total_sections,
                returned,
                offset: Some(offset),
                has_more: Some(has_more),
                next_offset,
            },
            results,
        }
    }

    /// Format summary footer in raw mode.
    /// Format: `--- 3/12 sections shown, next: --offset 3 ---`
    pub fn format_footer(&self) -> String {
        let returned = self.meta.returned;
        let total = self.meta.total_sections;
        let offset = self.meta.offset.unwrap_or(0);

        if offset + returned >= total {
            format!("--- {}/{} sections shown ---", returned, total)
        } else {
            format!("--- {}/{} sections shown, next: --offset {} ---", returned, total, offset + returned)
        }
    }
}

// ---------------------------------------------------------------------------
// Tree output
// ---------------------------------------------------------------------------

/// Tree node for directory structure output.
#[derive(Serialize)]
pub struct TreeNode {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>, // "file" or "dir"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

impl TreeNode {
    /// Format tree node in raw mode with indentation.
    pub fn format_raw(&self, indent: usize, is_last: bool) -> String {
        let prefix = if indent == 0 {
            String::new()
        } else {
            let connector = if is_last { "└── " } else { "├── " };
            "  ".repeat(indent - 1) + connector
        };

        let mut line = format!("{}{}", prefix, self.path);

        if let Some(size) = self.size {
            line.push_str(&format!(" ({} bytes)", size));
        }

        let mut result = line;

        if let Some(ref children) = self.children {
            for (i, child) in children.iter().enumerate() {
                let is_child_last = i == children.len() - 1;
                result.push('\n');
                result.push_str(&child.format_raw(indent + 1, is_child_last));
            }
        }

        result
    }
}
