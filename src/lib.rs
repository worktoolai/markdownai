//! Markdown AI - Agent-first Markdown CLI
//!
//! This library provides comprehensive markdown processing capabilities including:
//! - YAML frontmatter parsing and manipulation
//! - Table of contents generation
//! - Full-text search
//! - Link graph analysis

pub mod cli;
pub mod engine;
pub mod frontmatter;
pub mod index;
pub mod links;
pub mod markdown;
pub mod output;
pub mod section;
