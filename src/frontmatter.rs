//! YAML frontmatter parsing, filtering, and serialization for Markdown files.
//!
//! This module provides functionality to:
//! - Parse YAML frontmatter from markdown content (delimited by `---`)
//! - Extract individual fields
//! - Filter frontmatter by expressions like `tags contains "rust"`
//! - Set/update frontmatter fields
//! - Serialize frontmatter back to YAML

use regex::Regex;
use serde_json::json;

/// Represents a single frontmatter field with its metadata
#[derive(Debug, Clone, PartialEq)]
pub struct FrontmatterField {
    /// The field key/name
    pub key: String,
    /// The field value as JSON string
    pub value_json: String,
    /// The type of the value: "string", "number", "boolean", "array", "object"
    pub value_type: String,
    /// Human-readable text representation (if applicable)
    pub value_text: Option<String>,
}

/// Contains parsed frontmatter data with all fields and raw YAML
#[derive(Debug, Clone, PartialEq)]
pub struct FrontmatterData {
    /// All parsed frontmatter fields
    pub fields: Vec<FrontmatterField>,
    /// Raw YAML string as found in the document
    pub raw_yaml: String,
}

/// Parses YAML frontmatter from markdown content.
///
/// The frontmatter must be delimited by `---` at the start of the file:
/// ```markdown
/// ---
/// title: "My Post"
/// tags: ["rust", "cli"]
/// ---
/// # Content here
/// ```
///
/// # Arguments
/// * `content` - The markdown content to parse
///
/// # Returns
/// * `Some(FrontmatterData)` if frontmatter is found and valid
/// * `None` if no frontmatter is found
pub fn parse_frontmatter(content: &str) -> Option<FrontmatterData> {
    let (raw_yaml, _) = extract_frontmatter_raw(content)?;

    // Parse the YAML content
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(&raw_yaml).ok()?;

    // Convert to a mapping for easier field extraction
    let mapping = match yaml_value {
        serde_yaml::Value::Mapping(m) => m,
        _ => return None,
    };

    let mut fields = Vec::new();

    for (key, value) in mapping {
        let key_str = match key {
            serde_yaml::Value::String(s) => s,
            _ => continue,
        };

        let value_json = yaml_to_json(&value);
        let value_json_str = serde_json::to_string(&value_json).ok()?;
        let value_type = value_type_of(&value);
        let value_text = value_to_text(&value_json);

        fields.push(FrontmatterField {
            key: key_str,
            value_json: value_json_str,
            value_type,
            value_text,
        });
    }

    // Sort fields by key for consistent ordering
    fields.sort_by(|a, b| a.key.cmp(&b.key));

    Some(FrontmatterData { fields, raw_yaml })
}

/// Extracts the raw YAML frontmatter string from content.
///
/// # Arguments
/// * `content` - The markdown content to parse
///
/// # Returns
/// * `Some((yaml_string, end_line))` if frontmatter is found
///   - `yaml_string`: The raw YAML content between the delimiters
///   - `end_line`: The line number where frontmatter ends (1-indexed, line after closing `---`)
/// * `None` if no valid frontmatter delimiters are found
pub fn extract_frontmatter_raw(content: &str) -> Option<(String, usize)> {
    // Check if content starts with "---"
    if !content.starts_with("---") {
        return None;
    }

    let lines: Vec<&str> = content.lines().collect();

    if lines.is_empty() || lines[0].trim() != "---" {
        return None;
    }

    // Find the closing "---"
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            // Extract YAML content between opening and closing delimiters
            let yaml_lines = &lines[1..i];
            let yaml_content = yaml_lines.join("\n");
            // end_line is the line number after the closing delimiter (1-indexed)
            let end_line = i + 1;
            return Some((yaml_content, end_line));
        }
    }

    None
}

/// Gets a specific field from frontmatter data by key.
///
/// # Arguments
/// * `data` - The frontmatter data to search
/// * `key` - The field key to look up
///
/// # Returns
/// * `Some(&FrontmatterField)` if the key exists
/// * `None` if the key is not found
pub fn get_field<'a>(data: &'a FrontmatterData, key: &str) -> Option<&'a FrontmatterField> {
    data.fields.iter().find(|f| f.key == key)
}

