mod cli;
mod engine;
mod frontmatter;
mod index;
mod links;
mod manipulate;
mod markdown;
mod output;
mod section;

use std::io::{self, Read};
use std::path::Path;

use anyhow::{bail, Context, Result};
use clap::Parser;
use walkdir::WalkDir;

use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let exit_code = match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {:#}", e);
            2
        }
    };

    std::process::exit(exit_code);
}

fn run(cli: Cli) -> Result<i32> {
    let json = cli.json;
    let pretty = cli.pretty;
    let limit = cli.limit;
    let offset = cli.offset;
    let count_only = cli.count_only;
    let exists = cli.exists;
    let stats = cli.stats;
    let max_bytes = cli.max_bytes;
    let threshold = cli.threshold;
    let no_overflow = cli.no_overflow;
    let _plan_mode = cli.plan;
    let facets_field = cli.facets.clone();

    match cli.command {
        Commands::Toc(args) => run_toc(&args, json, pretty, limit, offset),
        Commands::Read(args) => run_read(&args, json, pretty, limit, offset, max_bytes, stats, exists),
        Commands::Tree(args) => run_tree(&args, json, pretty, stats),
        Commands::Search(args) => run_search(&args, json, pretty, limit, offset, max_bytes, count_only, exists, threshold, no_overflow),
        Commands::Frontmatter(args) => run_frontmatter(&args, json, pretty, limit, offset, count_only, facets_field),
        Commands::Links(args) => run_links(&args, json, pretty, limit, offset, count_only, exists),
        Commands::Backlinks(args) => run_backlinks(&args, json, pretty, limit, offset, count_only),
        Commands::Graph(args) => run_graph(&args, json, pretty, limit, offset),
        Commands::SectionSet(args) => run_section_set(&args),
        Commands::SectionAdd(args) => run_section_add(&args),
        Commands::SectionDelete(args) => run_section_delete(&args),
        Commands::FrontmatterSet(args) => run_frontmatter_set(&args),
        Commands::Index(args) => run_index(&args, json, pretty),
    }
}

fn run_toc(args: &cli::TocArgs, json: bool, pretty: bool, limit: usize, offset: usize) -> Result<i32> {
    let content = read_input(&args.file)?;
    let doc = markdown::parse_document(&content);

    if doc.sections.is_empty() {
        return Ok(1);
    }

    let mut sections: Vec<&markdown::Section> = doc.sections.iter().collect();
    if let Some(depth) = args.depth {
        sections.retain(|s| s.level <= depth);
    }

    let total = sections.len();
    let paged: Vec<&markdown::Section> = sections.into_iter().skip(offset).take(limit).collect();
    let returned = paged.len();

    if json {
        let entries: Vec<output::TocEntry> = paged.iter().map(|s| output::TocEntry {
            index: s.index.clone(),
            level: s.level,
            text: s.title.clone(),
            line: s.start_line,
        }).collect();
        let meta = output::Meta::paging(total, returned, offset, limit);
        let envelope = output::Envelope::with_results(meta, entries);
        println!("{}", output::to_json(&envelope, pretty));
    } else {
        for s in &paged {
            let indent = if args.flat { String::new() } else { "  ".repeat((s.level as usize).saturating_sub(1)) };
            println!("{}{:<8} {:<40} (L{})", indent, s.index, s.title, s.start_line);
        }
        if returned < total {
            println!("\n{}", output::format_raw_footer(returned, total, offset));
        }
    }

    Ok(0)
}

