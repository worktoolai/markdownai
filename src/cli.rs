use clap::{Args, Parser, Subcommand, ValueEnum};

const VERSION: &str = env!("GIT_VERSION");

#[derive(Parser)]
#[command(
    name = "markdownai",
    version = VERSION,
    about = "Agent-first Markdown CLI (auto-syncs DB, raw output default)",
    after_help = r####"Section: "#1.1" (toc index) | "## Head > ### Sub" (path) | "L10-L25" (lines)
  toc FILE                 headings with section numbers
  read FILE                content (--section ADDR --summary [N] --meta)
  tree PATH                directory structure
  search INPUT -q QUERY    full-text (multi -q, --scope, --match, --context)
  frontmatter INPUT        YAML fields (--field --filter --facets FIELD)
  overview INPUT            file overview with frontmatter + structure metadata
  links FILE               outgoing links (--broken --resolved)
  backlinks FILE           incoming links
  graph INPUT              link graph (--format adjacency|edges|stats)
  section-set FILE -s ADDR replace section (-c TEXT | --content-file F | --content -)
  section-add FILE -t HDR  add section (--after --before --level)
  section-delete FILE -s ADDR
  frontmatter-set FILE -k KEY -v VAL
  renum FILE                renumber heading numbers sequentially
  chars INPUT               Unicode script character statistics
  index PATH               DB management (--status --force --check)
Flags: --json --max-bytes N --limit N --offset N --count-only --exists --stats
Exit: 0=ok 1=not-found 2=error | Input: file, dir (recursive .md), "-" (stdin)"####
)]
pub struct Cli {
    /// Output as JSON envelope instead of raw markdown
    #[arg(long, global = true)]
    pub json: bool,

    /// Pretty-print JSON output (only with --json)
    #[arg(long, global = true)]
    pub pretty: bool,

    /// Max output bytes (truncate to fit)
    #[arg(long, global = true)]
    pub max_bytes: Option<usize>,

    /// Max result items
    #[arg(long, global = true, default_value_t = 20)]
    pub limit: usize,

    /// Result offset (paging)
    #[arg(long, global = true, default_value_t = 0)]
    pub offset: usize,

    /// Overflow threshold; results exceeding this trigger plan mode
    #[arg(long, global = true, default_value_t = 50)]
    pub threshold: usize,

    /// Bypass overflow protection
    #[arg(long, global = true)]
    pub no_overflow: bool,

    /// Force plan mode: metadata only, no results
    #[arg(long, global = true)]
    pub plan: bool,

    /// Return count only, no result body
    #[arg(long, global = true)]
    pub count_only: bool,

    /// Check existence only (exit code 0=exists, 1=not)
    #[arg(long, global = true)]
    pub exists: bool,

    /// Return size/structure stats only
    #[arg(long, global = true)]
    pub stats: bool,

    /// Return facet distribution for a field
    #[arg(long, global = true)]
    pub facets: Option<String>,

    /// Sync mode
    #[arg(long, global = true, value_enum, default_value_t = SyncMode::Auto)]
    pub sync: SyncMode,

    /// Project root override
    #[arg(long, global = true)]
    pub root: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Clone, ValueEnum)]
pub enum SyncMode {
    Auto,
    Force,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Headings with section numbers
    #[command(after_help = r#"  toc doc.md                        # full toc
  toc doc.md --depth 2              # h1+h2 only
  toc doc.md --flat                 # no indentation"#)]
    Toc(TocArgs),

    /// Read file or section
    #[command(after_help = r####"  read doc.md                        # full file
  read doc.md --section "#1.1"       # toc number / "## Setup" / "L10-L25"
  read doc.md --summary              # first 3 lines per section
  read doc.md --stats                # size/structure only"####)]
    Read(ReadArgs),

    /// Directory structure
    #[command(after_help = r#"  tree ./docs                        # full tree
  tree ./docs --depth 2              # limit depth
  tree ./docs --files-only           # files only"#)]
    Tree(TreeArgs),

    /// Full-text search
    #[command(after_help = r#"  search ./docs -q "OAuth"           # basic search
  search ./docs -q "OAuth" -q "JWT"  # multi query
  search ./docs -q "auth" --scope headers"#)]
    Search(SearchArgs),

