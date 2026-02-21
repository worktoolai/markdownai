use std::fs;
use std::io::{self, Read};

use anyhow::{bail, Context, Result};

use crate::frontmatter;
use crate::markdown::parse_document;
use crate::section;

pub fn read_content_input(inline: Option<&str>, file: Option<&str>) -> Result<String> {
    if let Some(content) = inline {
        Ok(content.to_string())
    } else if let Some(path) = file {
        if path == "-" {
            let mut buffer = String::new();
            io::stdin().read_to_string(&mut buffer).context("Failed to read stdin")?;
            Ok(buffer)
        } else {
            fs::read_to_string(path).with_context(|| format!("Failed to read content file '{}'", path))
        }
    } else {
        bail!("No content input provided. Use -c, --content-file, or --content -")
    }
}

pub fn section_set(
    file: &str,
    section_addr: &str,
    new_content: &str,
    output: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let original_content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file '{}'", file))?;
    let original_hash = xxhash_rust::xxh3::xxh3_64(original_content.as_bytes());

    let doc = parse_document(&original_content);
    let addr = section::parse_section_address(section_addr)?;
    let target = section::resolve_section_address(&addr, &doc.sections)?;

    let new_full_content = section::replace_section_content(&original_content, target, new_content)?;

    if dry_run {
        print_diff(&original_content, &new_full_content);
    } else {
        verify_and_write(file, original_hash, &new_full_content, output)?;
    }

    Ok(new_full_content)
}

pub fn section_add(
    file: &str,
    title: &str,
    content: &str,
    after: Option<&str>,
    before: Option<&str>,
    level: Option<u8>,
    output: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let original_content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file '{}'", file))?;
    let original_hash = xxhash_rust::xxh3::xxh3_64(original_content.as_bytes());

    let doc = parse_document(&original_content);

    let heading_level = level.unwrap_or(2);
    if !(1..=6).contains(&heading_level) {
        bail!("Heading level must be between 1 and 6");
    }

    let heading = if title.starts_with('#') {
        title.to_string()
    } else {
        format!("{} {}", "#".repeat(heading_level as usize), title)
    };

    let position_line = if let Some(after_addr) = after {
        let addr = section::parse_section_address(after_addr)?;
        let target = section::resolve_section_address(&addr, &doc.sections)?;
        target.end_line
    } else if let Some(before_addr) = before {
        let addr = section::parse_section_address(before_addr)?;
        let target = section::resolve_section_address(&addr, &doc.sections)?;
        target.start_line.saturating_sub(1)
    } else {
        doc.total_lines
    };

    let new_full_content = section::insert_section(&original_content, position_line, &heading, content)?;

    if dry_run {
        print_diff(&original_content, &new_full_content);
    } else {
        verify_and_write(file, original_hash, &new_full_content, output)?;
    }

    Ok(new_full_content)
}

pub fn section_delete(
    file: &str,
    section_addr: &str,
    output: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let original_content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file '{}'", file))?;
    let original_hash = xxhash_rust::xxh3::xxh3_64(original_content.as_bytes());

    let doc = parse_document(&original_content);
    let addr = section::parse_section_address(section_addr)?;
    let target = section::resolve_section_address(&addr, &doc.sections)?;

    let new_full_content = section::delete_section(&original_content, target)?;

    if dry_run {
        print_diff(&original_content, &new_full_content);
    } else {
        verify_and_write(file, original_hash, &new_full_content, output)?;
    }

    Ok(new_full_content)
}

pub fn frontmatter_set(
    file: &str,
    key: &str,
    value: &str,
    output: Option<&str>,
    dry_run: bool,
) -> Result<String> {
    let original_content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file '{}'", file))?;
    let original_hash = xxhash_rust::xxh3::xxh3_64(original_content.as_bytes());

    let new_full_content = frontmatter::set_frontmatter_field(&original_content, key, value);

    if dry_run {
        print_diff(&original_content, &new_full_content);
    } else {
        verify_and_write(file, original_hash, &new_full_content, output)?;
    }

    Ok(new_full_content)
}

fn verify_and_write(file: &str, original_hash: u64, new_content: &str, output: Option<&str>) -> Result<()> {
    let current_content = fs::read_to_string(file)
        .with_context(|| format!("Failed to re-read file '{}'", file))?;
    let current_hash = xxhash_rust::xxh3::xxh3_64(current_content.as_bytes());

    if original_hash != current_hash {
        bail!("File has been modified externally since read. Aborting write.");
    }

    let output_path = output.unwrap_or(file);
    fs::write(output_path, new_content)
        .with_context(|| format!("Failed to write file '{}'", output_path))?;
    Ok(())
}

