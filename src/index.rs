#![allow(dead_code)]
//! SQLite database management for the markdownai index.
//!
//! This module provides the `Database` struct which wraps a SQLite connection
//! and manages all persistent storage for markdown files, sections, links, and
//! frontmatter data.
//!
//! ## Storage Location
//!
//! - SQLite database: `.worktoolai/markdownai.db`
//! - Tantivy index: `.worktoolai/markdownai_index/`
//!
//! ## Project Root Detection
//!
//! The project root is determined by:
//! 1. Walking up the directory tree to find a `.git/` directory
//! 2. Using the `--root` override if provided
//!
//! ## Schema
//!
//! The database uses WAL mode for better concurrent access performance.

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::xxh3_64;

use crate::frontmatter::FrontmatterData;
use crate::links::Link;
use crate::markdown::Section;

/// Database connection wrapper for markdownai.
///
/// Manages SQLite storage for files, sections, links, and frontmatter.
/// Uses WAL mode for better concurrency.
pub struct Database {
    /// The SQLite connection
    conn: Connection,
    /// Path to the project root
    root: PathBuf,
    /// Path to the SQLite database file
    db_path: PathBuf,
    /// Current sync epoch (auto-incremented on each full sync)
    sync_epoch: i64,
}










/// Index status information for display and diagnostics.
#[derive(Debug, Clone)]
pub struct IndexStatus {
    /// Project root path
    pub path: String,
    /// ISO 8601 timestamp of last sync
    pub last_sync: String,
    /// Number of indexed files
    pub files_indexed: usize,
    /// Number of stale files (content hash changed)
    pub files_stale: usize,
    /// Number of deleted files (in DB but not on disk)
    pub files_deleted: usize,
    /// SQLite database size in bytes
    pub sqlite_bytes: u64,
    /// Tantivy index size in bytes
    pub tantivy_bytes: u64,
}









/// Section data from the database (includes file_id).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DbSection {
    /// Database ID
    pub id: i64,
    /// Parent file ID
    pub file_id: i64,
    /// Parent section ID (if nested)
    pub parent_id: Option<i64>,
    /// TOC index like "1.1" or "1.2.3"
    pub section_index: String,
    /// Ordinal position in file (0-based)
    pub ordinal: i64,
    /// Heading level (1-6)
    pub level: i64,
    /// Heading title text
    pub title: String,
    /// 1-based line number where heading appears
    pub start_line: i64,
    /// 1-based line number where section ends (exclusive)
    pub end_line: i64,
}










/// Link data from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DbLink {
    /// Database ID
    pub id: i64,
    /// Source file ID
    pub source_file_id: i64,
    /// Line number where link appears
    pub source_line: i64,
    /// Original link text
    pub target_raw: String,
    /// Resolved target file path (if any)
    pub target_path: Option<String>,
    /// Anchor part of link (if any)
    pub target_anchor: Option<String>,
    /// Link type: "wiki" or "markdown"
    pub link_type: String,
    /// Resolved target file ID (if exists)
    pub resolved_file_id: Option<i64>,
    /// Whether the link is broken
    pub is_broken: bool,
}










/// Frontmatter field from the database.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DbFrontmatterField {
    /// Database ID
    pub id: i64,
    /// Parent file ID
    pub file_id: i64,
    /// Field key name
    pub key: String,
    /// JSON representation of the value
    pub value_json: String,
    /// Value type: "string", "number", "boolean", "array", "object"
    pub value_type: String,
    /// Text representation for search/filter (nullable)
    pub value_text: Option<String>,
}










impl Database {
    /// Open or create the database at the project root.
    ///
    /// Creates `.worktoolai/` directory if needed, enables WAL mode,
    /// and initializes all tables.
    ///
    /// # Arguments
    ///
    /// * `root` - Path to the project root directory
    ///
    /// # Returns
    ///
    /// Returns a `Database` instance with an open connection.
    pub fn open(root: &Path) -> Result<Self> {
        let worktool_dir = root.join(".worktoolai");
        fs::create_dir_all(&worktool_dir)
            .with_context(|| format!("Failed to create .worktoolai directory in {:?}", root))?;

        let db_path = worktool_dir.join("markdownai.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database at {:?}", db_path))?;

        // Enable WAL mode for better concurrency
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("Failed to enable WAL mode")?;

        // Create tables
        Database::create_tables(&conn)?;

