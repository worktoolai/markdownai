# markdownai

**Agent-first CLI for structured markdown access.**

## Why markdownai?

AI agents working with markdown collections (Obsidian vaults, documentation repos, knowledge bases) face a fundamental problem: **reading entire files wastes tokens**. A 500-line document costs thousands of tokens, but the agent usually needs only one section.

Traditional tools like `cat`, `grep`, `head` are designed for humans. They return raw text without structure, forcing agents to parse headers, count lines, and guess section boundaries on every read. This is slow, error-prone, and expensive.

markdownai solves this by treating markdown documents as **structured, queryable data**:

- **Read only what you need** — address sections by TOC index (`#1.2`), header path (`## Setup > ### Prerequisites`), or line range (`L10-L25`)
- **Search before reading** — full-text search with Tantivy returns exact file, section, and line — no need to scan files
- **Modify surgically** — replace, add, or delete sections by address without touching the rest of the document
- **Zero-cost re-reads** — SQLite + Tantivy index auto-syncs via content hash (xxh3), so repeated queries hit the database, not the filesystem
- **Predictable output** — every command follows the same envelope format with paging, truncation, and byte budgets

The recommended workflow: `toc` → identify section → `read --section` → work on just what matters. This typically reduces token usage by 80-90% compared to reading whole files.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/worktoolai/markdownai/main/install.sh | sh
```

Or build from source:

```bash
cargo install --path .
```

## Quick Reference

```
markdownai <COMMAND> [OPTIONS]

Section address formats:
  "#1.1"                    TOC index (matches toc output)
  "## Parent > ### Child"   Header path
  "L10-L25"                 Line range (1-based, inclusive)

Commands:
  toc <FILE>                Headings with section numbers
  read <FILE>               Content (--section ADDR --summary [N] --meta)
  tree <PATH>               Directory structure
  search <INPUT> -q QUERY   Full-text search (multi -q, --scope, --match)
  frontmatter <INPUT>       YAML fields (--field --filter --facets FIELD)
  links <FILE>              Outgoing links (--broken --resolved)
  backlinks <FILE>          Incoming links
  graph <INPUT>             Link graph (--format adjacency|edges|stats)
  section-set <FILE>        Replace section (-s ADDR -c TEXT)
  section-add <FILE>        Add section (-t TITLE -c TEXT --after/--before)
  section-delete <FILE>     Delete section (-s ADDR)
  frontmatter-set <FILE>    Set YAML field (-k KEY -v VALUE)
  index <PATH>              DB management (--status --force --check)

Global flags:
  --json                    JSON envelope output (default: raw markdown)
  --pretty                  Pretty-print JSON
  --max-bytes <N>           Truncate output to byte budget
  --limit <N>               Max result items (default: 20)
  --offset <N>              Result start position for paging
  --count-only              Return count only
  --exists                  Check existence only (exit code)
  --stats                   Size/structure statistics only
  --plan                    Metadata only, no results
  --sync auto|force         Sync mode (default: auto)
  --root <DIR>              Project root override

Exit codes: 0=ok  1=not-found  2=error
Input: file, directory (recursive .md), "-" (stdin)
```

## Usage

### Explore document structure

```bash
# View table of contents with section numbers
markdownai toc docs/guide.md

# Output:
# 1   # Guide                          (L1)
# 1.1 ## Setup                         (L5)
# 1.1.1 ### Prerequisites              (L8)
# 1.2 ## Usage                         (L20)

# Preview all sections (first 3 lines each)
markdownai read docs/guide.md --summary

# Read a specific section by TOC index
markdownai read docs/guide.md --section "#1.1"

# Read by header path
markdownai read docs/guide.md --section "## Setup > ### Prerequisites"

# Read by line range
markdownai read docs/guide.md --section "L10-L25"
```

### Search across documents

```bash
# Full-text search
markdownai search ./docs -q "authentication"

# Search with scope and match mode
markdownai search ./docs -q "OAuth" --scope headers --match exact

# Multi-query search
markdownai search ./docs -q "OAuth" -q "JWT"

# Count matches only
markdownai search ./docs -q "OAuth" --count-only

# Check if matches exist (exit code only)
markdownai search ./docs -q "OAuth" --exists
```

### Query frontmatter

```bash
# List all frontmatter fields
markdownai frontmatter ./docs --list