/// Evaluates a filter expression against frontmatter data.
///
/// Supported operators:
/// - `==` - Equality: `tags == "rust"`
/// - `!=` - Inequality: `status != "published"`
/// - `contains` - String/array contains: `tags contains "rust"`
/// - `>` - Greater than (for numbers and dates): `date > "2024-01-01"`
/// - `<` - Less than: `priority < "5"`
///
/// # Arguments
/// * `data` - The frontmatter data to filter
/// * `filter_expr` - The filter expression to evaluate
///
/// # Returns
/// * `true` if the filter matches
/// * `false` if it doesn't match or the expression is invalid
pub fn filter_matches(data: &FrontmatterData, filter_expr: &str) -> bool {
    // Parse the filter expression
    let parsed = match parse_filter_expression(filter_expr) {
        Some(p) => p,
        None => return false,
    };

    // Find the field
    let field = match get_field(data, &parsed.field) {
        Some(f) => f,
        None => return false,
    };

    // Parse the field value
    let field_value: serde_json::Value = match serde_json::from_str(&field.value_json) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // Evaluate based on operator
    match parsed.operator.as_str() {
        "==" => evaluate_equals(&field_value, &parsed.value),
        "!=" => !evaluate_equals(&field_value, &parsed.value),
        "contains" => evaluate_contains(&field_value, &parsed.value),
        ">" => evaluate_greater_than(&field_value, &parsed.value, &field.value_type),
        "<" => evaluate_less_than(&field_value, &parsed.value, &field.value_type),
        _ => false,
    }
}

/// Sets or updates a frontmatter field in the content.
///
/// If no frontmatter exists, a new one will be created at the start.
///
/// # Arguments
/// * `content` - The markdown content
/// * `key` - The field key to set
/// * `value` - The value to set (will be parsed as YAML)
///
/// # Returns
/// The updated content with the field set
pub fn set_frontmatter_field(content: &str, key: &str, value: &str) -> String {
    // Try to parse the value as YAML first
    let yaml_value: serde_yaml::Value = match serde_yaml::from_str(value) {
        Ok(v) => v,
        // If it fails, treat it as a string
        Err(_) => serde_yaml::Value::String(value.to_string()),
    };

    // Check if frontmatter exists
    if let Some((raw_yaml, end_line)) = extract_frontmatter_raw(content) {
        // Parse existing YAML
        let mut mapping: serde_yaml::Mapping = match serde_yaml::from_str(&raw_yaml) {
            Ok(m) => m,
            Err(_) => {
                // If parsing fails, create new frontmatter
                return create_new_frontmatter(content, key, &yaml_value);
            }
        };

        // Update or insert the field
        mapping.insert(serde_yaml::Value::String(key.to_string()), yaml_value);

        // Serialize back to YAML
        let new_yaml = match serde_yaml::to_string(&mapping) {
            Ok(y) => y,
            Err(_) => return content.to_string(),
        };

        // Rebuild the content
        let lines: Vec<&str> = content.lines().collect();
        let after_frontmatter = if end_line < lines.len() {
            lines[end_line..].join("\n")
        } else {
            String::new()
        };

        format!("---\n{}---\n{}", new_yaml, after_frontmatter)
    } else {
        // Create new frontmatter
        create_new_frontmatter(content, key, &yaml_value)
    }
}

/// Returns the type name of a YAML value.
///
/// # Arguments
/// * `val` - The YAML value to inspect
///
/// # Returns
/// One of: "string", "number", "boolean", "array", "object", or "null"
pub fn value_type_of(val: &serde_yaml::Value) -> String {
    match val {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(_) => "boolean".to_string(),
        serde_yaml::Value::Number(_) => "number".to_string(),
        serde_yaml::Value::String(_) => "string".to_string(),
        serde_yaml::Value::Sequence(_) => "array".to_string(),
        serde_yaml::Value::Mapping(_) => "object".to_string(),
        serde_yaml::Value::Tagged(_) => "tagged".to_string(),
    }
}

/// Converts a YAML value to a JSON value.
///
/// # Arguments
/// * `val` - The YAML value to convert
///
/// # Returns
/// The equivalent JSON value
pub fn yaml_to_json(val: &serde_yaml::Value) -> serde_json::Value {
    match val {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => json!(b),
        serde_yaml::Value::Number(n) => {
            if n.is_i64() {
                json!(n.as_i64())
            } else if n.is_u64() {
                json!(n.as_u64())
            } else {
                json!(n.as_f64())
            }
        }
        serde_yaml::Value::String(s) => json!(s),
        serde_yaml::Value::Sequence(seq) => {
            let arr: Vec<serde_json::Value> = seq.iter().map(yaml_to_json).collect();
            json!(arr)
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in map {
                if let Some(key_str) = k.as_str() {
                    obj.insert(key_str.to_string(), yaml_to_json(v));
                }
            }
            json!(obj)
        }
        serde_yaml::Value::Tagged(_) => json!(null),
    }
}

