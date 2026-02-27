#![allow(dead_code)]
//! Full-text search engine using Tantivy for markdown content.
//!
//! This module provides persistent full-text search using Tantivy with a disk-based
//! index at `.worktoolai/markdownai_index/`.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use tantivy::{
    collector::TopDocs,
    query::{BooleanQuery, FuzzyTermQuery, QueryParser, RegexQuery, TermQuery},
    schema::*,
    Index, IndexWriter, ReloadPolicy, Term,
};

use crate::cli::{MatchMode, SearchScope};

// Schema field names
const FIELD_PATH: &str = "path";
const FIELD_SECTION_INDEX: &str = "section_index";
const FIELD_SECTION_TITLE: &str = "section_title";
const FIELD_BODY: &str = "body";
const FIELD_LINE: &str = "line";
const FIELD_SYNC_EPOCH: &str = "sync_epoch";

/// Create the Tantivy schema for markdown section indexing.
fn create_schema() -> Schema {
    let mut schema_builder = Schema::builder();

    schema_builder.add_text_field(FIELD_PATH, STRING | STORED);
    schema_builder.add_text_field(FIELD_SECTION_INDEX, STRING | STORED);
    schema_builder.add_text_field(FIELD_SECTION_TITLE, TEXT | STORED);
    schema_builder.add_text_field(FIELD_BODY, TEXT | STORED);
    schema_builder.add_u64_field(FIELD_LINE, STORED);
    schema_builder.add_u64_field(FIELD_SYNC_EPOCH, STORED);

    schema_builder.build()
}

/// A single search result hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub file: String,
    pub section_index: String,
    pub section_title: String,
    pub line: usize,
    pub snippet: String,
    pub score: f32,
}

/// Full-text search engine for markdown content.
pub struct SearchEngine {
    index: Index,
    _schema: Schema,
    path_field: Field,
    section_index_field: Field,
    section_title_field: Field,
    body_field: Field,
    line_field: Field,
    sync_epoch_field: Field,
}

impl SearchEngine {
    /// Open an existing index or create a new one at the specified directory.
    pub fn open(index_dir: &Path) -> Result<Self> {
        let schema = create_schema();

        std::fs::create_dir_all(index_dir)
            .with_context(|| format!("Failed to create index directory: {:?}", index_dir))?;

        // Try to open existing index first, fall back to creating new one
        let index = match Index::open_in_dir(index_dir) {
            Ok(idx) => idx,
            Err(_) => Index::create_in_dir(index_dir, schema.clone())
                .with_context(|| format!("Failed to create index at {:?}", index_dir))?,
        };

        let path_field = schema.get_field(FIELD_PATH).map_err(|e| anyhow!("{}", e))?;
        let section_index_field = schema.get_field(FIELD_SECTION_INDEX).map_err(|e| anyhow!("{}", e))?;
        let section_title_field = schema.get_field(FIELD_SECTION_TITLE).map_err(|e| anyhow!("{}", e))?;
        let body_field = schema.get_field(FIELD_BODY).map_err(|e| anyhow!("{}", e))?;
        let line_field = schema.get_field(FIELD_LINE).map_err(|e| anyhow!("{}", e))?;
        let sync_epoch_field = schema.get_field(FIELD_SYNC_EPOCH).map_err(|e| anyhow!("{}", e))?;

        Ok(SearchEngine {
            index,
            _schema: schema,
            path_field,
            section_index_field,
            section_title_field,
            body_field,
            line_field,
            sync_epoch_field,
        })
    }