    /// YAML frontmatter fields
    #[command(after_help = r#"  frontmatter doc.md                 # all fields
  frontmatter ./docs --field tags    # specific field
  frontmatter ./docs --facets tags   # value distribution"#)]
    Frontmatter(FrontmatterArgs),

    /// File overview with frontmatter and structure metadata
    #[command(after_help = r#"  overview ./docs                     # all files
  overview ./docs --field title --field tags
  overview ./docs --filter 'status == "published"'
  overview ./docs --sort title"#)]
    Overview(OverviewArgs),

    /// Outgoing links
    #[command(after_help = r#"  links doc.md                       # all links
  links doc.md --broken              # broken only
  links doc.md --type wiki           # wiki links only"#)]
    Links(LinksArgs),

    /// Incoming links (backlinks)
    #[command(after_help = r#"  backlinks doc.md                   # who links here"#)]
    Backlinks(BacklinksArgs),

    /// Link graph
    #[command(after_help = r#"  graph ./docs                       # full graph
  graph ./docs --format stats        # stats only
  graph ./docs --format orphans      # find orphan files
  graph ./docs --start index.md --depth 2"#)]
    Graph(GraphArgs),

    /// Replace section content
    #[command(name = "section-set", after_help = r####"  section-set doc.md -s "#1.1" -c "New content"
  echo "content" | section-set doc.md -s "## Setup" --content -"####)]
    SectionSet(SectionSetArgs),

    /// Replace entire section (heading + body)
    #[command(name = "section-replace", after_help = r####"  section-replace doc.md -s "#1.1" -c "## New Title\n\nNew body"
  section-replace doc.md -s "## Old" --content-file new_section.md"####)]
    SectionReplace(SectionReplaceArgs),

    /// Write content to file
    #[command(name = "write")]
    Write(WriteArgs),

    /// Add new section
    #[command(name = "section-add", after_help = r####"  section-add doc.md -t "## New Section" -c "Content"
  section-add doc.md -t "### Sub" --after "#1.1""####)]
    SectionAdd(SectionAddArgs),

    /// Delete section
    #[command(name = "section-delete", after_help = r####"  section-delete doc.md -s "#1.1"
  section-delete doc.md -s "## Old Section""####)]
    SectionDelete(SectionDeleteArgs),

    /// Set frontmatter field
    #[command(name = "frontmatter-set", after_help = r#"  frontmatter-set doc.md -k tags -v '["rust","cli"]'
  frontmatter-set doc.md -k draft -v true"#)]
    FrontmatterSet(FrontmatterSetArgs),

    /// Renumber heading numbers sequentially
    #[command(after_help = r#"  renum doc.md                       # renumber all
  renum doc.md --dry-run             # preview changes"#)]
    Renum(RenumArgs),

    /// Unicode script character statistics
    #[command(after_help = r#"  chars doc.md                       # single file
  chars ./docs                       # directory (per-file)
  echo "text" | chars -              # stdin"#)]
    Chars(CharsArgs),

    /// DB management
    #[command(after_help = r#"  index ./docs                       # sync
  index ./docs --force               # full rebuild
  index ./docs --status              # current status"#)]
    Index(IndexArgs),

    /// Query frontmatter fields across files
    #[command(name = "frontmatter-query", after_help = r#"  frontmatter-query ./docs --field tags
  frontmatter-query ./docs --field tags,title --json
  frontmatter-query ./docs --field tags --filter 'tags contains "rust"'
  frontmatter-query ./docs --field status --count-only"#)]
    FrontmatterQuery(FrontmatterQueryArgs),
}

// ---------- toc ----------
#[derive(Parser)]
pub struct TocArgs {
    /// File path or "-" for stdin
    pub file: String,

    /// Max heading depth (1-6)
    #[arg(long)]
    pub depth: Option<u8>,

    /// Flat output (no indentation)
    #[arg(long)]
    pub flat: bool,
}

// ---------- read ----------
#[derive(Parser)]
pub struct ReadArgs {
    /// File path or "-" for stdin
    pub file: String,

    /// Section address: "#1.1", "## Heading", "L10-L25"
    #[arg(short, long)]
    pub section: Option<String>,

