//! Markdown parsing module using pulldown-cmark.
//!
//! This module provides functionality to parse markdown content and extract
//! headers with their hierarchical structure, line numbers, and section indices.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::fmt;

/// A section representing a header and its content in a markdown document.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Hierarchical section index like "#1", "#1.1", "#1.1.1", etc.
    pub index: String,
    /// Header level (1-6)
    pub level: u8,
    /// The title text of the header (no markdown formatting)
    pub title: String,
    /// 1-based line number where this header starts
    pub start_line: usize,
    /// Line number where this section ends (line before next header or EOF)
    pub end_line: usize,
    /// 0-based position in the document
    pub ordinal: usize,
    /// Parent section index, if this is not a top-level header
    pub parent_index: Option<String>,
}

impl Section {
    /// Create a new section.
    #[allow(dead_code)]
    pub fn new(
        index: String,
        level: u8,
        title: String,
        start_line: usize,
        end_line: usize,
        ordinal: usize,
        parent_index: Option<String>,
    ) -> Self {
        Section {
            index,
            level,
            title,
            start_line,
            end_line,
            ordinal,
            parent_index,
        }
    }

    /// Get the line count of this section's content (excluding header).
    pub fn content_line_count(&self) -> usize {
        self.end_line.saturating_sub(self.start_line)
    }

    /// Check if this section contains a given line number.
    pub fn contains_line(&self, line: usize) -> bool {
        line >= self.start_line && line <= self.end_line
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} (L{}-L{})",
            self.index,
            "#".repeat(self.level as usize),
            self.title,
            self.start_line,
            self.end_line
        )
    }
}

/// A parsed markdown document with all extracted sections.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDocument {
    /// All sections found in the document
    pub sections: Vec<Section>,
    /// Whether the document has YAML frontmatter
    pub has_frontmatter: bool,
    /// Total number of lines in the document
    pub total_lines: usize,
}

/// Represents a header during parsing.
struct HeaderInfo {
    level: u8,
    title: String,
    start_line: usize,
    _source_position: usize,
}

/// Main function to parse a markdown document.
///
/// # Arguments
/// * `content` - The markdown content as a string
///
/// # Returns
/// A `ParsedDocument` containing all extracted sections
pub fn parse_document(content: &str) -> ParsedDocument {
    let total_lines = content.lines().count();
    let has_frontmatter = detect_frontmatter(content);

    // Build a line number lookup from byte offsets
    let line_lookup = build_line_lookup(content);

    // Parse headers from the markdown
    let headers = extract_headers(content, &line_lookup);

    // Build hierarchical section indices
    let sections = build_sections(headers, total_lines);

    ParsedDocument {
        sections,
        has_frontmatter,
        total_lines,
    }
}

/// Detect if the content has YAML frontmatter.
fn detect_frontmatter(content: &str) -> bool {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return false;
    }

    let after_first_delim = &trimmed[3..];
    if let Some(second_delim_pos) = after_first_delim.find('\n') {
        let rest = &after_first_delim[second_delim_pos + 1..];
        return rest.starts_with("---") || rest.contains("\n---");
    }

    false
}

/// Build a lookup table mapping byte positions to line numbers.
fn build_line_lookup(content: &str) -> Vec<usize> {
    let mut lookup = vec![0];
    for (byte_pos, ch) in content.char_indices() {
        if ch == '\n' {
            lookup.push(byte_pos + 1);
        }
    }
    lookup
}

/// Get the line number for a given byte position.
fn get_line_number(lookup: &[usize], byte_pos: usize) -> usize {
    match lookup.binary_search(&byte_pos) {
        Ok(i) => i + 1,
        Err(i) => {
            if i == 0 {
                1
            } else {
                i
            }
        }
    }
}

/// Extract all headers from the markdown content.
fn extract_headers(content: &str, line_lookup: &[usize]) -> Vec<HeaderInfo> {
    let mut headers = Vec::new();
    let parser = Parser::new(content);

    let mut current_level: Option<u8> = None;
    let mut current_title = String::new();
    let mut in_heading = false;
    let mut heading_start_pos = 0;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current_level = Some(level as u8);
                in_heading = true;
                current_title.clear();
                heading_start_pos = range.start;
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(level) = current_level {
                    let start_line = get_line_number(line_lookup, heading_start_pos);
                    headers.push(HeaderInfo {
                        level,
                        title: current_title.clone(),
                        start_line,
                        _source_position: heading_start_pos,
                    });
                }
                current_level = None;
                in_heading = false;
            }
            Event::Text(text) | Event::Code(text) => {
                if in_heading {
                    current_title.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_heading {
                    current_title.push(' ');
                }
            }
            _ => {}
        }
    }

    headers
}

