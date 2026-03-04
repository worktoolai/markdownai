#![allow(dead_code)]
//! Section address parsing and section content reading/manipulation.
//!
//! This module provides functionality for:
//! - Parsing section addresses in three formats: index, header path, and line range
//! - Reading section content from markdown files
//! - Manipulating section content (replace, insert, delete)
//!
//! # Section Address Formats
//!
//! 1. **Index**: `"#1.1"` - TOC number, 1:1 correspondence with TOC output
//! 2. **Header Path**: `"## Parent > ### Child"` - human-readable, matches by title text
//! 3. **Line Range**: `"L10-L25"` - precise line range (1-based, inclusive)
//!
//! # Example
//!
//! ```ignore
//! use markdownai::section::{parse_section_address, read_section_content};
//!
//! let addr = parse_section_address("#1.1")?;
//! let content = read_section_content(file_content, &section)?;
//! ```

use crate::markdown::Section;
use anyhow::{bail, Context, Result};

/// Represents a parsed section address.
///
/// Section addresses can be specified in three formats:
/// - `Index`: TOC number like "#1.1" or "#1.2.3"
/// - `HeaderPath`: Hierarchical path like "## Parent > ### Child"
/// - `LineRange`: Line numbers like "L10-L25" (1-based, inclusive)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionAddress {
    /// TOC index like "#1.1" or "#1.2.3"
    Index(String),
    /// Header path like ["Parent", "Child"] for "## Parent > ### Child"
    HeaderPath(Vec<String>),
    /// Line range (start, end) both 1-based and inclusive
    LineRange(usize, usize),
}

impl SectionAddress {
    /// Returns true if this address refers to a line range.
    pub fn is_line_range(&self) -> bool {
        matches!(self, SectionAddress::LineRange(_, _))
    }

    /// Returns true if this address refers to a TOC index.
    pub fn is_index(&self) -> bool {
        matches!(self, SectionAddress::Index(_))
    }

    /// Returns true if this address refers to a header path.
    pub fn is_header_path(&self) -> bool {
        matches!(self, SectionAddress::HeaderPath(_))
    }
}

impl std::fmt::Display for SectionAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionAddress::Index(idx) => write!(f, "{}", idx),
            SectionAddress::HeaderPath(parts) => {
                write!(f, "{}", parts.join(" > "))
            }
            SectionAddress::LineRange(start, end) => write!(f, "L{}-L{}", start, end),
        }
    }
}

/// Preview of a section's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionPreview {
    /// TOC index like "1.1"
    pub index: String,
    /// Section title
    pub title: String,
    /// Line number where section starts
    pub line: usize,
    /// Preview of the first N lines of content
    pub preview: String,
}

impl SectionPreview {
    /// Create a new section preview.
    pub fn new(index: String, title: String, line: usize, preview: String) -> Self {
        SectionPreview {
            index,
            title,
            line,
            preview,
        }
    }
}

/// Parse a section address string into a `SectionAddress` enum.
///
/// Supports three formats:
/// - Index: `"#1.1"` or `"1.1"` (optional leading #)
/// - Header path: `"## Parent > ### Child"` or `"Parent > Child"`
/// - Line range: `"L10-L25"` or `"10-25"` (L prefix optional)
///
/// # Examples
///
/// ```ignore
/// let addr = parse_section_address("#1.1")?;      // Index("1.1")
/// let addr = parse_section_address("## A > ### B")?; // HeaderPath(vec!["A", "B"])
/// let addr = parse_section_address("L10-L25")?;    // LineRange(10, 25)
/// ```
///
/// # Errors
///
/// Returns an error if the address format is not recognized or invalid.
pub fn parse_section_address(addr: &str) -> Result<SectionAddress> {
    let addr = addr.trim();

    if addr.is_empty() {
        bail!("Section address cannot be empty");
    }

    // Try line range first (most distinct pattern)
    if let Ok(range) = try_parse_line_range(addr) {
        return Ok(range);
    }

    // Try index pattern
    if let Ok(index) = try_parse_index(addr) {
        return Ok(SectionAddress::Index(index));
    }

    // Try header path pattern
    if let Ok(path) = try_parse_header_path(addr) {
        return Ok(SectionAddress::HeaderPath(path));
    }

    bail!(
        "Invalid section address format: '{}'. Expected: #1.1, ## Title > ### Subtitle, or L10-L25",
        addr
    )
}