// ============================================================================
// Helper structs and functions
// ============================================================================

/// Represents a parsed filter expression
struct FilterExpression {
    field: String,
    operator: String,
    value: String,
}

/// Parses a filter expression into its components.
///
/// Supports formats:
/// - `field == "value"`
/// - `field != "value"`
/// - `field contains "value"`
/// - `field > "value"`
/// - `field < "value"`
fn parse_filter_expression(expr: &str) -> Option<FilterExpression> {
    let expr = expr.trim();

    // Try each operator pattern
    let operators = vec!["==", "!=", "contains", ">", "<"];

    for op in operators {
        let pattern = format!(r#"^\s*(\w+)\s+{}\s*"([^"]*)"\s*$"#, op);
        let re = Regex::new(&pattern).ok()?;
        if let Some(caps) = re.captures(expr) {
            return Some(FilterExpression {
                field: caps.get(1)?.as_str().to_string(),
                operator: op.to_string(),
                value: caps.get(2)?.as_str().to_string(),
            });
        }

        // Also try without quotes for numeric values
        let pattern = format!(r#"^\s*(\w+)\s+{}\s*(\S+)\s*$"#, op);
        let re = Regex::new(&pattern).ok()?;
        if let Some(caps) = re.captures(expr) {
            return Some(FilterExpression {
                field: caps.get(1)?.as_str().to_string(),
                operator: op.to_string(),
                value: caps.get(2)?.as_str().to_string(),
            });
        }
    }

    None
}

/// Evaluates equality between a field value and a string value.
fn evaluate_equals(field_value: &serde_json::Value, target: &str) -> bool {
    match field_value {
        serde_json::Value::String(s) => s == target,
        serde_json::Value::Number(n) => {
            if let Ok(i) = target.parse::<i64>() {
                n.as_i64() == Some(i)
            } else if let Ok(f) = target.parse::<f64>() {
                n.as_f64() == Some(f)
            } else {
                false
            }
        }
        serde_json::Value::Bool(b) => {
            if let Ok(parsed) = target.parse::<bool>() {
                *b == parsed
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Evaluates whether a field value contains a target string.
///
/// For arrays, checks if any element contains the target.
/// For strings, checks if the string contains the target.
fn evaluate_contains(field_value: &serde_json::Value, target: &str) -> bool {
    match field_value {
        serde_json::Value::String(s) => s.contains(target),
        serde_json::Value::Array(arr) => arr.iter().any(|v| match v {
            serde_json::Value::String(s) => s == target,
            _ => false,
        }),
        _ => false,
    }
}

/// Evaluates greater-than comparison.
fn evaluate_greater_than(field_value: &serde_json::Value, target: &str, value_type: &str) -> bool {
    match value_type {
        "number" => {
            if let Some(n) = field_value.as_i64() {
                if let Ok(target_num) = target.parse::<i64>() {
                    return n > target_num;
                }
            }
            if let Some(n) = field_value.as_f64() {
                if let Ok(target_num) = target.parse::<f64>() {
                    return n > target_num;
                }
            }
        }
        "string" => {
            // For strings, compare lexicographically
            if let Some(s) = field_value.as_str() {
                return s > target;
            }
        }
        _ => {}
    }
    false
}

/// Evaluates less-than comparison.
fn evaluate_less_than(field_value: &serde_json::Value, target: &str, value_type: &str) -> bool {
    match value_type {
        "number" => {
            if let Some(n) = field_value.as_i64() {
                if let Ok(target_num) = target.parse::<i64>() {
                    return n < target_num;
                }
            }
            if let Some(n) = field_value.as_f64() {
                if let Ok(target_num) = target.parse::<f64>() {
                    return n < target_num;
                }
            }
        }
        "string" => {
            if let Some(s) = field_value.as_str() {
                return s < target;
            }
        }
        _ => {}
    }
    false
}

/// Creates a human-readable text representation of a JSON value.
fn value_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Array(arr) => {
            let strings: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
            if !strings.is_empty() {
                Some(strings.join(", "))
            } else {
                Some(format!("[{} items]", arr.len()))
            }
        }
        serde_json::Value::Object(obj) => Some(format!(
            "{{{}}}",
            obj.keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        serde_json::Value::Null => Some("null".to_string()),
    }
}

/// Creates new frontmatter at the start of content.
fn create_new_frontmatter(content: &str, key: &str, value: &serde_yaml::Value) -> String {
    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(serde_yaml::Value::String(key.to_string()), value.clone());

    match serde_yaml::to_string(&mapping) {
        Ok(yaml) => format!("---\n{}---\n{}", yaml, content),
        Err(_) => content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
title: "Test Post"
tags: ["rust", "cli"]
draft: true
count: 42
---
# Content here"#;

        let result = parse_frontmatter(content);
        assert!(result.is_some());

        let data = result.unwrap();
        assert_eq!(data.fields.len(), 4);

        let title = get_field(&data, "title").unwrap();
        assert_eq!(title.key, "title");
        assert_eq!(title.value_type, "string");
        assert_eq!(title.value_text, Some("Test Post".to_string()));

        let tags = get_field(&data, "tags").unwrap();
        assert_eq!(tags.key, "tags");
        assert_eq!(tags.value_type, "array");

        let draft = get_field(&data, "draft").unwrap();
        assert_eq!(draft.value_type, "boolean");

        let count = get_field(&data, "count").unwrap();
        assert_eq!(count.value_type, "number");
    }

    #[test]
    fn test_extract_frontmatter_raw() {
        let content = r#"---
title: "Test"
tags: ["a", "b"]
---
Body content"#;

        let result = extract_frontmatter_raw(content);
        assert!(result.is_some());

        let (yaml, end_line) = result.unwrap();
        assert!(yaml.contains("title: \"Test\""));
        assert!(yaml.contains("tags:"));
        assert_eq!(end_line, 4);
    }

    #[test]
    fn test_filter_equals() {
        let content = r#"---
status: "draft"
---
"#;

        let data = parse_frontmatter(content).unwrap();
        assert!(filter_matches(&data, r#"status == "draft""#));
        assert!(!filter_matches(&data, r#"status == "published""#));
    }

    #[test]
    fn test_filter_contains() {
        let content = r#"---
tags: ["rust", "cli", "markdown"]
---
"#;

        let data = parse_frontmatter(content).unwrap();
        assert!(filter_matches(&data, r#"tags contains "rust""#));
        assert!(filter_matches(&data, r#"tags contains "cli""#));
        assert!(!filter_matches(&data, r#"tags contains "python""#));
    }

    #[test]
    fn test_set_frontmatter_field() {
        let content = r#"---
title: "Test"
---
Content"#;

        let updated = set_frontmatter_field(content, "title", "New Title");
        assert!(updated.contains("title: New Title"));

        let updated = set_frontmatter_field(content, "author", "John Doe");
        assert!(updated.contains("author: John Doe"));
        assert!(updated.contains("title: Test"));
    }

    #[test]
    fn test_set_frontmatter_field_new() {
        let content = "# No frontmatter\nJust content";

        let updated = set_frontmatter_field(content, "title", "New Post");
        assert!(updated.starts_with("---\n"));
        assert!(updated.contains("title: New Post"));
        assert!(updated.contains("# No frontmatter"));
    }

    #[test]
    fn test_value_type_of() {
        use serde_yaml::{Number, Value};

        assert_eq!(value_type_of(&Value::String("test".to_string())), "string");
        assert_eq!(value_type_of(&Value::Bool(true)), "boolean");
        assert_eq!(value_type_of(&Value::Number(Number::from(42))), "number");
        assert_eq!(value_type_of(&Value::Sequence(vec![])), "array");
        assert_eq!(
            value_type_of(&Value::Mapping(serde_yaml::Mapping::new())),
            "object"
        );
    }

    #[test]
    fn test_yaml_to_json() {
        use serde_yaml::Value;

        let yaml_str = Value::String("test".to_string());
        let json = yaml_to_json(&yaml_str);
        assert_eq!(json, serde_json::Value::String("test".to_string()));

        let yaml_arr = Value::Sequence(vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ]);
        let json = yaml_to_json(&yaml_arr);
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "This is just content without frontmatter";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_unclosed() {
        let content = "---\ntitle: Test\nThis has unclosed frontmatter";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_frontmatter_empty() {
        let content = "---\n---\nSome content";
        let result = parse_frontmatter(content);
        // Empty YAML parses to Null, not a Mapping, so parse_frontmatter returns None
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_frontmatter_nested_objects() {
        let content = r#"---
title: Test
metadata:
  author: John
  tags:
    - rust
    - testing
deep:
  nested:
    value: 42
---
Content here"#;
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();
        assert_eq!(data.fields.len(), 3);

        // Check nested metadata exists
        let metadata = get_field(&data, "metadata");
        assert!(metadata.is_some());
    }

    #[test]
    fn test_parse_frontmatter_null_values() {
        let content = "---\ntitle: Test\ndescription: null\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        let description = get_field(&data, "description");
        assert!(description.is_some());
    }

    #[test]
    fn test_filter_matches_not_equals() {
        let content = "---\ntitle: Test\nstatus: active\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        // Should match: status != inactive
        assert!(filter_matches(&data, "status != inactive"));
        // Should not match: status != active
        assert!(!filter_matches(&data, "status != active"));
    }

    #[test]
    fn test_filter_matches_greater_than() {
        let content = "---\ncount: 42\nrating: 3.5\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        assert!(filter_matches(&data, "count > 40"));
        assert!(!filter_matches(&data, "count > 50"));
        assert!(filter_matches(&data, "rating > 3.0"));
        assert!(!filter_matches(&data, "rating > 4.0"));
    }

    #[test]
    fn test_filter_matches_less_than() {
        let content = "---\ncount: 42\nrating: 3.5\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        assert!(filter_matches(&data, "count < 50"));
        assert!(!filter_matches(&data, "count < 40"));
        assert!(filter_matches(&data, "rating < 4.0"));
        assert!(!filter_matches(&data, "rating < 3.0"));
    }

    #[test]
    fn test_filter_matches_non_existent_field() {
        let content = "---\ntitle: Test\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        // Non-existent field should return false
        assert!(!filter_matches(&data, "nonexistent == value"));
        assert!(!filter_matches(&data, "missing > 10"));
    }

    #[test]
    fn test_filter_matches_invalid_expression() {
        let content = "---\ntitle: Test\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        // Invalid expressions should return false
        assert!(!filter_matches(&data, "invalid"));
        assert!(!filter_matches(&data, "title"));
        assert!(!filter_matches(&data, ""));
    }

    #[test]
    fn test_set_frontmatter_field_numeric() {
        let content = "---\ntitle: Test\ncount: 5\n---\nContent";
        let result = set_frontmatter_field(content, "count", "42");

        assert!(result.contains("count: 42"));
        assert!(result.contains("---"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn test_set_frontmatter_field_boolean() {
        let content = "---\ntitle: Test\npublished: false\n---\nContent";
        let result = set_frontmatter_field(content, "published", "true");

        assert!(result.contains("published: true"));
        assert!(result.contains("---"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn test_set_frontmatter_field_array() {
        let content = "---\ntitle: Test\n---\nContent";
        let result = set_frontmatter_field(content, "tags", "[rust, testing, examples]");

        assert!(result.contains("tags:") && result.contains("- rust") && result.contains("- testing") && result.contains("- examples"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn test_get_field_non_existent() {
        let content = "---\ntitle: Test\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        assert!(get_field(&data, "nonexistent").is_none());
        assert!(get_field(&data, "").is_none());
    }

    #[test]
    fn test_yaml_to_json_nested_mapping() {
        use serde_yaml::Value;

        let mut mapping = serde_yaml::Mapping::new();
        mapping.insert(
            Value::String("outer".to_string()),
            Value::String("value1".to_string()),
        );

        let mut inner = serde_yaml::Mapping::new();
        inner.insert(
            Value::String("inner".to_string()),
            Value::String("value2".to_string()),
        );

        mapping.insert(Value::String("nested".to_string()), Value::Mapping(inner));

        let yaml_val = Value::Mapping(mapping);
        let json = yaml_to_json(&yaml_val);

        assert!(json.is_object());
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("outer"));
        assert!(obj.contains_key("nested"));
    }

    #[test]
    fn test_yaml_to_json_number_types() {
        use serde_yaml::Value;

        // Integer
        let yaml_int = Value::Number(42.into());
        let json_int = yaml_to_json(&yaml_int);
        assert!(json_int.is_number());
        assert_eq!(json_int.as_i64(), Some(42));

        // Float
        let yaml_float = Value::Number(3.14.into());
        let json_float = yaml_to_json(&yaml_float);
        assert!(json_float.is_number());

        // Negative number
        let yaml_neg = Value::Number((-10).into());
        let json_neg = yaml_to_json(&yaml_neg);
        assert!(json_neg.is_number());
        assert_eq!(json_neg.as_i64(), Some(-10));
    }

    #[test]
    fn test_parse_frontmatter_field_ordering() {
        let content = "---\nzebra: last\napple: first\nbanana: middle\n---\nContent";
        let result = parse_frontmatter(content);
        assert!(result.is_some());
        let data = result.unwrap();

        // Fields should be sorted by key
        let keys: Vec<&str> = data.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["apple", "banana", "zebra"]);
    }
}