/// Build sections with hierarchical indices from headers.
fn build_sections(headers: Vec<HeaderInfo>, total_lines: usize) -> Vec<Section> {
    if headers.is_empty() {
        return Vec::new();
    }

    let mut sections = Vec::new();

    // Track counters for each level (1-indexed for readability, then converted to 0-indexed)
    // counters[0] = h1 counter, counters[1] = h2 counter, etc.
    let mut counters: [usize; 6] = [0; 6];
    let mut parent_stack: Vec<(String, u8)> = Vec::new();

    for (ordinal, header) in headers.iter().enumerate() {
        let level_idx = (header.level - 1) as usize;

        // Reset counters for all levels deeper than current
        for i in (level_idx + 1)..6 {
            counters[i] = 0;
        }

        // Increment counter for this level
        counters[level_idx] += 1;

        // Build section index from counters
        let mut index_parts = Vec::new();
        for i in 0..=level_idx {
            if counters[i] > 0 {
                index_parts.push(counters[i].to_string());
            }
        }
        let index = format!("#{}", index_parts.join("."));

        // Determine parent index
        let parent_index = if level_idx > 0 {
            // Find the parent: the most recent header with higher level (lower number)
            if let Some((parent_idx, _)) = parent_stack
                .iter()
                .rev()
                .find(|(_, lvl)| *lvl < header.level)
            {
                Some(parent_idx.clone())
            } else {
                None
            }
        } else {
            None
        };

        // Update parent stack - remove entries at or below current level
        parent_stack.retain(|(_, lvl)| *lvl < header.level);
        parent_stack.push((index.clone(), header.level));

        // Calculate end_line
        let end_line = calculate_end_line(&headers, ordinal, header.level, total_lines);

        sections.push(Section {
            index: index.clone(),
            level: header.level,
            title: header.title.clone(),
            start_line: header.start_line,
            end_line,
            ordinal,
            parent_index,
        });
    }

    sections
}

/// Calculate the end line for a section.
fn calculate_end_line(
    headers: &[HeaderInfo],
    ordinal: usize,
    level: u8,
    total_lines: usize,
) -> usize {
    if let Some(next_header) = headers.get(ordinal + 1) {
        // Find the next header at same or higher level
        if next_header.level <= level {
            next_header.start_line - 1
        } else {
            // Next header is deeper (child), so we need to find the next header at same/higher level
            let mut end = total_lines;
            for h in &headers[ordinal + 1..] {
                if h.level <= level {
                    end = h.start_line - 1;
                    break;
                }
            }
            end
        }
    } else {
        total_lines
    }
}