fn run_read(
    args: &cli::ReadArgs, json: bool, pretty: bool,
    limit: usize, offset: usize, max_bytes: Option<usize>,
    stats: bool, exists: bool,
) -> Result<i32> {
    let content = read_input(&args.file)?;

    if stats {
        let doc = markdown::parse_document(&content);
        let parsed_links = links::parse_links(&content);
        let code_blocks = content.matches("```").count() / 2;
        let wiki_count = parsed_links.iter().filter(|l| matches!(l.link_type, links::LinkKind::Wiki)).count();
        let md_count = parsed_links.iter().filter(|l| matches!(l.link_type, links::LinkKind::Markdown)).count();

        let file_stats = output::FileStats {
            file: args.file.clone(),
            bytes: content.len(),
            lines: content.lines().count(),
            sections: doc.sections.len(),
            code_blocks,
            has_frontmatter: doc.has_frontmatter,
            links: Some(output::LinkStats {
                wiki: Some(wiki_count),
                markdown: Some(md_count),
                total: Some(wiki_count + md_count),
            }),
        };

        if json {
            println!("{}", output::to_json(&file_stats, pretty));
        } else {
            println!("{}", file_stats.format_raw());
        }
        return Ok(0);
    }

    let doc = markdown::parse_document(&content);

    if exists {
        if let Some(ref section_addr) = args.section {
            let addr = section::parse_section_address(section_addr)?;
            return Ok(if section::resolve_section_address(&addr, &doc.sections).is_ok() { 0 } else { 1 });
        }
        return Ok(0);
    }

    if let Some(preview_lines) = args.summary {
        let previews = section::read_summary(&content, &doc.sections, preview_lines)?;
        let total = previews.len();
        let paged: Vec<_> = previews.into_iter().skip(offset).take(limit).collect();
        let returned = paged.len();

        if json {
            let summaries: Vec<output::SectionSummary> = paged.iter().map(|p| output::SectionSummary {
                index: p.index.clone(),
                title: p.title.clone(),
                line: p.line,
                preview: p.preview.clone(),
            }).collect();
            let env = output::SummaryEnvelope::new(args.file.clone(), total, returned, offset, summaries);
            println!("{}", output::to_json(&env, pretty));
        } else {
            for p in &paged {
                println!("{} (L{}, {})", p.title, p.line, p.index);
                for line in p.preview.lines().take(preview_lines) {
                    println!("  {}", line);
                }
                println!("  ...\n");
            }
            if returned < total {
                println!("{}", output::format_raw_footer(returned, total, offset));
            }
        }
        return Ok(0);
    }

    let text = if let Some(ref section_addr) = args.section {
        let addr = section::parse_section_address(section_addr)?;
        match addr {
            section::SectionAddress::LineRange(start, end) => {
                section::read_section_lines(&content, start, end)?
            }
            _ => {
                let target = section::resolve_section_address(&addr, &doc.sections)?;
                section::read_section_content(&content, target)?
            }
        }
    } else {
        content.clone()
    };

    let (output_text, truncated) = if let Some(budget) = max_bytes {
        if text.len() > budget {
            let truncated_text: String = text.chars().take(budget).collect();
            (truncated_text, true)
        } else {
            (text, false)
        }
    } else {
        (text, false)
    };

    if json {
        let mut result = output::ReadResult::new(args.file.clone(), output_text.clone());
        if let Some(ref section_addr) = args.section {
            let addr = section::parse_section_address(section_addr)?;
            if let Ok(target) = section::resolve_section_address(&addr, &doc.sections) {
                result = result.with_section(section_addr.to_string(), target.start_line, target.end_line);
            }
        }
        if truncated {
            let bytes_total = content.len();
            let next_line = output_text.lines().count() + 1;
            result = result.with_truncation(output_text.len(), bytes_total, next_line);
        }
        println!("{}", output::to_json(&result, pretty));
    } else {
        if args.meta {
            if let Some(fm) = frontmatter::parse_frontmatter(&content) {
                println!("---\n{}---\n", fm.raw_yaml);
            }
        }
        print!("{}", output_text);
        if truncated {
            let next_line = output_text.lines().count() + 1;
            println!("\n--- truncated at L{}, {}/{} bytes, next: --section \"L{}-\" ---",
                next_line, output_text.len(), content.len(), next_line);
        }
    }

    Ok(0)
}