    /// Get an index writer with retry logic for lock acquisition.
    fn get_writer(&self) -> Result<IndexWriter> {
        let mut last_err = None;
        for attempt in 0..3 {
            match self.index.writer(50_000_000) {
                Ok(w) => return Ok(w),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        }
        Err(anyhow!("Failed to acquire index writer after 3 attempts: {}", last_err.unwrap()))
    }

    /// Index or re-index all sections for a single file.
    pub fn index_file(
        &self,
        path: &str,
        sections: &[(String, String, String, usize)],
        sync_epoch: u64,
    ) -> Result<()> {
        self.remove_file(path)?;

        let mut writer = self.get_writer()?;

        for (section_index, section_title, body_text, start_line) in sections {
            let mut doc = TantivyDocument::default();

            doc.add_text(self.path_field, path);
            doc.add_text(self.section_index_field, section_index);
            doc.add_text(self.section_title_field, section_title);
            doc.add_text(self.body_field, body_text);
            doc.add_u64(self.line_field, *start_line as u64);
            doc.add_u64(self.sync_epoch_field, sync_epoch);

            writer.add_document(doc)?;
        }

        writer.commit()?;
        Ok(())
    }

    /// Remove all documents for a specific file path.
    pub fn remove_file(&self, path: &str) -> Result<()> {
        let mut writer = self.get_writer()?;
        let path_term = Term::from_field_text(self.path_field, path);
        writer.delete_term(path_term);
        writer.commit()?;
        Ok(())
    }

    /// Search the index for matching sections.
    pub fn search(
        &self,
        query_str: &str,
        match_mode: &MatchMode,
        scope: &SearchScope,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<SearchHit>, usize)> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        let searcher = reader.searcher();

        let query = self.build_query(query_str, match_mode, scope)?;

        let top_docs = TopDocs::with_limit(limit + offset);
        let collected_docs: Vec<(f32, tantivy::DocAddress)> =
            searcher.search(&*query, &top_docs)?;

        let total_count = collected_docs.len();

        let hits: Vec<SearchHit> = collected_docs
            .into_iter()
            .skip(offset)
            .map(|(score, doc_address)| {
                let retrieved_doc: TantivyDocument = searcher.doc(doc_address).unwrap();

                let file = retrieved_doc
                    .get_first(self.path_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let section_index = retrieved_doc
                    .get_first(self.section_index_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let section_title = retrieved_doc
                    .get_first(self.section_title_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let line = retrieved_doc
                    .get_first(self.line_field)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                let body = retrieved_doc
                    .get_first(self.body_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let snippet = generate_snippet(query_str, &body, 150);

                SearchHit {
                    file,
                    section_index,
                    section_title,
                    line,
                    snippet,
                    score,
                }
            })
            .collect();

        Ok((hits, total_count))
    }

    /// Build a Tantivy query based on match mode and scope.
    fn build_query(
        &self,
        query_str: &str,
        match_mode: &MatchMode,
        scope: &SearchScope,
    ) -> Result<Box<dyn tantivy::query::Query>> {
        let search_fields = match scope {
            SearchScope::All => vec![self.body_field, self.section_title_field],
            SearchScope::Body => vec![self.body_field],
            SearchScope::Headers => vec![self.section_title_field],
            SearchScope::Code => vec![self.body_field],
            SearchScope::Frontmatter => {
                return Ok(Box::new(BooleanQuery::from(vec![])));
            }
        };

        let query: Box<dyn tantivy::query::Query> = match match_mode {
            MatchMode::Text => {
                let mut query_parser = QueryParser::for_index(
                    &self.index,
                    search_fields,
                );
                query_parser.set_conjunction_by_default();

                let parsed = query_parser
                    .parse_query(query_str)
                    .unwrap_or_else(|_| Box::new(BooleanQuery::from(vec![])));
                parsed
            }
            MatchMode::Exact => {
                let term = Term::from_field_text(search_fields[0], query_str);
                Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs))
            }
            MatchMode::Fuzzy => {
                let lowercase_query = query_str.to_lowercase();
                let term = Term::from_field_text(search_fields[0], &lowercase_query);
                Box::new(FuzzyTermQuery::new(term, 2, true))
            }
            MatchMode::Regex => {
                Box::new(
                    RegexQuery::from_pattern(query_str, search_fields[0])
                        .map_err(|e| anyhow!("Invalid regex: {}", e))?,
                )
            }
        };

        Ok(query)
    }

    /// Destroy the entire index directory.
    pub fn destroy(index_dir: &Path) -> Result<()> {
        if !index_dir.exists() {
            return Ok(());
        }

        std::fs::remove_dir_all(index_dir)
            .with_context(|| format!("Failed to delete index directory: {:?}", index_dir))
    }
}

/// Generate a snippet with context around matched terms.
fn generate_snippet(query_str: &str, body: &str, context_chars: usize) -> String {
    if body.is_empty() {
        return String::new();
    }

    let query_words: Vec<String> = query_str
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();

    if query_words.is_empty() {
        return body.chars().take(context_chars * 2).collect();
    }

    let body_lower = body.to_lowercase();
    let mut first_match_pos = None;

    for word in &query_words {
        if let Some(pos) = body_lower.find(word.as_str()) {
            first_match_pos = Some(pos);
            break;
        }
    }

    match first_match_pos {
        Some(pos) => {
            let start = pos.saturating_sub(context_chars);
            let end = std::cmp::min(pos + query_str.len() + context_chars, body.len());

            let snippet: String = body
                .chars()
                .skip(start)
                .take(end - start)
                .collect();

            let prefix = if start > 0 { "..." } else { "" };
            let suffix = if end < body.len() { "..." } else { "" };

            format!("{}{}{}", prefix, snippet, suffix)
        }
        None => {
            body.chars()
                .take(context_chars * 2)
                .chain("...".chars())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_create_schema() {
        let schema = create_schema();
        assert!(schema.get_field(FIELD_PATH).is_ok());
        assert!(schema.get_field(FIELD_BODY).is_ok());
    }

    #[test]
    fn test_open_engine() {
        let temp_dir = std::env::temp_dir().join("mdai_test_open");
        let _ = fs::remove_dir_all(&temp_dir);
        let engine = SearchEngine::open(&temp_dir);
        assert!(engine.is_ok());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snippet_generation() {
        let snippet = generate_snippet("test", "This is a test body with content", 10);
        assert!(snippet.contains("test"));
    }

    #[test]
    fn test_index_and_search_basic() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_basic");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index a file with sections
        let sections = vec![
            ("0".to_string(), "Header".to_string(), "This is test content".to_string(), 1),
        ];
        engine.index_file("/test/path.md", &sections, 0).expect("Failed to index");
        
        // Search should find results
        let (hits, total) = engine.search(
            "test", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "/test/path.md");
        assert_eq!(hits[0].section_index, "0");
        assert!(hits[0].snippet.contains("test") || hits[0].score > 0.0);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_remove_file_removes_all_docs() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_remove");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index multiple sections for the same file
        let sections = vec![
            ("0".to_string(), "Section 1".to_string(), "Content one".to_string(), 1),
            ("1".to_string(), "Section 2".to_string(), "Content two".to_string(), 10),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        
        // Verify indexed
        let (_hits, total) = engine.search(
            "content",
            &crate::cli::MatchMode::Text,
            &crate::cli::SearchScope::All,
            10,
            0
        ).expect("Search failed");
        assert_eq!(total, 2);

        // Remove the file
        engine.remove_file("/test/file.md").expect("Failed to remove file");
        
        // Verify all docs removed
        let (hits, total) = engine.search(
            "content", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        assert_eq!(total, 0);
        assert_eq!(hits.len(), 0);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_no_results");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index some content
        let sections = vec![
            ("0".to_string(), "Header".to_string(), "Specific content here".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        
        // Search for non-existent term
        let (hits, total) = engine.search(
            "nonexistent_term_xyz", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 0);
        assert_eq!(hits.len(), 0);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_offset() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_offset");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index multiple files with same content
        for i in 0..5 {
            let sections = vec![
                ("0".to_string(), "Header".to_string(), "test content".to_string(), 1),
            ];
            engine.index_file(&format!("/test/file{}.md", i), &sections, 0).expect("Failed to index");
        }
        
        // Search with offset
        let (hits, total) = engine.search(
            "test", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            2
        ).expect("Search failed");
        
        // Total should be 5, but we only get 3 due to offset of 2
        assert_eq!(total, 5);
        assert!(hits.len() <= 3);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_limit() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_limit");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index multiple files
        for i in 0..10 {
            let sections = vec![
                ("0".to_string(), "Header".to_string(), "test content".to_string(), 1),
            ];
            engine.index_file(&format!("/test/file{}.md", i), &sections, 0).expect("Failed to index");
        }
        
        // Search with limit
        let (hits, total) = engine.search(
            "test", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            3, 
            0
        ).expect("Search failed");
        
        // TopDocs collector caps total at limit, so total <= limit
        assert!(total <= 3);
        assert!(hits.len() <= 3);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multiple_files_indexed_and_searched() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_multiple");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        // Index multiple files with different content
        let sections1 = vec![
            ("0".to_string(), "Introduction".to_string(), "Intro content about apples".to_string(), 1),
            ("1".to_string(), "Details".to_string(), "More details about apples".to_string(), 5),
        ];
        engine.index_file("/doc1.md", &sections1, 0).expect("Failed to index doc1");
        
        let sections2 = vec![
            ("0".to_string(), "Chapter 1".to_string(), "Content about oranges".to_string(), 1),
        ];
        engine.index_file("/doc2.md", &sections2, 0).expect("Failed to index doc2");
        
        let sections3 = vec![
            ("0".to_string(), "Overview".to_string(), "Overview of bananas".to_string(), 1),
        ];
        engine.index_file("/doc3.md", &sections3, 0).expect("Failed to index doc3");
        
        // Search for content from doc1
        let (hits, _) = engine.search(
            "apples", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        assert!(hits.len() >= 1);
        assert!(hits.iter().any(|h| h.file == "/doc1.md"));
        
        // Search for content from doc2
        let (hits, _) = engine.search(
            "oranges", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        assert!(hits.len() >= 1);
        assert!(hits.iter().any(|h| h.file == "/doc2.md"));
        
        // Search all files
        let (_hits, total) = engine.search(
            "content", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        assert!(total >= 2); // At least 2 "content" matches (doc1 has 2, doc2 has 1)
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_destroy_removes_directory() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_destroy");
        let _ = fs::remove_dir_all(&temp_dir);
        
        // Create index
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        let sections = vec![
            ("0".to_string(), "Header".to_string(), "Test content".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        drop(engine);
        
        // Verify directory exists
        assert!(temp_dir.exists());
        
        // Destroy the index
        SearchEngine::destroy(&temp_dir).expect("Failed to destroy");
        
        // Verify directory is removed
        assert!(!temp_dir.exists());
        
        // No cleanup needed - directory should be gone
    }

    #[test]
    fn test_snippet_generation_with_match() {
        let snippet = generate_snippet("test", "This is a test body with some content here", 30);
        
        // Should contain the matched term
        assert!(snippet.contains("test") || snippet.contains("Test"));
        
        // Should be a reasonable length
        assert!(snippet.len() > 0);
        assert!(snippet.len() <= 70); // 30 chars on each side + "test"
    }

    #[test]
    fn test_snippet_generation_no_match() {
        let snippet = generate_snippet("nonexistent", "This is a body with different content", 20);
        
        // When no match, should return beginning of body
        assert!(snippet.starts_with("This is a body"));
    }

    #[test]
    fn test_snippet_generation_empty_body() {
        let snippet = generate_snippet("test", "", 20);
        
        // Should handle empty body gracefully
        assert_eq!(snippet, "");
    }

    #[test]
    fn test_snippet_generation_empty_query() {
        let snippet = generate_snippet("", "This is a test body", 20);
        
        // Should handle empty query - typically returns beginning
        let result = snippet.is_empty() || snippet.starts_with("This is a test");
        assert!(result, "Snippet should be empty or start with body text");
    }

    #[test]
    fn test_open_same_directory_twice() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_reopen");
        let _ = fs::remove_dir_all(&temp_dir);
        
        // Open and create index
        let engine1 = SearchEngine::open(&temp_dir).expect("Failed to open engine first time");
        
        // Index some data
        let sections = vec![
            ("0".to_string(), "Header".to_string(), "Original content".to_string(), 1),
        ];
        engine1.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        drop(engine1);
        
        // Re-open the same directory
        let engine2 = SearchEngine::open(&temp_dir).expect("Failed to open engine second time");
        
        // Should be able to search for previously indexed content
        let (hits, total) = engine2.search(
            "original", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::All, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, "/test/file.md");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_body_scope() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_body_scope");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        let sections = vec![
            ("0".to_string(), "HeaderTitle".to_string(), "unique_body_content xyz".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        
        // Search with Body scope - should match body content
        let (hits, total) = engine.search(
            "unique_body_content", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::Body, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_headers_scope() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_headers_scope");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        let sections = vec![
            ("0".to_string(), "UniqueHeader123".to_string(), "body content".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        
        // Search with Headers scope - should match header content
        let (hits, total) = engine.search(
            "UniqueHeader123", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::Headers, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].section_title, "UniqueHeader123");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_code_scope() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_code_scope");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");
        
        let sections = vec![
            ("0".to_string(), "Code".to_string(), "fn unique_code_function() {}".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");
        
        // Search with Code scope - should match body content (code blocks are indexed in body)
        let (hits, total) = engine.search(
            "unique_code_function", 
            &crate::cli::MatchMode::Text, 
            &crate::cli::SearchScope::Code, 
            10, 
            0
        ).expect("Search failed");
        
        assert_eq!(total, 1);
        assert_eq!(hits.len(), 1);
        
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_search_with_frontmatter_scope() {
        let temp_dir = std::env::temp_dir().join("mdai_eng_test_frontmatter_scope");
        let _ = fs::remove_dir_all(&temp_dir);

        let engine = SearchEngine::open(&temp_dir).expect("Failed to open engine");

        let sections = vec![
            ("0".to_string(), "Doc".to_string(), "unique_frontmatter_value data".to_string(), 1),
        ];
        engine.index_file("/test/file.md", &sections, 0).expect("Failed to index");

        // Frontmatter scope returns empty query (not indexed in Tantivy)
        let (hits, total) = engine.search(
            "unique_frontmatter_value",
            &crate::cli::MatchMode::Text,
            &crate::cli::SearchScope::Frontmatter,
            10,
            0
        ).expect("Search failed");

        assert_eq!(total, 0);
        assert_eq!(hits.len(), 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