/// Try to parse a line range address.
///
/// Formats: "L10-L25", "L10-25", "10-25"
fn try_parse_line_range(addr: &str) -> Result<SectionAddress> {
    // Match patterns: L10-L25, L10-25, 10-25
    // The dash is a hyphen-minus (U+002D), not an en dash
    let range_re = regex::Regex::new(r"^L?(\d+)-L?(\d+)$").unwrap();

    if let Some(caps) = range_re.captures(addr) {
        let start: usize = caps[1]
            .parse()
            .context("Invalid start line number in range")?;
        let end: usize = caps[2].parse().context("Invalid end line number in range")?;

        if start == 0 {
            bail!("Line numbers are 1-based, got start=0");
        }

        if end == 0 {
            bail!("Line numbers are 1-based, got end=0");
        }

        if start > end {
            bail!("Start line ({}) cannot be greater than end line ({})", start, end);
        }

        return Ok(SectionAddress::LineRange(start, end));
    }

    // Try single line format: L10 or 10
    let single_re = regex::Regex::new(r"^L?(\d+)$").unwrap();
    if let Some(caps) = single_re.captures(addr) {
        let line: usize = caps[1].parse().context("Invalid line number")?;
        if line == 0 {
            bail!("Line numbers are 1-based, got 0");
        }
        return Ok(SectionAddress::LineRange(line, line));
    }

    bail!("Not a valid line range format")
}

/// Try to parse an index address.
///
/// Formats: "#1.1", "#1.1.2", "1.1", "1.1.2"
fn try_parse_index(addr: &str) -> Result<String> {
    let stripped = addr.trim_start_matches('#');

    // Must match pattern like "1", "1.1", "1.2.3", etc.
    let index_re = regex::Regex::new(r"^\d+(?:\.\d+)*$").unwrap();

    if !index_re.is_match(stripped) {
        bail!("Not a valid index format");
    }

    // Validate no leading zeros in segments
    for segment in stripped.split('.') {
        if segment.len() > 1 && segment.starts_with('0') {
            bail!("Index segments should not have leading zeros: {}", segment);
        }
    }

    // Preserve '#' prefix to match Section.index format (e.g., "#1.1")
    Ok(format!("#{}", stripped))
}

/// Try to parse a header path address.
///
/// Formats: "## Title", "## Parent > ### Child", "Title > Child"
fn try_parse_header_path(addr: &str) -> Result<Vec<String>> {
    // Split by " > " to get path components
    let parts: Vec<&str> = addr.split(" > ").collect();

    if parts.is_empty() {
        bail!("Empty header path");
    }

    let mut result = Vec::new();

    for part in parts {
        let part = part.trim();

        // Remove leading # marks and whitespace
        let title = part.trim_start_matches('#').trim();

        if title.is_empty() {
            bail!("Empty title in header path");
        }

        result.push(title.to_string());
    }

    Ok(result)
}

/// Read the content of a section from file content.
///
/// Returns the full section content including the heading line.
/// Lines are 1-based as per the plan specification.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `section` - The section to read
///
/// # Examples
///
/// ```ignore
/// let section = Section::new("1.1".to_string(), 2, "Title".to_string(), 5, 20, 0, None);
/// let text = read_section_content(file_content, &section)?;
/// ```
pub fn read_section_content(content: &str, section: &Section) -> Result<String> {
    read_section_lines(content, section.start_line, section.end_line)
}