fn run_tree(args: &cli::TreeArgs, json: bool, pretty: bool, stats: bool) -> Result<i32> {
    let path = Path::new(&args.path);
    if !path.is_dir() {
        bail!("{} is not a directory", args.path);
    }

    if stats {
        let mut files = 0usize;
        let mut total_bytes = 0usize;
        let mut total_lines = 0usize;
        let mut total_sections = 0usize;

        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("md") && entry.file_type().is_file() {
                files += 1;
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    total_bytes += content.len();
                    total_lines += content.lines().count();
                    let doc = markdown::parse_document(&content);
                    total_sections += doc.sections.len();
                }
            }
        }

        let dir_stats = output::DirStats {
            path: args.path.clone(),
            files,
            total_bytes,
            total_lines,
            total_sections,
        };

        if json {
            println!("{}", output::to_json(&dir_stats, pretty));
        } else {
            println!("{}", dir_stats.format_raw());
        }
        return Ok(0);
    }

    let max_depth = args.depth.unwrap_or(usize::MAX);
    let walker = WalkDir::new(path).max_depth(max_depth);

    if json {
        let mut entries: Vec<serde_json::Value> = Vec::new();
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if args.files_only && !p.is_file() { continue; }
            if p.extension().and_then(|e| e.to_str()) != Some("md") && p.is_file() { continue; }
            let rel = p.strip_prefix(path).unwrap_or(p);
            entries.push(serde_json::json!({
                "path": rel.display().to_string(),
                "type": if p.is_dir() { "dir" } else { "file" },
            }));
        }
        println!("{}", output::to_json(&entries, pretty));
    } else if args.count {
        let count = WalkDir::new(path).into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("md") && e.file_type().is_file())
            .count();
        println!("{} markdown files", count);
    } else {
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if args.files_only && !p.is_file() { continue; }
            let rel = p.strip_prefix(path).unwrap_or(p);
            let depth = entry.depth();
            let indent = "  ".repeat(depth);
            println!("{}{}", indent, rel.display());
        }
    }

    Ok(0)
}

fn run_search(
    args: &cli::SearchArgs, json: bool, pretty: bool,
    limit: usize, offset: usize, max_bytes: Option<usize>,
    count_only: bool, exists: bool,
    _threshold: usize, _no_overflow: bool,
) -> Result<i32> {
    let files = collect_md_files(&args.input)?;
    if files.is_empty() {
        return Ok(1);
    }

    let multi = args.query.len() > 1;

    if multi && count_only && json {
        let mut counts = Vec::new();
        for q in &args.query {
            let total = count_matches_in_files(&files, q)?;
            counts.push(output::CountResult { query: q.clone(), total });
        }
        let env = output::SearchEnvelope::count_only(counts);
        println!("{}", output::to_json(&env, pretty));
        return Ok(0);
    }

    if multi && count_only {
        for q in &args.query {
            let total = count_matches_in_files(&files, q)?;
            println!("{}: {}", q, total);
        }
        return Ok(0);
    }

    for q in &args.query {
        let results = search_in_files(&files, q, args.context)?;
        let total = results.len();

        if exists {
            return Ok(if total > 0 { 0 } else { 1 });
        }

        if count_only {
            if json {
                println!("{}", output::to_json(&serde_json::json!({"meta":{"query": q, "total": total}}), pretty));
            } else {
                println!("{}: {}", q, total);
            }
            continue;
        }

        let paged: Vec<_> = results.into_iter().skip(offset).take(limit).collect();
        let returned = paged.len();

        if json {
            let (items, _truncated) = output::truncate_to_budget(&paged, max_bytes);
            let env = output::SearchEnvelope::single_query(
                q.clone(), total, items.len(), offset, limit,
                total > limit, items,
            );
            println!("{}", output::to_json(&env, pretty));
        } else {
            for r in &paged {
                let sec_info = r.section_index.as_deref().unwrap_or("");
                let sec_title = r.section_title.as_deref().unwrap_or("");
                println!("{}:{} {} (L{}, score:{:.2})", r.file, sec_info, sec_title, r.line, r.score);
                println!("  {}\n", r.snippet);
            }
            if returned < total {
                println!("{}", output::format_raw_footer(returned, total, offset));
            }
        }
    }

    Ok(0)
}