    /// Preview first N lines per section (default 3)
    #[arg(long, num_args = 0..=1, default_missing_value = "3")]
    pub summary: Option<usize>,

    /// Include frontmatter in output
    #[arg(long)]
    pub meta: bool,
}

// ---------- tree ----------
#[derive(Parser)]
pub struct TreeArgs {
    /// Directory path
    pub path: String,

    /// Max depth
    #[arg(long)]
    pub depth: Option<usize>,

    /// Show files only (no directories)
    #[arg(long)]
    pub files_only: bool,

    /// Show count only
    #[arg(long)]
    pub count: bool,
}

// ---------- search ----------
#[derive(Parser)]
pub struct SearchArgs {
    /// Input: file, directory, or "-" for stdin
    pub input: String,

    /// Search query (repeatable for multi-query)
    #[arg(short, long, required = true)]
    pub query: Vec<String>,

    /// Match mode
    #[arg(short, long, value_enum, default_value_t = MatchMode::Text)]
    pub r#match: MatchMode,

    /// Search scope
    #[arg(long, value_enum, default_value_t = SearchScope::All)]
    pub scope: SearchScope,

    /// Context lines around match
    #[arg(long, default_value_t = 0)]
    pub context: usize,

    /// Output bare results (no envelope)
    #[arg(long)]
    pub bare: bool,
}

#[derive(Clone, ValueEnum)]
pub enum MatchMode {
    Text,
    Exact,
    Fuzzy,
    Regex,
}

#[derive(Clone, ValueEnum)]
pub enum SearchScope {
    All,
    Body,
    Headers,
    Frontmatter,
    Code,
}

// ---------- frontmatter ----------
#[derive(Parser)]
pub struct FrontmatterArgs {
    /// Input: file or directory
    pub input: String,

    /// Specific field to extract
    #[arg(long)]
    pub field: Option<String>,

    /// Filter expression (e.g., 'tags contains "rust"')
    #[arg(long)]
    pub filter: Option<String>,

    /// List all unique keys
    #[arg(long)]
    pub list: bool,
}

// ---------- overview ----------
#[derive(Parser)]
pub struct OverviewArgs {
    /// Input: file or directory
    pub input: String,
    /// Frontmatter fields to include (repeatable; omit for all)
    #[arg(long)]
    pub field: Vec<String>,
    /// Filter expression (e.g., 'tags contains "rust"')
    #[arg(long)]
    pub filter: Option<String>,
    /// Sort by field name or "name"/"lines"/"sections"
    #[arg(long)]
    pub sort: Option<String>,
    /// Reverse sort order
    #[arg(long)]
    pub reverse: bool,
}

// ---------- links ----------
#[derive(Parser)]
pub struct LinksArgs {
    /// File path
    pub file: String,

    /// Link type filter
    #[arg(long, value_enum)]
    pub r#type: Option<LinkType>,

    /// Show only resolved links
    #[arg(long)]
    pub resolved: bool,

    /// Show only broken links
    #[arg(long)]
    pub broken: bool,
}

#[derive(Clone, ValueEnum)]
pub enum LinkType {
    Wiki,
    Markdown,
    All,
}

// ---------- backlinks ----------
#[derive(Parser)]
pub struct BacklinksArgs {
    /// File path
    pub file: String,
}

// ---------- graph ----------
#[derive(Parser)]
pub struct GraphArgs {
    /// Input: file or directory
    pub input: String,

    /// Output format
    #[arg(long, value_enum, default_value_t = GraphFormat::Adjacency)]
    pub format: GraphFormat,

    /// Start node for subgraph
    #[arg(long)]
    pub start: Option<String>,

    /// Traversal depth
    #[arg(long)]
    pub depth: Option<usize>,

}

#[derive(Clone, ValueEnum)]
pub enum GraphFormat {
    /// Node → neighbors list
    Adjacency,
    /// Flat from → to edge list
    Edges,
    /// Summary counts and top nodes
    Stats,
    /// Files with no incoming links
    Orphans,
}

// ---------- section-set ----------
#[derive(Parser)]
pub struct SectionSetArgs {
    /// File path
    pub file: String,

    /// Section address
    #[arg(short, long)]
    pub section: String,