/// Read a specific range of lines from content.
///
/// Lines are 1-based (the first line is line 1, not 0).
/// The range is inclusive: both start and end lines are included.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `start` - 1-based start line number (inclusive)
/// * `end` - 1-based end line number (inclusive)
///
/// # Examples
///
/// ```ignore
/// // Read lines 10 through 25 (inclusive)
/// let text = read_section_lines(content, 10, 25)?;
/// ```
pub fn read_section_lines(content: &str, start: usize, end: usize) -> Result<String> {
    if start == 0 {
        bail!("Line numbers are 1-based, got start=0");
    }

    if end == 0 {
        bail!("Line numbers are 1-based, got end=0");
    }

    if start > end {
        bail!(
            "Start line ({}) cannot be greater than end line ({})",
            start,
            end
        );
    }

    let lines: Vec<&str> = content.split('\n').collect();

    if start > lines.len() {
        bail!(
            "Start line {} exceeds file length ({} lines)",
            start,
            lines.len()
        );
    }

    // Convert to 0-based indexing for array access
    let start_idx = start - 1;
    let end_idx = end.min(lines.len());

    let selected_lines = &lines[start_idx..end_idx];

    // Join with newlines, preserving original line endings
    let result = selected_lines.join("\n");

    Ok(result)
}

/// Read a preview of sections (first N lines of each section's content).
///
/// This function is used for the `--summary` flag in the `read` command.
/// It returns previews for all sections in the provided slice.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `sections` - Slice of sections to preview
/// * `preview_lines` - Number of content lines to include per section (excluding heading)
///
/// # Examples
///
/// ```ignore
/// let previews = read_summary(content, &sections, 3)?;
/// for preview in previews {
///     println!("{}: {} - {}", preview.index, preview.title, preview.preview);
/// }
/// ```
pub fn read_summary(
    content: &str,
    sections: &[Section],
    preview_lines: usize,
) -> Result<Vec<SectionPreview>> {
    let lines: Vec<&str> = content.split('\n').collect();

    let mut previews = Vec::new();

    for section in sections {
        // Get preview lines (skip the heading line itself)
        let content_start = section.start_line; // 0-based after -1
        let content_end = (section.start_line + preview_lines).min(section.end_line);

        // Convert to 0-based for array access
        let start_idx = content_start.saturating_sub(1);
        let end_idx = content_end.saturating_sub(1).min(lines.len());

        let preview_text = if start_idx < lines.len() && start_idx < end_idx {
            lines[start_idx..end_idx].join("\n")
        } else {
            String::new()
        };

        previews.push(SectionPreview::new(
            section.index.clone(),
            section.title.clone(),
            section.start_line,
            preview_text,
        ));
    }

    Ok(previews)
}

/// Replace the content of a section with new content.
///
/// The heading line is preserved, only the content after the heading is replaced.
/// If the section is empty (only a heading), the new content is added after it.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `section` - The section whose content should be replaced
/// * `new_content` - The new content to insert (without the heading)
///
/// # Returns
///
/// The modified file content.
///
/// # Examples
///
/// ```ignore
/// let new_content = "This is the new content.\nWith multiple lines.";
/// let updated = replace_section_content(original, &section, new_content)?;
/// ```
pub fn replace_section_content(
    content: &str,
    section: &Section,
    new_content: &str,
) -> Result<String> {
    let lines: Vec<&str> = content.split('\n').collect();

    if section.start_line == 0 || section.start_line > lines.len() {
        bail!(
            "Invalid section start line: {} (file has {} lines)",
            section.start_line,
            lines.len()
        );
    }

    // Split into: before section, heading, old content, after section
    // Convert to 0-based
    let heading_end = section.start_line; // 0-based, first line of content
    let content_end = section.end_line.min(lines.len());

    let before: String = lines[..heading_end.saturating_sub(1)].join("\n");
    let heading: String = if heading_end > 0 {
        lines[heading_end - 1].to_string()
    } else {
        String::new()
    };
    let after: String = if content_end < lines.len() {
        lines[content_end..].join("\n")
    } else {
        String::new()
    };

    // Reject content that contains markdown headings — use section-add for new sections
    for (i, line) in new_content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') && trimmed.contains(' ') {
            let hashes: String = trimmed.chars().take_while(|&c| c == '#').collect();
            if hashes.len() <= 6 {
                bail!(
                    "Content must not contain headings (line {}: \"{}\"). Use section-add to create new sections.",
                    i + 1,
                    line
                );
            }
        }
    }

    // Build new content
    let mut result = String::new();

    if !before.is_empty() {
        result.push_str(&before);
        result.push('\n');
    }

    if !heading.is_empty() {
        result.push_str(&heading);
        result.push('\n');
    }

    if !new_content.is_empty() {
        result.push_str(new_content);

        // Ensure trailing newline if content doesn't have one
        if !new_content.ends_with('\n') && !after.is_empty() {
            result.push('\n');
        }
    }

    if !after.is_empty() {
        result.push_str(&after);
    }

    Ok(result)
}