# Filter by field value
markdownai frontmatter ./docs --filter 'tags contains "rust"'

# Get facet distribution
markdownai frontmatter ./docs --facets tags

# Extract specific field
markdownai frontmatter docs/guide.md --field title
```

### Analyze links

```bash
# Outgoing links from a file
markdownai links docs/index.md

# Find broken links
markdownai links docs/index.md --broken

# Who links to this file?
markdownai backlinks docs/auth.md

# Visualize link graph
markdownai graph ./docs --format stats
markdownai graph ./docs --start docs/index.md --depth 2
```

### Modify documents

```bash
# Replace section content
markdownai section-set docs/guide.md -s "#1.1" -c "New content here"

# Replace with content from file
markdownai section-set docs/guide.md -s "#1.1" --content-file patch.md

# Replace with content from stdin
echo "Updated content" | markdownai section-set docs/guide.md -s "#1.1" --content -

# Add a new section after an existing one
markdownai section-add docs/guide.md -t "Troubleshooting" -c "Common issues..." --after "#1.2" --level 2

# Delete a section
markdownai section-delete docs/guide.md -s "#1.1.1"

# Preview changes without writing
markdownai section-set docs/guide.md -s "#1.1" -c "New content" --dry-run

# Set frontmatter field (auto-creates frontmatter if missing)
markdownai frontmatter-set docs/guide.md -k tags -v '["rust", "cli"]'
```

### Manage index

```bash
# Build/update index for a directory
markdownai index ./docs

# Force full rebuild
markdownai index ./docs --force

# Check index status
markdownai index ./docs --status

# Verify SQLite <-> Tantivy consistency
markdownai index ./docs --check
```

### Control output

```bash
# JSON output
markdownai toc docs/guide.md --json

# Pretty JSON
markdownai toc docs/guide.md --json --pretty

# Limit results with paging
markdownai search ./docs -q "test" --limit 5 --offset 10

# Byte budget (truncates to fit)
markdownai read docs/guide.md --max-bytes 2048

# File stats without reading content
markdownai read docs/guide.md --stats

# Directory stats
markdownai read ./docs --stats
```

### Stdin support

```bash
# Pipe from git
git show HEAD:docs/guide.md | markdownai toc -
git show HEAD~1:docs/guide.md | markdownai read - --section "#1.1"

# Pipe between commands
curl -s https://example.com/doc.md | markdownai frontmatter -
```

## Section Address System

Three ways to address sections:

| Format | Example | Use case |
|--------|---------|----------|
| TOC index | `"#1.1"` | Stable reference from `toc` output |
| Header path | `"## Setup > ### Prerequisites"` | Human-readable |
| Line range | `"L10-L25"` | Precise byte-level access |

TOC indices are assigned by appearance order, not by header text. Duplicate header names get different indices:

```
## FAQ          → #1.1
### Question    → #1.1.1
### Question    → #1.1.2    ← same name, different index
## API          → #1.2
```

After modifying a document, use `--with-toc` to get updated indices in the response.

## Recommended Agent Workflow

```
1. toc FILE                    → discover structure
2. read FILE --summary         → preview all sections
3. read FILE --section "#N.M"  → read specific section
4. search DIR -q "keyword"     → find across files
5. section-set / section-add   → modify with --dry-run first
```

This pattern minimizes token usage by reading only what's needed at each step.

## Output Modes

**Raw (default)** — plain markdown with a paging footer:

```
## Setup

Rust 1.75+ required...

--- 10/47 shown, next: --offset 10 ---
```

**JSON (`--json`)** — structured envelope with metadata:

```json
{
  "meta": {"total": 47, "returned": 10, "offset": 0, "has_more": true, "next_offset": 10},
  "results": [...]
}
```

**Overflow protection** — when results exceed `--threshold` (default 50), the tool returns a plan instead of flooding the context:

```json
{
  "meta": {"total": 350, "overflow": true},
  "plan": {"suggestion": "add --scope headers or narrow query"}
}
```

## Storage

Index files are stored in `.worktoolai/` at the project root (auto-detected via `.git/`):

```
.worktoolai/
  markdownai.db          ← SQLite (metadata, sections, links, frontmatter)
  markdownai_index/      ← Tantivy (full-text search index)
```

Sync is automatic — content hashes (xxh3) detect changes regardless of timestamps. Works correctly across git clone, rsync, and NFS.

## License

MIT