fn print_diff(before: &str, after: &str) {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    let max = before_lines.len().max(after_lines.len());
    for i in 0..max {
        let b = before_lines.get(i).copied().unwrap_or("");
        let a = after_lines.get(i).copied().unwrap_or("");
        if b != a {
            if !b.is_empty() {
                eprintln!("-{}", b);
            }
            if !a.is_empty() {
                eprintln!("+{}", a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_dir(name: &str) -> PathBuf {
        let temp_dir = std::env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        temp_dir
    }

    #[test]
    fn test_read_content_input_inline() {
        let content = read_content_input(Some("inline content"), None);
        assert!(content.is_ok());
        assert_eq!(content.unwrap(), "inline content");
    }

    #[test]
    fn test_read_content_input_file() {
        let temp_dir = create_test_dir("mdai_test_read_file");
        let test_file = temp_dir.join("test.md");
        fs::write(&test_file, "file content").unwrap();

        let content = read_content_input(None, Some(test_file.to_str().unwrap()));
        assert!(content.is_ok());
        assert_eq!(content.unwrap(), "file content");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_read_content_input_neither() {
        let content = read_content_input(None, None);
        assert!(content.is_err());
        assert!(content.unwrap_err().to_string().contains("No content input provided"));
    }

    #[test]
    fn test_section_set_modifies_content() {
        let temp_dir = create_test_dir("mdai_test_section_set");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\n## Subsection\n\nOriginal content\n\n## Other\n\nContent\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_set(
            test_file.to_str().unwrap(),
            "Subsection",
            "New content",
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(new_content.contains("New content"));
        assert!(!new_content.contains("Original content"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_set_dry_run() {
        let temp_dir = create_test_dir("mdai_test_section_set_dry");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\n## Subsection\n\nOriginal content\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_set(
            test_file.to_str().unwrap(),
            "Subsection",
            "New content",
            None,
            true,
        );
        assert!(result.is_ok());
        
        let file_content = fs::read_to_string(&test_file).unwrap();
        assert_eq!(file_content, initial_content);
        assert!(file_content.contains("Original content"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_set_output_file() {
        let temp_dir = create_test_dir("mdai_test_section_set_output");
        let input_file = temp_dir.join("input.md");
        let output_file = temp_dir.join("output.md");
        let initial_content = "# Main\n\n## Subsection\n\nOriginal content\n";
        fs::write(&input_file, initial_content).unwrap();

        let result = section_set(
            input_file.to_str().unwrap(),
            "Subsection",
            "New content",
            Some(output_file.to_str().unwrap()),
            false,
        );
        assert!(result.is_ok());
        
        let input_content = fs::read_to_string(&input_file).unwrap();
        assert_eq!(input_content, initial_content);
        
        let output_content = fs::read_to_string(&output_file).unwrap();
        assert!(output_content.contains("New content"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_add_new_section() {
        let temp_dir = create_test_dir("mdai_test_section_add");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\nContent\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_add(
            test_file.to_str().unwrap(),
            "New Section",
            "Section content",
            None,
            None,
            None,
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(new_content.contains("## New Section"));
        assert!(new_content.contains("Section content"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_add_after() {
        let temp_dir = create_test_dir("mdai_test_section_add_after");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\n## First\n\nContent\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_add(
            test_file.to_str().unwrap(),
            "Second",
            "Second content",
            Some("First"),
            None,
            None,
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(new_content.contains("## Second"));
        assert!(new_content.contains("Second content"));
        let lines: Vec<&str> = new_content.lines().collect();
        let first_idx = lines.iter().position(|&l| l == "## First").unwrap();
        let second_idx = lines.iter().position(|&l| l == "## Second").unwrap();
        assert!(second_idx > first_idx);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_delete_removes_section() {
        let temp_dir = create_test_dir("mdai_test_section_delete");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\n## To Delete\n\nDelete me\n\n## Keep\n\nKeep this\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_delete(
            test_file.to_str().unwrap(),
            "To Delete",
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(!new_content.contains("To Delete"));
        assert!(!new_content.contains("Delete me"));
        assert!(new_content.contains("Keep"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_section_delete_dry_run() {
        let temp_dir = create_test_dir("mdai_test_section_delete_dry");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\n## To Delete\n\nDelete me\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = section_delete(
            test_file.to_str().unwrap(),
            "To Delete",
            None,
            true,
        );
        assert!(result.is_ok());
        
        let file_content = fs::read_to_string(&test_file).unwrap();
        assert_eq!(file_content, initial_content);
        assert!(file_content.contains("To Delete"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_frontmatter_set_adds_field() {
        let temp_dir = create_test_dir("mdai_test_frontmatter_set");
        let test_file = temp_dir.join("test.md");
        let initial_content = "# Main\n\nContent\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = frontmatter_set(
            test_file.to_str().unwrap(),
            "title",
            "My Title",
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(new_content.contains("---"));
        assert!(new_content.contains("title: My Title"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_frontmatter_set_updates_field() {
        let temp_dir = create_test_dir("mdai_test_frontmatter_update");
        let test_file = temp_dir.join("test.md");
        let initial_content = "---\ntitle: Old Title\n---\n\n# Main\n\nContent\n";
        fs::write(&test_file, initial_content).unwrap();

        let result = frontmatter_set(
            test_file.to_str().unwrap(),
            "title",
            "New Title",
            None,
            false,
        );
        assert!(result.is_ok());
        
        let new_content = fs::read_to_string(&test_file).unwrap();
        assert!(new_content.contains("title: New Title"));
        assert!(!new_content.contains("Old Title"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