/// Find a section by its address.
///
/// The address can be:
/// - A section index like "#1", "#1.1", "#1.1.1", etc.
/// - A header path like "## Heading" or "### Subheading"
///
/// # Arguments
/// * `doc` - The parsed document to search
/// * `addr` - The address string to look up
///
/// # Returns
/// An `Option` containing a reference to the section if found
pub fn find_section_by_address<'a>(doc: &'a ParsedDocument, addr: &str) -> Option<&'a Section> {
    let trimmed = addr.trim();

    // Check if it looks like a heading (e.g., "## Heading", "### Sub")
    // Headings have # followed by more # or a space
    if trimmed.starts_with('#') {
        let after_hashes = trimmed.trim_start_matches('#');
        if after_hashes.starts_with(' ') {
            // It's a heading like "## Title"
            let level = (trimmed.len() - after_hashes.len()) as u8;
            let title = after_hashes.trim();
            return doc.sections.iter().find(|s| s.level == level && s.title == title);
        }
        // It's a section index like "#1.1"
        return doc.sections.iter().find(|s| s.index == trimmed);
    }

    // Try as exact title match
    doc.sections.iter().find(|s| s.title == trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_parsing() {
        let content = r#"# Main

## Subsection

Content here.

### Sub-subsection

More content.
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 3);
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[0].title, "Main");

        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[1].level, 2);
        assert_eq!(doc.sections[1].title, "Subsection");
        assert_eq!(doc.sections[1].parent_index, Some("#1".to_string()));

        assert_eq!(doc.sections[2].index, "#1.1.1");
        assert_eq!(doc.sections[2].level, 3);
        assert_eq!(doc.sections[2].title, "Sub-subsection");
        assert_eq!(doc.sections[2].parent_index, Some("#1.1".to_string()));
    }

    #[test]
    fn test_only_body_text_no_headers() {
        let content = r#"This is just body text.

Another paragraph of text.

And a third one.
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 0);
        assert_eq!(doc.total_lines, 5);
        assert!(!doc.has_frontmatter);
    }

    #[test]
    fn test_single_line_header_only() {
        let content = "# Header only\n";

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].title, "Header only");
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[0].start_line, 1);
        assert_eq!(doc.sections[0].end_line, 1);
    }

    #[test]
    fn test_skipped_levels_h1_to_h3() {
        let content = r#"# H1 Header

### H3 Header (skipping H2)
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[1].level, 3);
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[1].parent_index, Some("#1".to_string()));
    }

    #[test]
    fn test_multiple_sections_same_level() {
        let content = r#"# Main H1

## First H2

## Second H2

## Third H2
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 4);
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[1].level, 2);
        assert_eq!(doc.sections[2].level, 2);
        assert_eq!(doc.sections[3].level, 2);

        // Check indices
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[2].index, "#1.2");
        assert_eq!(doc.sections[3].index, "#1.3");

        // All H2s should have the same parent
        assert_eq!(doc.sections[1].parent_index, Some("#1".to_string()));
        assert_eq!(doc.sections[2].parent_index, Some("#1".to_string()));
        assert_eq!(doc.sections[3].parent_index, Some("#1".to_string()));
    }

    #[test]
    fn test_find_by_address_exact_title() {
        let content = r#"# Introduction

## Overview

This is the overview section.

## Details

More details here.
"#;

        let doc = parse_document(content);

        // Find by path notation
        let section = find_section_by_address(&doc, "#1");
        assert!(section.is_some());
        assert_eq!(section.unwrap().title, "Introduction");

        let section = find_section_by_address(&doc, "#1.1");
        assert!(section.is_some());
        assert_eq!(section.unwrap().title, "Overview");

        let section = find_section_by_address(&doc, "#1.2");
        assert!(section.is_some());
        assert_eq!(section.unwrap().title, "Details");
    }

    #[test]
    fn test_find_by_address_non_existent() {
        let content = "# Only one header\n";

        let doc = parse_document(content);

        assert!(find_section_by_address(&doc, "#2").is_none());
        assert!(find_section_by_address(&doc, "#1.1").is_none());
        assert!(find_section_by_address(&doc, "#999").is_none());
        assert!(find_section_by_address(&doc, "invalid").is_none());
        assert!(find_section_by_address(&doc, "").is_none());
    }

    #[test]
    fn test_section_display_trait() {
        let content = r#"# Test Header

Some content.
"#;

        let doc = parse_document(content);
        let section = &doc.sections[0];

        let display_output = format!("{}", section);
        assert!(display_output.contains("#1"));
        assert!(display_output.contains("#"));
        assert!(display_output.contains("Test Header"));
        assert!(display_output.contains("L1"));
        assert!(display_output.contains("L3"));

        // Format: index #level title (Lstart-Lend)
        assert_eq!(display_output, "#1 # Test Header (L1-L3)");
    }

    #[test]
    fn test_document_with_frontmatter_before_headers() {
        let content = r#"---
title: My Document
author: Test Author
---

# First Header

Content after frontmatter.
"#;

        let doc = parse_document(content);

        assert!(doc.has_frontmatter);
        assert!(doc.sections.iter().any(|s| s.title == "First Header"));
    }

    #[test]
    fn test_only_whitespace_content() {
        let content = "   \n\n\t\n   \n";

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 0);
        assert_eq!(doc.total_lines, 4);
        assert!(!doc.has_frontmatter);
    }


    #[test]
    fn test_back_to_back_headers() {
        let content = r#"# Header 1
## Header 2
### Header 3
#### Header 4
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 4);

        // Each header should end at its own line since the next line is another header
        assert_eq!(doc.sections[0].start_line, 1);
        assert_eq!(doc.sections[0].end_line, 4);

        assert_eq!(doc.sections[1].start_line, 2);
        assert_eq!(doc.sections[1].end_line, 4);

        assert_eq!(doc.sections[2].start_line, 3);
        assert_eq!(doc.sections[2].end_line, 4);

        assert_eq!(doc.sections[3].start_line, 4);
        assert_eq!(doc.sections[3].end_line, 4);
    }

    #[test]
    fn test_single_header_document() {
        let content = "# Lone Header";

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 1);
        assert_eq!(doc.sections[0].title, "Lone Header");
        assert_eq!(doc.sections[0].level, 1);
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[0].start_line, 1);
        assert_eq!(doc.sections[0].end_line, 1);
        assert_eq!(doc.sections[0].parent_index, None);
        assert_eq!(doc.sections[0].ordinal, 0);
        assert_eq!(doc.total_lines, 1);
    }

    #[test]
    fn test_multiple_h1s() {
        let content = r#"# First H1

## Child of First

# Second H1

## Child of Second
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 4);
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[2].index, "#2");
        assert_eq!(doc.sections[3].index, "#2.1");
    }

    #[test]
    fn test_same_name_different_number() {
        let content = r#"# Main

## FAQ

### Question 1

### Question 2

## API
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 5);
        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[1].title, "FAQ");
        assert_eq!(doc.sections[2].index, "#1.1.1");
        assert_eq!(doc.sections[2].title, "Question 1");
        assert_eq!(doc.sections[3].index, "#1.1.2");
        assert_eq!(doc.sections[3].title, "Question 2");
        assert_eq!(doc.sections[4].index, "#1.2");
        assert_eq!(doc.sections[4].title, "API");
    }

    #[test]
    fn test_line_numbers() {
        let content = "# Header\n\nContent\n\n## Sub\n";
        let doc = parse_document(content);

        assert_eq!(doc.sections[0].start_line, 1);
        assert_eq!(doc.sections[0].end_line, 5);
        assert_eq!(doc.sections[1].start_line, 5);
        assert_eq!(doc.sections[1].end_line, 5);
    }

    #[test]
    fn test_find_by_index() {
        let content = "# Main\n\n## Sub\n";
        let doc = parse_document(content);

        let section = find_section_by_address(&doc, "#1.1");
        assert!(section.is_some());
        assert_eq!(section.unwrap().title, "Sub");
    }

    #[test]
    fn test_find_by_path() {
        let content = "# Main\n\n## Sub\n";
        let doc = parse_document(content);

        let section = find_section_by_address(&doc, "## Sub");
        assert!(section.is_some());
        assert_eq!(section.unwrap().index, "#1.1");
    }

    #[test]
    fn test_frontmatter_detection() {
        let content = r#"---
title: Test
---

# Header
"#;

        let doc = parse_document(content);
        assert!(doc.has_frontmatter);
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "# Header\n\nNo frontmatter here.";
        let doc = parse_document(content);
        assert!(!doc.has_frontmatter);
    }

    #[test]
    fn test_deep_nesting() {
        let content = r#"# H1
## H2
### H3
#### H4
##### H5
###### H6
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 6);
        assert_eq!(doc.sections[0].index, "#1");
        assert_eq!(doc.sections[1].index, "#1.1");
        assert_eq!(doc.sections[2].index, "#1.1.1");
        assert_eq!(doc.sections[3].index, "#1.1.1.1");
        assert_eq!(doc.sections[4].index, "#1.1.1.1.1");
        assert_eq!(doc.sections[5].index, "#1.1.1.1.1.1");
    }

    #[test]
    fn test_section_methods() {
        let content = "# Header\n\nLine 1\nLine 2\n";
        let doc = parse_document(content);
        let section = &doc.sections[0];

        assert_eq!(section.start_line, 1);
        assert_eq!(section.end_line, 4);
        assert_eq!(section.content_line_count(), 3);
        assert!(section.contains_line(2));
        assert!(!section.contains_line(10));
    }

    #[test]
    fn test_empty_document() {
        let content = "";
        let doc = parse_document(content);

        assert_eq!(doc.sections.len(), 0);
        assert_eq!(doc.total_lines, 0);
    }

    #[test]
    fn test_markdown_in_headers() {
        let content = r#"# Header with **bold** and `code`

## Header with *italic*
"#;

        let doc = parse_document(content);

        assert_eq!(doc.sections[0].title, "Header with bold and code");
        assert_eq!(doc.sections[1].title, "Header with italic");
    }
}