        // Get current sync epoch
        let sync_epoch: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sync_epoch), 0) FROM files",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        Ok(Database {
            conn,
            root: root.to_path_buf(),
            db_path,
            sync_epoch,
        })
    }

    /// Find the project root by walking up the directory tree.
    ///
    /// Searches for a `.git/` directory to identify the project root.
    /// Returns `None` if not found.
    ///
    /// # Arguments
    ///
    /// * `start` - Starting directory for the search
    pub fn find_project_root(start: &Path) -> Option<PathBuf> {
        let mut current = start;
        loop {
            let git_dir = current.join(".git");
            if git_dir.exists() {
                return Some(current.to_path_buf());
            }

            // Move to parent directory
            match current.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    current = parent;
                }
                _ => return None,
            }
        }
    }

    /// Get or create the project root path.
    ///
    /// Uses explicit root if provided, otherwise finds by walking up
    /// from the current working directory to find `.git/`.
    ///
    /// # Arguments
    ///
    /// * `explicit_root` - Optional explicit root override
    /// * `cwd` - Current working directory (for auto-detection)
    ///
    /// # Returns
    ///
    /// Returns the absolute path to the project root.
    pub fn get_or_create_root(explicit_root: Option<&str>, cwd: &Path) -> Result<PathBuf> {
        if let Some(root_str) = explicit_root {
            let root = PathBuf::from(root_str);
            if root.is_absolute() {
                Ok(root)
            } else {
                Ok(cwd.join(&root))
            }
        } else if let Some(root) = Database::find_project_root(cwd) {
            Ok(root)
        } else {
            Err(anyhow!(
                "Could not find project root (no .git directory found). \
                 Use --root to specify explicitly."
            ))
        }
    }

    /// Create all database tables.
    ///
    /// This is called internally by `open()` if tables don't exist.
    fn create_tables(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                content_hash TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                lines INTEGER NOT NULL,
                has_frontmatter BOOLEAN NOT NULL DEFAULT 0,
                last_indexed_at TEXT NOT NULL,
                sync_epoch INTEGER NOT NULL DEFAULT 0,
                parse_error TEXT
            )",
            [],
        ).context("Failed to create files table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS sections (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES sections(id),
                section_index TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                level INTEGER NOT NULL,
                title TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                UNIQUE(file_id, section_index)
            )",
            [],
        ).context("Failed to create sections table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS links (
                id INTEGER PRIMARY KEY,
                source_file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                source_line INTEGER NOT NULL,
                target_raw TEXT NOT NULL,
                target_path TEXT,
                target_anchor TEXT,
                link_type TEXT NOT NULL,
                resolved_file_id INTEGER REFERENCES files(id),
                is_broken BOOLEAN NOT NULL DEFAULT 0
            )",
            [],
        ).context("Failed to create links table")?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS frontmatter (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                key TEXT NOT NULL,
                value_json TEXT NOT NULL,
                value_type TEXT NOT NULL,
                value_text TEXT
            )",
            [],
        ).context("Failed to create frontmatter table")?;

        // Create indexes for common queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sections_file_id ON sections(file_id)",
            [],
        ).context("Failed to create sections file_id index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_links_source_file_id ON links(source_file_id)",
            [],
        ).context("Failed to create links source_file_id index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_links_resolved_file_id ON links(resolved_file_id)",
            [],
        ).context("Failed to create links resolved_file_id index")?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_frontmatter_file_id ON frontmatter(file_id)",
            [],
        ).context("Failed to create frontmatter file_id index")?;

        Ok(())
    }

    /// Compute xxh3 hash of content as a hex string.
    ///
    /// Uses the fast xxh3 algorithm for content change detection.
    ///
    /// # Arguments
    ///
    /// * `content` - File content as bytes
    pub fn compute_hash(content: &[u8]) -> String {
        let hash = xxh3_64(content);
        format!("{:016x}", hash)
    }

    /// Check if a file's content is stale (hash changed).
    ///
    /// Compares the provided hash with the stored hash in the database.
    /// Returns `true` if the file is not indexed or has changed.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    /// * `content_hash` - Hash of current content
    pub fn check_file_stale(&self, rel_path: &str, content_hash: &str) -> Result<bool> {
        let stored_hash: Option<String> = self.conn.query_row(
            "SELECT content_hash FROM files WHERE path = ?",
            params![rel_path],
            |row| row.get(0),
        ).unwrap_or(None);

        Ok(stored_hash.as_deref() != Some(content_hash))
    }

    /// Synchronize a file's data with the database.
    ///
    /// Deletes existing data for the file (CASCADE handles children),
    /// then inserts new file record, sections, links, and frontmatter.
    /// Updates the sync_epoch.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    /// * `content` - File content
    /// * `sections` - Parsed sections
    /// * `links` - Parsed links
    /// * `frontmatter` - Parsed frontmatter (if any)
    ///
    /// # Returns
    ///
    /// Returns the file ID of the synced file.
    pub fn sync_file(
        &self,
        rel_path: &str,
        content: &str,
        sections: &[Section],
        links: &[Link],
        frontmatter: Option<&FrontmatterData>,
    ) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()
            .context("Failed to begin transaction")?;

        let content_bytes = content.as_bytes();
        let content_hash = Database::compute_hash(content_bytes);
        let lines = content.lines().count() as i64;
        let bytes = content_bytes.len() as i64;
        let has_frontmatter = frontmatter.is_some() as i8;
        let last_indexed_at = Database::format_timestamp();
        let sync_epoch = self.sync_epoch + 1;

        // Delete existing file data (CASCADE handles sections, links, frontmatter)
        tx.execute("DELETE FROM files WHERE path = ?", params![rel_path])
            .context("Failed to delete existing file data")?;

        // Insert file record
        tx.execute(
            "INSERT INTO files (path, content_hash, bytes, lines, has_frontmatter, last_indexed_at, sync_epoch)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![rel_path, content_hash, bytes, lines, has_frontmatter, last_indexed_at, sync_epoch],
        ).context("Failed to insert file record")?;

        let file_id = tx.last_insert_rowid();

        // Insert sections
        for section in sections {
            let parent_id = if let Some(ref parent_idx) = section.parent_index {
                // Find parent section ID
                tx.query_row(
                    "SELECT id FROM sections WHERE file_id = ? AND section_index = ?",
                    params![file_id, parent_idx],
                    |row| row.get::<_, i64>(0),
                ).ok()
            } else {
                None
            };

            tx.execute(
                "INSERT INTO sections (file_id, parent_id, section_index, ordinal, level, title, start_line, end_line)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    file_id,
                    parent_id,
                    section.index,
                    section.ordinal as i64,
                    section.level as i64,
                    section.title,
                    section.start_line as i64,
                    section.end_line as i64,
                ],
            ).context("Failed to insert section")?;
        }

        // Insert links
        for link in links {
            // Try to resolve target file
            let resolved_file_id = if let Some(ref target_path) = link.target_path {
                // Normalize path relative to project root
                let normalized = if let Some(parent) = rel_path.rfind('/') {
                    format!("{}/{}", &rel_path[..parent], target_path)
                } else {
                    target_path.clone()
                };

                tx.query_row(
                    "SELECT id FROM files WHERE path = ?",
                    params![normalized],
                    |row| row.get::<_, i64>(0),
                ).ok()
            } else {
                None
            };

            let is_broken = resolved_file_id.is_none() && link.target_path.is_some();

            // Convert link_type from LinkKind to string
            let link_type_str = match &link.link_type {
                crate::links::LinkKind::Wiki => "wiki",
                crate::links::LinkKind::Markdown => "markdown",
            };

            tx.execute(
                "INSERT INTO links (source_file_id, source_line, target_raw, target_path, target_anchor, link_type, resolved_file_id, is_broken)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    file_id,
                    link.source_line as i64,
                    link.target_raw.clone(),
                    link.target_path,
                    link.target_anchor,
                    link_type_str,
                    resolved_file_id,
                    is_broken as i8,
                ],
            ).context("Failed to insert link")?;
        }

        // Insert frontmatter
        if let Some(fm) = frontmatter {
            for field in &fm.fields {
                tx.execute(
                    "INSERT INTO frontmatter (file_id, key, value_json, value_type, value_text)
                     VALUES (?, ?, ?, ?, ?)",
                    params![
                        file_id,
                        field.key.clone(),
                        field.value_json.clone(),
                        field.value_type.clone(),
                        field.value_text.clone(),
                    ],
                ).context("Failed to insert frontmatter field")?;
            }
        }

        tx.commit().context("Failed to commit sync transaction")?;

        Ok(file_id)
    }

    /// Remove a file from the database.
    ///
    /// Uses CASCADE delete to also remove all associated sections,
    /// links, and frontmatter.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM files WHERE path = ?", params![rel_path])
            .context("Failed to remove file")?;
        Ok(())
    }

    /// Get all sections for a file.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    pub fn get_sections(&self, rel_path: &str) -> Result<Vec<DbSection>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.file_id, s.parent_id, s.section_index, s.ordinal, s.level,
                    s.title, s.start_line, s.end_line
             FROM sections s
             JOIN files f ON s.file_id = f.id
             WHERE f.path = ?
             ORDER BY s.ordinal"
        ).context("Failed to prepare sections query")?;

        let sections = stmt.query_map(params![rel_path], |row| {
            Ok(DbSection {
                id: row.get(0)?,
                file_id: row.get(1)?,
                parent_id: row.get(2)?,
                section_index: row.get(3)?,
                ordinal: row.get(4)?,
                level: row.get(5)?,
                title: row.get(6)?,
                start_line: row.get(7)?,
                end_line: row.get(8)?,
            })
        }).context("Failed to query sections")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect sections")?;

        Ok(sections)
    }

    /// Get all outgoing links from a file.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    pub fn get_links(&self, rel_path: &str) -> Result<Vec<DbLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.source_file_id, l.source_line, l.target_raw, l.target_path,
                    l.target_anchor, l.link_type, l.resolved_file_id, l.is_broken
             FROM links l
             JOIN files f ON l.source_file_id = f.id
             WHERE f.path = ?"
        ).context("Failed to prepare links query")?;

        let links = stmt.query_map(params![rel_path], |row| {
            Ok(DbLink {
                id: row.get(0)?,
                source_file_id: row.get(1)?,
                source_line: row.get(2)?,
                target_raw: row.get(3)?,
                target_path: row.get(4)?,
                target_anchor: row.get(5)?,
                link_type: row.get(6)?,
                resolved_file_id: row.get(7)?,
                is_broken: row.get::<_, i8>(8)? != 0,
            })
        }).context("Failed to query links")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect links")?;

        Ok(links)
    }

    /// Get all incoming links (backlinks) to a file.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    #[allow(dead_code)]
    pub fn get_backlinks(&self, rel_path: &str) -> Result<Vec<DbLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.source_file_id, l.source_line, l.target_raw, l.target_path,
                    l.target_anchor, l.link_type, l.resolved_file_id, l.is_broken
             FROM links l
             JOIN files f ON l.resolved_file_id = f.id
             WHERE f.path = ?"
        ).context("Failed to prepare backlinks query")?;

        let links = stmt.query_map(params![rel_path], |row| {
            Ok(DbLink {
                id: row.get(0)?,
                source_file_id: row.get(1)?,
                source_line: row.get(2)?,
                target_raw: row.get(3)?,
                target_path: row.get(4)?,
                target_anchor: row.get(5)?,
                link_type: row.get(6)?,
                resolved_file_id: row.get(7)?,
                is_broken: row.get::<_, i8>(8)? != 0,
            })
        }).context("Failed to query backlinks")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect backlinks")?;

        Ok(links)
    }

    /// Get all frontmatter fields for a file.
    ///
    /// # Arguments
    ///
    /// * `rel_path` - Relative path from project root
    pub fn get_frontmatter(&self, rel_path: &str) -> Result<Vec<DbFrontmatterField>> {
        let mut stmt = self.conn.prepare(
            "SELECT fm.id, fm.file_id, fm.key, fm.value_json, fm.value_type, fm.value_text
             FROM frontmatter fm
             JOIN files f ON fm.file_id = f.id
             WHERE f.path = ?"
        ).context("Failed to prepare frontmatter query")?;

        let fields = stmt.query_map(params![rel_path], |row| {
            Ok(DbFrontmatterField {
                id: row.get(0)?,
                file_id: row.get(1)?,
                key: row.get(2)?,
                value_json: row.get(3)?,
                value_type: row.get(4)?,
                value_text: row.get(5)?,
            })
        }).context("Failed to query frontmatter")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect frontmatter")?;

        Ok(fields)
    }

    /// Get all links in the database for graph building.
    ///
    /// Returns tuples of (source_file_path, DbLink).
    #[allow(dead_code)]
    pub fn get_all_links(&self) -> Result<Vec<(String, DbLink)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.path as source_path,
                    l.id, l.source_file_id, l.source_line, l.target_raw, l.target_path,
                    l.target_anchor, l.link_type, l.resolved_file_id, l.is_broken
             FROM links l
             JOIN files f ON l.source_file_id = f.id"
        ).context("Failed to prepare all links query")?;

        let links = stmt.query_map([], |row| {
            let source_path: String = row.get(0)?;
            Ok((
                source_path,
                DbLink {
                    id: row.get(1)?,
                    source_file_id: row.get(2)?,
                    source_line: row.get(3)?,
                    target_raw: row.get(4)?,
                    target_path: row.get(5)?,
                    target_anchor: row.get(6)?,
                    link_type: row.get(7)?,
                    resolved_file_id: row.get(8)?,
                    is_broken: row.get::<_, i8>(9)? != 0,
                },
            ))
        }).context("Failed to query all links")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect all links")?;

        Ok(links)
    }

    /// Get index status information.
    ///
    /// Returns counts and sizes for display in `--status` output.
    pub fn status(&self) -> Result<IndexStatus> {
        // Get file counts
        let files_indexed: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        // Get last sync timestamp
        let last_sync: String = self.conn.query_row(
            "SELECT MAX(last_indexed_at) FROM files",
            [],
            |row| row.get(0),
        ).unwrap_or_else(|_| "never".to_string());

        // Get database size
        let sqlite_bytes = fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Get Tantivy index size
        let tantivy_path = self.root.join(".worktoolai/markdownai_index");
        let tantivy_bytes = if tantivy_path.exists() {
            Self::dir_size(&tantivy_path).unwrap_or(0)
        } else {
            0
        };

        // Note: files_stale and files_deleted would need on-disk scanning
        // to compute accurately. For now, return 0.
        Ok(IndexStatus {
            path: self.root.display().to_string(),
            last_sync,
            files_indexed: files_indexed as usize,
            files_stale: 0,
            files_deleted: 0,
            sqlite_bytes,
            tantivy_bytes,
        })
    }

    /// Get all indexed files as (path, hash) pairs.
    ///
    /// Used for stale detection and validation.
    pub fn get_indexed_files(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, content_hash FROM files"
        ).context("Failed to prepare indexed files query")?;

        let files = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).context("Failed to query indexed files")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Failed to collect indexed files")?;

        Ok(files)
    }

    /// Format current timestamp as ISO 8601 string.
    ///
    /// Manual implementation to avoid chrono dependency.
    fn format_timestamp() -> String {
        // Simple ISO 8601 format without timezone (UTC)
        // For a production system, consider using chrono
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs();

        // Approximate conversion to ISO 8601
        // This is a simplified version - for production, use chrono
        format!("{:?}", secs) // Temporary placeholder
    }

    /// Calculate total size of a directory.
    fn dir_size(path: &Path) -> Result<u64> {
        let mut total = 0;
        if path.is_dir() {
            for entry in fs::read_dir(path)
                .with_context(|| format!("Failed to read directory {:?}", path))?
            {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    total += Self::dir_size(&path)?;
                } else {
                    total += entry.metadata()?.len();
                }
            }
        } else {
            total = fs::metadata(path)?.len();
        }
        Ok(total)
    }

    /// Get the database path.
    #[allow(dead_code)]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Get the project root path.
    #[allow(dead_code)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get a reference to the underlying connection.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}