fn collect_md_files(input: &str) -> Result<Vec<String>> {
    if input == "-" {
        return Ok(vec!["-".to_string()]);
    }
    let path = Path::new(input);
    if path.is_file() {
        return Ok(vec![input.to_string()]);
    }
    if path.is_dir() {
        let mut files = Vec::new();
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("md") && entry.file_type().is_file() {
                files.push(entry.path().to_string_lossy().to_string());
            }
        }
        return Ok(files);
    }
    bail!("Input '{}' is not a file or directory", input);
}

fn count_matches_in_files(files: &[String], query: &str) -> Result<usize> {
    let mut total = 0;
    let query_lower = query.to_lowercase();
    for file in files {
        let content = if file == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            std::fs::read_to_string(file).unwrap_or_default()
        };
        total += content.to_lowercase().matches(&query_lower).count();
    }
    Ok(total)
}

fn search_in_files(
    files: &[String], query: &str, context_lines: usize,
) -> Result<Vec<output::SearchResult>> {
    let mut results = Vec::new();
    let query_lower = query.to_lowercase();

    for file_path in files {
        let content = if file_path == "-" {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            }
        };

        let doc = markdown::parse_document(&content);
        let lines: Vec<&str> = content.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&query_lower) {
                let line_num = i + 1;
                let sec = doc.sections.iter().find(|s| s.start_line <= line_num && s.end_line >= line_num);

                let start = i.saturating_sub(context_lines);
                let end = (i + context_lines + 1).min(lines.len());
                let snippet = lines[start..end].join("\n");

                results.push(output::SearchResult {
                    file: file_path.clone(),
                    section_index: sec.map(|s| s.index.clone()),
                    section_title: sec.map(|s| s.title.clone()),
                    line: line_num,
                    snippet,
                    score: 1.0,
                });
            }
        }
    }

    Ok(results)
}