/// Replace an entire section (heading + body) with new content.
pub fn replace_section_full(
    content: &str,
    section: &Section,
    new_content: &str,
) -> Result<String> {
    let lines: Vec<&str> = content.split('\n').collect();

    if section.start_line == 0 || section.start_line > lines.len() {
        bail!(
            "Invalid section start line: {} (file has {} lines)",
            section.start_line,
            lines.len()
        );
    }

    let content_end = section.end_line.min(lines.len());

    // before = everything before the heading line
    let before: String = lines[..section.start_line.saturating_sub(1)].join("\n");
    // after = everything after the section
    let after: String = if content_end < lines.len() {
        lines[content_end..].join("\n")
    } else {
        String::new()
    };

    let mut result = String::new();

    if !before.is_empty() {
        result.push_str(&before);
        result.push('\n');
    }

    result.push_str(new_content);
    if !new_content.ends_with('\n') && !after.is_empty() {
        result.push('\n');
    }

    if !after.is_empty() {
        result.push_str(&after);
    }

    Ok(result)
}

/// Insert a new section at the specified position.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `position_line` - 1-based line number where to insert (0 = start of file)
/// * `title` - The heading title (e.g., "## New Section" or just "New Section")
/// * `body` - The section body content
///
/// # Returns
///
/// The modified file content with the new section inserted.
///
/// # Examples
///
/// ```ignore
/// // Insert at line 10 (after line 9)
/// let updated = insert_section(content, 10, "## New Section", "Body content")?;
///
/// // Insert at start of file
/// let updated = insert_section(content, 0, "# Title", "Initial content")?;
/// ```
pub fn insert_section(
    content: &str,
    position_line: usize,
    title: &str,
    body: &str,
) -> Result<String> {
    let lines: Vec<&str> = content.split('\n').collect();

    if position_line > lines.len() {
        bail!(
            "Position line {} exceeds file length ({} lines)",
            position_line,
            lines.len()
        );
    }

    // Normalize title: ensure it has # prefix
    let heading = if title.starts_with('#') {
        title.to_string()
    } else {
        // Default to ## for new sections
        format!("## {}", title)
    };

    // Build the new section text
    let mut new_section = String::new();

    // Add blank line before if not at start and previous line is not blank
    if position_line > 0 && !lines.is_empty() {
        let prev_line_idx = position_line.saturating_sub(1);
        if prev_line_idx < lines.len() && !lines[prev_line_idx].trim().is_empty() {
            new_section.push('\n');
        }
    }

    new_section.push_str(&heading);
    new_section.push('\n');

    if !body.is_empty() {
        new_section.push_str(body);

        // Ensure body ends with newline
        if !body.ends_with('\n') {
            new_section.push('\n');
        }
    }

    // Split and insert
    let (before, after) = if position_line == 0 {
        (Vec::new(), lines.iter().copied().collect::<Vec<_>>())
    } else {
        let split_idx = position_line.min(lines.len());
        (
            lines[..split_idx].iter().copied().collect::<Vec<_>>(),
            lines[split_idx..].iter().copied().collect::<Vec<_>>(),
        )
    };

    let mut result = String::new();

    if !before.is_empty() {
        result.push_str(&before.join("\n"));

        // Add newline before new section if not present
        if !result.ends_with('\n') && !new_section.starts_with('\n') {
            result.push('\n');
        }
    }

    result.push_str(&new_section);

    if !after.is_empty() {
        if !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&after.join("\n"));
    }

    Ok(result)
}