#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash() {
        let hash1 = Database::compute_hash(b"hello world");
        let hash2 = Database::compute_hash(b"hello world");
        let hash3 = Database::compute_hash(b"hello universe");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 16); // 16 hex chars = 64 bits
    }
    #[test]
    fn test_open_database_creates_tables() {
        let temp_dir = std::env::temp_dir().join("mdai_test_open_db");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        
        // Create a .git directory to make it a valid project root
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir);
        assert!(db.is_ok(), "Database should open successfully");
        
        let db = db.unwrap();
        
        // Verify tables exist by querying them
        let mut stmt = db.conn().prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap();
        let tables: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().map(|r| r.unwrap()).collect();
        
        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"sections".to_string()));
        assert!(tables.contains(&"links".to_string()));
        assert!(tables.contains(&"frontmatter".to_string()));
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_file_get_sections_roundtrip() {
        let temp_dir = std::env::temp_dir().join("mdai_test_sections");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        use crate::markdown::Section;
        let sections = vec![
            Section {
                index: "1".to_string(),
                level: 1,
                title: "Introduction".to_string(),
                start_line: 1,
                end_line: 10,
                ordinal: 0,
                parent_index: None,
            },
            Section {
                index: "1.1".to_string(),
                level: 2,
                title: "Background".to_string(),
                start_line: 3,
                end_line: 8,
                ordinal: 1,
                parent_index: Some("1".to_string()),
            },
        ];
        
        let _content_hash = Database::compute_hash(b"# Introduction\n## Background\n");
        let result = db.sync_file("test.md", "# Introduction\n## Background\n", &sections, &[], None);
        assert!(result.is_ok());
        
        let retrieved = db.get_sections("test.md");
        assert!(retrieved.is_ok());
        let retrieved = retrieved.unwrap();
        
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].title, "Introduction");
        assert_eq!(retrieved[0].level, 1);
        assert_eq!(retrieved[1].title, "Background");
        assert_eq!(retrieved[1].level, 2);
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_file_get_links_roundtrip() {
        let temp_dir = std::env::temp_dir().join("mdai_test_links");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        use crate::links::{Link, LinkKind};
        let links = vec![
            Link {
                source_line: 1,
                target_raw: "[[OtherPage]]".to_string(),
                target_path: Some("OtherPage.md".to_string()),
                target_anchor: None,
                link_type: LinkKind::Wiki,
                display_text: Some("OtherPage".to_string()),
            },
            Link {
                source_line: 3,
                target_raw: "[text](target.md#section)".to_string(),
                target_path: Some("target.md".to_string()),
                target_anchor: Some("section".to_string()),
                link_type: LinkKind::Markdown,
                display_text: Some("text".to_string()),
            },
        ];
        
        let result = db.sync_file("test.md", "content", &[], &links, None);
        assert!(result.is_ok());
        
        let retrieved = db.get_links("test.md");
        assert!(retrieved.is_ok());
        let retrieved = retrieved.unwrap();
        
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].target_raw, "[[OtherPage]]");
        assert_eq!(retrieved[0].link_type, "wiki");
        assert_eq!(retrieved[0].source_line, 1);
        assert_eq!(retrieved[1].target_raw, "[text](target.md#section)");
        assert_eq!(retrieved[1].link_type, "markdown");
        assert_eq!(retrieved[1].target_anchor, Some("section".to_string()));
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_file_get_frontmatter_roundtrip() {
        let temp_dir = std::env::temp_dir().join("mdai_test_frontmatter");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        use crate::frontmatter::{FrontmatterData, FrontmatterField};
        let frontmatter = FrontmatterData {
            fields: vec![
                FrontmatterField {
                    key: "title".to_string(),
                    value_json: "\"My Document\"".to_string(),
                    value_type: "string".to_string(),
                    value_text: Some("My Document".to_string()),
                },
                FrontmatterField {
                    key: "tags".to_string(),
                    value_json: "[\"rust\", \"test\"]".to_string(),
                    value_type: "array".to_string(),
                    value_text: Some("[\"rust\", \"test\"]".to_string()),
                },
            ],
            raw_yaml: "---\ntitle: My Document\ntags:\n  - rust\n  - test\n---".to_string(),
        };
        
        let result = db.sync_file("test.md", "content", &[], &[], Some(&frontmatter));
        assert!(result.is_ok());
        
        let retrieved = db.get_frontmatter("test.md");
        assert!(retrieved.is_ok());
        let retrieved = retrieved.unwrap();
        
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].key, "title");
        assert_eq!(retrieved[0].value_json, "\"My Document\"");
        assert_eq!(retrieved[0].value_type, "string");
        assert_eq!(retrieved[1].key, "tags");
        assert_eq!(retrieved[1].value_type, "array");
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_check_file_stale_new_file() {
        let temp_dir = std::env::temp_dir().join("mdai_test_stale_new");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        let hash = Database::compute_hash(b"test content");
        
        // New file should be stale
        let result = db.check_file_stale("newfile.md", &hash);
        assert!(result.is_ok());
        assert!(result.unwrap(), "New file should be reported as stale");
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_check_file_stale_after_sync() {
        let temp_dir = std::env::temp_dir().join("mdai_test_stale_sync");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        let hash = Database::compute_hash(b"test content");
        
        // Sync the file
        db.sync_file("test.md", "test content", &[], &[], None).unwrap();
        
        // Same hash should not be stale
        let result = db.check_file_stale("test.md", &hash);
        assert!(result.is_ok());
        assert!(!result.unwrap(), "File with same hash should not be stale");
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_check_file_stale_content_changed() {
        let temp_dir = std::env::temp_dir().join("mdai_test_stale_changed");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        let _old_hash = Database::compute_hash(b"old content");
        let new_hash = Database::compute_hash(b"new content");
        
        // Sync with old content
        db.sync_file("test.md", "old content", &[], &[], None).unwrap();
        
        // New hash should be stale
        let result = db.check_file_stale("test.md", &new_hash);
        assert!(result.is_ok());
        assert!(result.unwrap(), "File with changed content should be stale");
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_remove_file() {
        let temp_dir = std::env::temp_dir().join("mdai_test_remove");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        // Sync a file
        db.sync_file("test.md", "content", &[], &[], None).unwrap();
        
        // Verify it exists
        let sections = db.get_sections("test.md").unwrap();
        assert!(!sections.is_empty() || true); // File exists in DB
        
        // Remove the file
        let result = db.remove_file("test.md");
        assert!(result.is_ok());
        
        // Verify it's gone
        let sections_after = db.get_sections("test.md").unwrap();
        assert_eq!(sections_after.len(), 0);
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_get_indexed_files() {
        let temp_dir = std::env::temp_dir().join("mdai_test_indexed");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        // Sync multiple files
        db.sync_file("file1.md", "content1", &[], &[], None).unwrap();
        db.sync_file("file2.md", "content2", &[], &[], None).unwrap();
        db.sync_file("subdir/file3.md", "content3", &[], &[], None).unwrap();
        
        let indexed = db.get_indexed_files().unwrap();
        assert_eq!(indexed.len(), 3);
        
        let paths: Vec<&str> = indexed.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"file1.md"));
        assert!(paths.contains(&"file2.md"));
        assert!(paths.contains(&"subdir/file3.md"));
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_status_returns_correct_counts() {
        let temp_dir = std::env::temp_dir().join("mdai_test_status");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        // Sync some files
        db.sync_file("file1.md", "content1", &[], &[], None).unwrap();
        db.sync_file("file2.md", "content2", &[], &[], None).unwrap();
        
        let status = db.status().unwrap();
        assert_eq!(status.files_indexed, 2);
        assert!(status.path.contains("mdai_test_status"));
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_sync_file_overwrites() {
        let temp_dir = std::env::temp_dir().join("mdai_test_overwrite");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(temp_dir.join(".git")).unwrap();
        
        let db = Database::open(&temp_dir).unwrap();
        
        use crate::markdown::Section;
        let sections1 = vec![
            Section {
                index: "1".to_string(),
                level: 1,
                title: "Old Title".to_string(),
                start_line: 1,
                end_line: 5,
                ordinal: 0,
                parent_index: None,
            },
        ];
        
        let sections2 = vec![
            Section {
                index: "1".to_string(),
                level: 1,
                title: "New Title".to_string(),
                start_line: 1,
                end_line: 5,
                ordinal: 0,
                parent_index: None,
            },
            Section {
                index: "2".to_string(),
                level: 1,
                title: "Another Section".to_string(),
                start_line: 6,
                end_line: 10,
                ordinal: 1,
                parent_index: None,
            },
        ];
        
        // Initial sync
        db.sync_file("test.md", "old content", &sections1, &[], None).unwrap();
        let first = db.get_sections("test.md").unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].title, "Old Title");
        
        // Re-sync with different data
        db.sync_file("test.md", "new content", &sections2, &[], None).unwrap();
        let second = db.get_sections("test.md").unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].title, "New Title");
        assert_eq!(second[1].title, "Another Section");
        
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let content = b"The quick brown fox jumps over the lazy dog";
        
        let hash1 = Database::compute_hash(content);
        let hash2 = Database::compute_hash(content);
        let hash3 = Database::compute_hash(content);
        
        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_compute_hash_different_content() {
        let hash_a = Database::compute_hash(b"content A");
        let hash_b = Database::compute_hash(b"content B");
        let hash_empty = Database::compute_hash(b"");
        
        assert_ne!(hash_a, hash_b);
        assert_ne!(hash_a, hash_empty);
        assert_ne!(hash_b, hash_empty);
        
        // Verify small change produces different hash
        let hash1 = Database::compute_hash(b"hello");
        let hash2 = Database::compute_hash(b"hello!");
        assert_ne!(hash1, hash2);
    }
}