    /// Inline content
    #[arg(short, long)]
    pub content: Option<String>,

    /// Content from file
    #[arg(long)]
    pub content_file: Option<String>,

    /// Dry run (show changes, no write)
    #[arg(long)]
    pub dry_run: bool,

    /// Write to different file
    #[arg(long)]
    pub output: Option<String>,

    /// Include updated toc in response
    #[arg(long)]
    pub with_toc: bool,
}

// ---------- section-replace ----------
#[derive(Parser)]
pub struct SectionReplaceArgs {
    /// File path
    pub file: String,

    /// Section address to replace
    #[arg(short, long)]
    pub section: String,

    /// Inline content (must start with heading)
    #[arg(short, long)]
    pub content: Option<String>,

    /// Content from file
    #[arg(long)]
    pub content_file: Option<String>,

    /// Dry run (show changes, no write)
    #[arg(long)]
    pub dry_run: bool,

    /// Write to different file
    #[arg(long)]
    pub output: Option<String>,

    /// Include updated toc in response
    #[arg(long)]
    pub with_toc: bool,
}

// ---------- write ----------
#[derive(Args)]
#[command(name = "write")]
pub struct WriteArgs {
    /// Target file path
    pub file: String,

    /// Inline content
    #[arg(short, long)]
    pub content: Option<String>,

    /// Read content from file (use - for stdin)
    #[arg(long)]
    pub content_file: Option<String>,

    /// Preview without writing
    #[arg(long)]
    pub dry_run: bool,
}

// ---------- section-add ----------
#[derive(Parser)]
pub struct SectionAddArgs {
    /// File path
    pub file: String,

    /// Section title (e.g., "## New Section")
    #[arg(short, long)]
    pub title: String,

    /// Inline content
    #[arg(short, long)]
    pub content: Option<String>,

    /// Content from file
    #[arg(long)]
    pub content_file: Option<String>,

    /// Insert after this section address
    #[arg(long)]
    pub after: Option<String>,

    /// Insert before this section address
    #[arg(long)]
    pub before: Option<String>,

    /// Heading level (1-6)
    #[arg(long)]
    pub level: Option<u8>,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub output: Option<String>,

    #[arg(long)]
    pub with_toc: bool,
}

// ---------- section-delete ----------
#[derive(Parser)]
pub struct SectionDeleteArgs {
    /// File path
    pub file: String,

    /// Section address to delete
    #[arg(short, long)]
    pub section: String,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub output: Option<String>,

    #[arg(long)]
    pub with_toc: bool,
}

// ---------- frontmatter-set ----------
#[derive(Parser)]
pub struct FrontmatterSetArgs {
    /// File path
    pub file: String,

    /// Key name
    #[arg(short, long)]
    pub key: String,

    /// Value (JSON or plain text)
    #[arg(short, long)]
    pub value: String,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long)]
    pub output: Option<String>,

    #[arg(long)]
    pub with_toc: bool,
}

// ---------- renum ----------
#[derive(Parser)]
pub struct RenumArgs {
    /// File path
    pub file: String,

    /// Dry run (show changes, no write)
    #[arg(long)]
    pub dry_run: bool,

    /// Write to different file
    #[arg(long)]
    pub output: Option<String>,
}

// ---------- chars ----------
#[derive(Parser)]
pub struct CharsArgs {
    /// Input: file, directory, or "-" for stdin
    pub input: String,
}

// ---------- index ----------
#[derive(Parser)]
pub struct IndexArgs {
    /// Path to index
    pub path: String,

    /// Full rebuild (delete DB first)
    #[arg(long)]
    pub force: bool,

    /// Show status only
    #[arg(long)]
    pub status: bool,

    /// Dry run: show what would change
    #[arg(long)]
    pub dry_run: bool,

    /// Check SQLite <-> Tantivy consistency
    #[arg(long)]
    pub check: bool,
}

// ---------- frontmatter-query ----------
#[derive(Parser)]
pub struct FrontmatterQueryArgs {
    /// Target directory
    pub path: String,

    /// Frontmatter field(s) to extract (comma-separated)
    #[arg(long)]
    pub field: String,

    /// Filter expression (e.g., 'tags contains "rust"')
    #[arg(long)]
    pub filter: Option<String>,
}