/// Delete a section from the content.
///
/// This removes both the heading line and all content up to the next section.
/// The section is completely removed including its heading.
///
/// # Arguments
///
/// * `content` - The full file content
/// * `section` - The section to delete
///
/// # Returns
///
/// The modified file content with the section removed.
///
/// # Examples
///
/// ```ignore
/// let updated = delete_section(content, &section)?;
/// ```
pub fn delete_section(content: &str, section: &Section) -> Result<String> {
    let lines: Vec<&str> = content.split('\n').collect();

    if section.start_line == 0 || section.start_line > lines.len() {
        bail!(
            "Invalid section start line: {} (file has {} lines)",
            section.start_line,
            lines.len()
        );
    }

    // Convert to 0-based indexing
    let start_idx = section.start_line - 1;
    let end_idx = section.end_line.min(lines.len());

    // Get content before and after the section
    let before: Vec<&str> = lines[..start_idx].to_vec();
    let after: Vec<&str> = if end_idx < lines.len() {
        lines[end_idx..].to_vec()
    } else {
        Vec::new()
    };

    // Merge before and after, handling spacing
    let mut result = String::new();

    if !before.is_empty() {
        result.push_str(&before.join("\n"));

        // Remove trailing blank lines if section deleted in middle
        if !after.is_empty() {
            while result.ends_with("\n\n") {
                result.pop();
            }
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
        }
    }

    if !after.is_empty() {
        // Skip leading blank lines after deleted section
        let skip_blank = after.iter().take_while(|l| l.trim().is_empty()).count();
        let after_content: Vec<&str> = after[skip_blank..].to_vec();

        if !after_content.is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }

            // Add one blank line before next section
            if !result.is_empty() {
                result.push('\n');
            }

            result.push_str(&after_content.join("\n"));
        }
    }

    Ok(result)
}

/// Find a section by its index among the provided sections.
///
/// # Arguments
///
/// * `sections` - All sections in the file
/// * `index` - The section index to find (e.g., "1.1" or "1.2.3")
///
/// # Returns
///
/// The section if found, None otherwise.
pub fn find_section_by_index<'a>(sections: &'a [Section], index: &str) -> Option<&'a Section> {
    sections.iter().find(|s| s.index == index)
}

/// Find a section by its header path.
///
/// This matches sections by traversing the hierarchy based on titles.
/// For example, ["Parent", "Child"] finds the section titled "Child"
/// that is a descendant of a section titled "Parent".
///
/// # Arguments
///
/// * `sections` - All sections in the file (must be in order)
/// * `path` - The header path components to match
///
/// # Returns
///
/// The section if found, None otherwise.
pub fn find_section_by_header_path<'a>(sections: &'a [Section], path: &[String]) -> Option<&'a Section> {
    if path.is_empty() {
        return None;
    }

    let mut current_level: u8 = 0;
    let mut parent_section: Option<&Section> = None;

    for component in path {
        // Find the next section with matching title at appropriate level
        let found = sections.iter().find(|s| {
            s.title.trim() == component.trim()
                && s.level > current_level
                && (parent_section.is_none()
                    || s.parent_index.as_ref() == parent_section.map(|p| &p.index))
        });

        match found {
            Some(section) => {
                current_level = section.level;
                parent_section = Some(section);
            }
            None => return None,
        }
    }

    parent_section
}