fn run_frontmatter(
    args: &cli::FrontmatterArgs, json: bool, pretty: bool,
    limit: usize, offset: usize, count_only: bool,
    facets_field: Option<String>,
) -> Result<i32> {
    let files = collect_md_files(&args.input)?;

    if let Some(ref facet_key) = facets_field {
        let mut facet_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut total_files = 0;
        for file in &files {
            if let Ok(content) = std::fs::read_to_string(file) {
                if let Some(fm) = frontmatter::parse_frontmatter(&content) {
                    total_files += 1;
                    if let Some(field) = frontmatter::get_field(&fm, facet_key) {
                        let val: serde_json::Value = serde_json::from_str(&field.value_json).unwrap_or_default();
                        match val {
                            serde_json::Value::Array(arr) => {
                                for item in arr {
                                    if let Some(s) = item.as_str() {
                                        *facet_counts.entry(s.to_string()).or_insert(0) += 1;
                                    }
                                }
                            }
                            serde_json::Value::String(s) => {
                                *facet_counts.entry(s).or_insert(0) += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let mut facets_result = output::FacetsResult::new(facet_key.clone(), total_files);
        for (k, v) in facet_counts {
            facets_result.add(k, v);
        }

        if json {
            println!("{}", output::to_json(&facets_result, pretty));
        } else {
            println!("{}", facets_result.format_raw());
        }
        return Ok(0);
    }

    let mut all_entries = Vec::new();
    for file in &files {
        if let Ok(content) = std::fs::read_to_string(file) {
            if let Some(fm) = frontmatter::parse_frontmatter(&content) {
                if let Some(ref filter_expr) = args.filter {
                    if !frontmatter::filter_matches(&fm, filter_expr) {
                        continue;
                    }
                }

                let mut fields_map = std::collections::HashMap::new();
                for field in &fm.fields {
                    if let Some(ref specific_field) = args.field {
                        if &field.key != specific_field { continue; }
                    }
                    let val: serde_json::Value = serde_json::from_str(&field.value_json).unwrap_or_default();
                    fields_map.insert(field.key.clone(), val);
                }

                if !fields_map.is_empty() || args.field.is_none() {
                    all_entries.push(output::FrontmatterEntry {
                        file: file.clone(),
                        fields: fields_map,
                    });
                }
            }
        }
    }

    let total = all_entries.len();

    if count_only {
        if json {
            println!("{}", output::to_json(&serde_json::json!({"meta":{"total": total}}), pretty));
        } else {
            println!("{}", total);
        }
        return Ok(if total > 0 { 0 } else { 1 });
    }

    let paged: Vec<_> = all_entries.into_iter().skip(offset).take(limit).collect();
    let returned = paged.len();

    if json {
        let meta = output::Meta::paging(total, returned, offset, limit);
        let env = output::Envelope::with_results(meta, paged);
        println!("{}", output::to_json(&env, pretty));
    } else {
        for entry in &paged {
            println!("{}", entry.format_raw());
        }
        if returned < total {
            println!("\n{}", output::format_raw_footer(returned, total, offset));
        }
    }

    Ok(if total > 0 { 0 } else { 1 })
}

fn run_links(
    args: &cli::LinksArgs, json: bool, pretty: bool,
    limit: usize, offset: usize, count_only: bool, exists: bool,
) -> Result<i32> {
    let content = std::fs::read_to_string(&args.file)
        .with_context(|| format!("Failed to read {}", args.file))?;
    let mut parsed = links::parse_links(&content);

    if let Some(ref link_type) = args.r#type {
        match link_type {
            cli::LinkType::Wiki => parsed.retain(|l| matches!(l.link_type, links::LinkKind::Wiki)),
            cli::LinkType::Markdown => parsed.retain(|l| matches!(l.link_type, links::LinkKind::Markdown)),
            cli::LinkType::All => {}
        }
    }

    let total = parsed.len();

    if exists {
        return Ok(if total > 0 { 0 } else { 1 });
    }

    if count_only {
        if json {
            println!("{}", output::to_json(&serde_json::json!({"meta":{"total": total}}), pretty));
        } else {
            println!("{}", total);
        }
        return Ok(if total > 0 { 0 } else { 1 });
    }

    let paged: Vec<_> = parsed.into_iter().skip(offset).take(limit).collect();
    let returned = paged.len();

    if json {
        let meta = output::Meta::paging(total, returned, offset, limit);
        let env = output::Envelope::with_results(meta, paged);
        println!("{}", output::to_json(&env, pretty));
    } else {
        for link in &paged {
            let link_type_str = match link.link_type {
                links::LinkKind::Wiki => "wiki",
                links::LinkKind::Markdown => "markdown",
            };
            let target = link.target_path.as_deref().unwrap_or(&link.target_raw);
            println!("{} > {} {} L{}", args.file, target, link_type_str, link.source_line);
        }
        if returned < total {
            println!("\n{}", output::format_raw_footer(returned, total, offset));
        }
    }

    Ok(0)
}

fn run_backlinks(
    args: &cli::BacklinksArgs, json: bool, pretty: bool,
    limit: usize, offset: usize, count_only: bool,
) -> Result<i32> {
    let target_file = &args.file;
    let target_name = Path::new(target_file).file_stem()
        .and_then(|s| s.to_str()).unwrap_or(target_file);

    let cwd = std::env::current_dir()?;
    let files = collect_md_files(cwd.to_str().unwrap_or("."))?;
    let mut backlinks = Vec::new();

    for file in &files {
        if file == target_file { continue; }
        if let Ok(content) = std::fs::read_to_string(file) {
            let parsed = links::parse_links(&content);
            for link in parsed {
                let target = link.target_path.as_deref().or(Some(&link.target_raw)).unwrap_or("");
                if target.contains(target_name) {
                    backlinks.push(output::SearchResult {
                        file: file.clone(),
                        section_index: None,
                        section_title: None,
                        line: link.source_line,
                        snippet: link.target_raw.clone(),
                        score: 1.0,
                    });
                }
            }
        }
    }

    let total = backlinks.len();

    if count_only {
        if json {
            println!("{}", output::to_json(&serde_json::json!({"meta":{"total": total}}), pretty));
        } else {
            println!("{}", total);
        }
        return Ok(if total > 0 { 0 } else { 1 });
    }

    let paged: Vec<_> = backlinks.into_iter().skip(offset).take(limit).collect();
    let returned = paged.len();

    if json {
        let meta = output::Meta::paging(total, returned, offset, limit);
        let env = output::Envelope::with_results(meta, paged);
        println!("{}", output::to_json(&env, pretty));
    } else {
        for bl in &paged {
            println!("{} L{}: {}", bl.file, bl.line, bl.snippet);
        }
        if returned < total {
            println!("\n{}", output::format_raw_footer(returned, total, offset));
        }
    }

    Ok(if total > 0 { 0 } else { 1 })
}

fn run_graph(
    args: &cli::GraphArgs, json: bool, pretty: bool,
    limit: usize, offset: usize,
) -> Result<i32> {
    let files = collect_md_files(&args.input)?;
    let mut files_with_links: Vec<(String, Vec<links::Link>)> = Vec::new();

    for file in &files {
        if let Ok(content) = std::fs::read_to_string(file) {
            let parsed = links::parse_links(&content);
            files_with_links.push((file.clone(), parsed));
        }
    }

    match args.format {
        cli::GraphFormat::Stats => {
            let nodes = links::build_graph(&files_with_links);
            let stats = links::compute_graph_stats(&nodes);
            if json {
                println!("{}", output::to_json(&stats, pretty));
            } else {
                println!("nodes: {}, edges: {}, orphans: {}", stats.nodes, stats.edges, stats.orphans);
                if let Some((ref path, count)) = stats.most_linked {
                    println!("most linked: {} ({} in)", path, count);
                }
                if let Some((ref path, count)) = stats.most_linking {
                    println!("most linking: {} ({} out)", path, count);
                }
            }
        }
        cli::GraphFormat::Edges => {
            let edges = links::collect_edges(&files_with_links);
            let total = edges.len();
            let paged: Vec<_> = edges.into_iter().skip(offset).take(limit).collect();
            let returned = paged.len();

            if json {
                let meta = output::GraphMeta { nodes: files.len(), edges: total };
                let edge_infos: Vec<output::EdgeInfo> = paged.iter().map(|e| output::EdgeInfo {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    r#type: Some(match e.link_type { links::LinkKind::Wiki => "wiki".to_string(), links::LinkKind::Markdown => "markdown".to_string() }),
                    line: Some(e.line),
                }).collect();
                let env = output::GraphEdges { meta, edges: edge_infos };
                println!("{}", output::to_json(&env, pretty));
            } else {
                for e in &paged {
                    let lt = match e.link_type { links::LinkKind::Wiki => "wiki", links::LinkKind::Markdown => "markdown" };
                    println!("{} > {} {} L{}", e.from, e.to, lt, e.line);
                }
                if returned < total {
                    println!("\n{}", output::format_raw_footer(returned, total, offset));
                }
            }
        }
        cli::GraphFormat::Adjacency => {
            let nodes = links::build_graph(&files_with_links);
            let total_edges: usize = nodes.iter().map(|n| n.outgoing.len()).sum();

            if json {
                let mut graph = std::collections::BTreeMap::new();
                for node in &nodes {
                    graph.insert(node.path.clone(), output::NodeInfo {
                        out: node.outgoing.clone(),
                        r#in: node.incoming.clone(),
                    });
                }
                let env = output::GraphAdjacency {
                    meta: output::GraphMeta { nodes: nodes.len(), edges: total_edges },
                    graph,
                };
                println!("{}", output::to_json(&env, pretty));
            } else {
                let mut edge_count = 0;
                for node in &nodes {
                    for target in &node.outgoing {
                        if edge_count >= offset && edge_count < offset + limit {
                            println!("{} > {}", node.path, target);
                        }
                        edge_count += 1;
                    }
                }
                if edge_count > offset + limit {
                    let shown = (offset + limit).min(edge_count) - offset;
                    println!("\n{}", output::format_raw_footer(shown, edge_count, offset));
                }
            }
        }
    }

    Ok(0)
}

fn run_section_set(args: &cli::SectionSetArgs) -> Result<i32> {
    let content = manipulate::read_content_input(
        args.content.as_deref(),
        args.content_file.as_deref(),
    )?;

    manipulate::section_set(
        &args.file, &args.section, &content,
        args.output.as_deref(), args.dry_run,
    )?;

    Ok(0)
}

fn run_section_add(args: &cli::SectionAddArgs) -> Result<i32> {
    let content = manipulate::read_content_input(
        args.content.as_deref(),
        args.content_file.as_deref(),
    ).unwrap_or_default();

    manipulate::section_add(
        &args.file, &args.title, &content,
        args.after.as_deref(), args.before.as_deref(),
        args.level, args.output.as_deref(), args.dry_run,
    )?;

    Ok(0)
}

fn run_section_delete(args: &cli::SectionDeleteArgs) -> Result<i32> {
    manipulate::section_delete(
        &args.file, &args.section,
        args.output.as_deref(), args.dry_run,
    )?;

    Ok(0)
}

fn run_frontmatter_set(args: &cli::FrontmatterSetArgs) -> Result<i32> {
    manipulate::frontmatter_set(
        &args.file, &args.key, &args.value,
        args.output.as_deref(), args.dry_run,
    )?;

    Ok(0)
}

fn run_index(args: &cli::IndexArgs, json: bool, pretty: bool) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let root = index::Database::get_or_create_root(None, &cwd)?;
    let db = index::Database::open(&root)?;

    if args.status {
        let status = db.status()?;
        if json {
            println!("{}", output::to_json(&serde_json::json!({
                "path": status.path,
                "last_sync": status.last_sync,
                "files": { "indexed": status.files_indexed, "stale": status.files_stale, "deleted": status.files_deleted },
                "size": { "sqlite_bytes": status.sqlite_bytes, "tantivy_bytes": status.tantivy_bytes },
            }), pretty));
        } else {
            println!("db: {} (last sync: {})", status.path, status.last_sync);
            println!("files: {} indexed, {} stale, {} deleted", status.files_indexed, status.files_stale, status.files_deleted);
            println!("size: sqlite {:.1}MB, tantivy {:.1}MB",
                status.sqlite_bytes as f64 / 1_048_576.0,
                status.tantivy_bytes as f64 / 1_048_576.0);
        }
        return Ok(0);
    }

    if args.force {
        let tantivy_dir = root.join(".worktoolai/markdownai_index");
        if tantivy_dir.exists() {
            engine::SearchEngine::destroy(&tantivy_dir)?;
        }
        let db_path = root.join(".worktoolai/markdownai.db");
        if db_path.exists() {
            std::fs::remove_file(&db_path)?;
        }
        eprintln!("Cleared database. Rebuilding...");
        let db = index::Database::open(&root)?;
        return index_path(&args.path, &root, &db, args.dry_run);
    }

    index_path(&args.path, &root, &db, args.dry_run)
}

fn index_path(input: &str, root: &Path, db: &index::Database, dry_run: bool) -> Result<i32> {
    let files = collect_md_files(input)?;
    let mut synced = 0;
    let mut skipped = 0;

    for file in &files {
        let content = std::fs::read_to_string(file)?;
        let rel_path = Path::new(file).strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| file.clone());

        let hash = index::Database::compute_hash(content.as_bytes());

        if !db.check_file_stale(&rel_path, &hash)? {
            skipped += 1;
            continue;
        }

        if dry_run {
            eprintln!("would sync: {}", rel_path);
            synced += 1;
            continue;
        }

        let doc = markdown::parse_document(&content);
        let parsed_links = links::parse_links(&content);
        let fm = frontmatter::parse_frontmatter(&content);

        db.sync_file(&rel_path, &content, &doc.sections, &parsed_links, fm.as_ref())?;
        synced += 1;
    }

    if synced > 0 || skipped > 0 {
        eprintln!("synced {} files, {} unchanged", synced, skipped);
    }

    Ok(0)
}

fn read_input(file: &str) -> Result<String> {
    if file == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).context("Failed to read stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(file).with_context(|| format!("Failed to read {}", file))
    }
}