/// Resolve a section address to a concrete section.
///
/// This takes a parsed `SectionAddress` and resolves it against
/// the actual sections in a file to return the matching `Section`.
///
/// # Arguments
///
/// * `address` - The parsed section address
/// * `sections` - All sections in the file
///
/// # Returns
///
/// The resolved section if found.
///
/// # Errors
///
/// Returns an error if the address cannot be resolved to a section.
pub fn resolve_section_address<'a>(
    address: &SectionAddress,
    sections: &'a [Section],
) -> Result<&'a Section> {
    match address {
        SectionAddress::Index(index) => {
            find_section_by_index(sections, index)
                .ok_or_else(|| anyhow::anyhow!("Section #{} not found", index))
        }
        SectionAddress::HeaderPath(path) => {
            find_section_by_header_path(sections, path)
                .ok_or_else(|| anyhow::anyhow!("Section path '{}' not found", path.join(" > ")))
        }
        SectionAddress::LineRange(start, end) => {
            // Find section that contains this line range
            let section = sections.iter().find(|s| {
                // Section contains the range if start is within it
                // or if it overlaps
                (*start >= s.start_line && *start < s.end_line)
                    || (*end >= s.start_line && *end <= s.end_line)
                    || (s.start_line >= *start && s.end_line <= *end)
            });

            section.ok_or_else(|| {
                anyhow::anyhow!("No section found for line range L{}-L{}", start, end)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_index() {
        assert_eq!(
            parse_section_address("#1.1").unwrap(),
            SectionAddress::Index("#1.1".to_string())
        );
        assert_eq!(
            parse_section_address("1.2.3").unwrap(),
            SectionAddress::Index("#1.2.3".to_string())
        );
    }

    #[test]
    fn test_parse_line_range() {
        assert_eq!(
            parse_section_address("L10-L25").unwrap(),
            SectionAddress::LineRange(10, 25)
        );
        assert_eq!(
            parse_section_address("10-25").unwrap(),
            SectionAddress::LineRange(10, 25)
        );
        assert_eq!(
            parse_section_address("L10").unwrap(),
            SectionAddress::LineRange(10, 10)
        );
    }

    #[test]
    fn test_parse_header_path() {
        assert_eq!(
            parse_section_address("## Parent > ### Child").unwrap(),
            SectionAddress::HeaderPath(vec!["Parent".to_string(), "Child".to_string()])
        );
        assert_eq!(
            parse_section_address("Parent > Child").unwrap(),
            SectionAddress::HeaderPath(vec!["Parent".to_string(), "Child".to_string()])
        );
    }

    #[test]
    fn test_read_lines() {
        let content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5";
        assert_eq!(read_section_lines(content, 1, 3).unwrap(), "Line 1\nLine 2\nLine 3");
        assert_eq!(read_section_lines(content, 2, 4).unwrap(), "Line 2\nLine 3\nLine 4");
    }

    #[test]
    fn test_read_lines_bounds() {
        let content = "Line 1\nLine 2\nLine 3";
        // Requesting beyond file bounds should truncate
        assert_eq!(read_section_lines(content, 2, 10).unwrap(), "Line 2\nLine 3");
    }

    #[test]
    fn test_parse_empty_address() {
        assert!(parse_section_address("").is_err());
    }

    #[test]
    fn test_parse_single_index() {
        assert_eq!(
            parse_section_address("#1").unwrap(),
            SectionAddress::Index("#1".to_string())
        );
    }

    #[test]
    fn test_parse_deep_index() {
        assert_eq!(
            parse_section_address("1.2.3.4").unwrap(),
            SectionAddress::Index("#1.2.3.4".to_string())
        );
    }

    #[test]
    fn test_parse_single_line() {
        assert_eq!(
            parse_section_address("L5").unwrap(),
            SectionAddress::LineRange(5, 5)
        );
    }


    #[test]
    fn test_read_section_content() {
        use crate::markdown::parse_document;
        let content = "# Header\n\nContent line 1\nContent line 2\n\n## Sub\n\nSub content\n";
        let doc = parse_document(content);
        let section = &doc.sections[0];
        let text = read_section_content(content, section).unwrap();
        assert!(text.contains("# Header"));
        assert!(text.contains("Content line 1"));
    }

    #[test]
    fn test_read_lines_start_zero_error() {
        let content = "Line 1\nLine 2";
        assert!(read_section_lines(content, 0, 2).is_err());
    }

    #[test]
    fn test_read_lines_reversed_error() {
        let content = "Line 1\nLine 2\nLine 3";
        assert!(read_section_lines(content, 3, 1).is_err());
    }

    #[test]
    fn test_read_lines_past_end() {
        let content = "Line 1\nLine 2";
        assert!(read_section_lines(content, 5, 10).is_err());
    }

    #[test]
    fn test_read_summary() {
        use crate::markdown::parse_document;
        let content = "# Title\n\nFirst line\nSecond line\nThird line\n\n## Sub\n\nSub content\n";
        let doc = parse_document(content);
        let previews = read_summary(content, &doc.sections, 2).unwrap();
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[0].title, "Title");
        assert_eq!(previews[1].title, "Sub");
    }

    #[test]
    fn test_replace_section_content() {
        use crate::markdown::parse_document;
        let content = "# Header\n\nOld content\n\n## Sub\n\nSub content\n";
        let doc = parse_document(content);
        let section = &doc.sections[1];
        let updated = replace_section_content(content, section, "New sub content\n").unwrap();
        assert!(updated.contains("New sub content"));
        assert!(updated.contains("# Header"));
        assert!(!updated.contains("Sub content"));
    }

    #[test]
    fn test_insert_section_at_end() {
        let content = "# Header\n\nContent\n";
        let result = insert_section(content, 3, "## New", "Body text").unwrap();
        assert!(result.contains("## New"));
        assert!(result.contains("Body text"));
        assert!(result.contains("# Header"));
    }

    #[test]
    fn test_insert_section_at_start() {
        let content = "# Header\n\nContent\n";
        let result = insert_section(content, 0, "# Title", "Intro").unwrap();
        assert!(result.starts_with("# Title"));
    }

    #[test]
    fn test_delete_section() {
        use crate::markdown::parse_document;
        let content = "# Header\n\nContent\n\n## Sub\n\nSub content\n\n## Another\n\nMore\n";
        let doc = parse_document(content);
        let section = &doc.sections[1];
        let result = delete_section(content, section).unwrap();
        assert!(result.contains("# Header"));
        assert!(!result.contains("## Sub"));
        assert!(!result.contains("Sub content"));
        assert!(result.contains("## Another"));
    }

    #[test]
    fn test_find_section_by_index() {
        use crate::markdown::parse_document;
        let content = "# Header\n\n## Sub A\n\n## Sub B\n";
        let doc = parse_document(content);
        let found = find_section_by_index(&doc.sections, "#1.1");
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Sub A");
        let not_found = find_section_by_index(&doc.sections, "#9.9");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_find_section_by_header_path() {
        use crate::markdown::parse_document;
        let content = "# Main\n\n## FAQ\n\n### Question 1\n";
        let doc = parse_document(content);
        let path = vec!["Main".to_string(), "FAQ".to_string(), "Question 1".to_string()];
        let found = find_section_by_header_path(&doc.sections, &path);
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Question 1");
    }

    #[test]
    fn test_resolve_index_address() {
        use crate::markdown::parse_document;
        let content = "# Header\n\n## Sub\n";
        let doc = parse_document(content);
        let addr = SectionAddress::Index("#1.1".to_string());
        let result = resolve_section_address(&addr, &doc.sections);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().title, "Sub");
    }

    #[test]
    fn test_resolve_line_range_address() {
        use crate::markdown::parse_document;
        let content = "# Header\n\nContent\n\n## Sub\n\nSub content\n";
        let doc = parse_document(content);
        let addr = SectionAddress::LineRange(5, 7);
        let result = resolve_section_address(&addr, &doc.sections);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().title, "Header");
    }

    #[test]
    fn test_section_address_display() {
        let idx = SectionAddress::Index("#1.1".to_string());
        assert_eq!(format!("{}", idx), "#1.1");
        let lr = SectionAddress::LineRange(10, 25);
        assert_eq!(format!("{}", lr), "L10-L25");
        let hp = SectionAddress::HeaderPath(vec!["A".to_string(), "B".to_string()]);
        assert_eq!(format!("{}", hp), "A > B");
    }

    #[test]
    fn test_section_address_predicates() {
        let idx = SectionAddress::Index("#1".to_string());
        assert!(idx.is_index());
        assert!(!idx.is_line_range());
        assert!(!idx.is_header_path());
        let lr = SectionAddress::LineRange(1, 5);
        assert!(lr.is_line_range());
        let hp = SectionAddress::HeaderPath(vec!["A".to_string()]);
        assert!(hp.is_header_path());
    }


}
