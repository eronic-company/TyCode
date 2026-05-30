use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::ToolResult;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // skip files larger than 10 MB

/// Find files matching a glob pattern.
pub fn glob_search(pattern: &str, base_path: &str) -> ToolResult {
    if pattern.is_empty() {
        return ToolResult::err("No pattern provided");
    }

    let base = if base_path.is_empty() {
        ".".to_string()
    } else {
        super::shellexpand(base_path)
    };

    let full_pattern = if pattern.starts_with('/') || pattern.starts_with('.') {
        pattern.to_string()
    } else {
        format!("{}/{}", base, pattern)
    };

    match glob::glob(&full_pattern) {
        Ok(paths) => {
            let mut results: Vec<String> = Vec::new();
            for entry in paths {
                match entry {
                    Ok(path) => {
                        results.push(path.display().to_string());
                        if results.len() >= 500 {
                            results.push("... (truncated at 500 results)".into());
                            break;
                        }
                    }
                    Err(e) => {
                        results.push(format!("(error: {e})"));
                    }
                }
            }

            if results.is_empty() {
                ToolResult::ok(format!("No files matching '{pattern}' in {base}"))
            } else {
                ToolResult::ok(format!(
                    "{} file(s) found:\n{}",
                    results.len(),
                    results.join("\n")
                ))
            }
        }
        Err(e) => ToolResult::err(format!("Invalid glob pattern: {e}")),
    }
}

/// Search file contents using regex, with optional glob filter and context.
pub fn grep(
    pattern: &str,
    path: &str,
    glob_filter: &str,
    case_insensitive: bool,
    context: usize,
    max_results: usize,
) -> ToolResult {
    if pattern.is_empty() {
        return ToolResult::err("No search pattern provided");
    }

    let base = if path.is_empty() {
        ".".to_string()
    } else {
        super::shellexpand(path)
    };

    let regex_pattern = if case_insensitive {
        format!("(?i){}", pattern)
    } else {
        pattern.to_string()
    };

    let re = match regex::Regex::new(&regex_pattern) {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Invalid regex pattern: {e}")),
    };

    let base_path = Path::new(&base);
    let max = if max_results == 0 { 50 } else { max_results };

    let mut matches: Vec<String> = Vec::new();
    let mut files_searched = 0u32;

    if base_path.is_file() {
        search_file(base_path, &re, context, max, &mut matches);
        files_searched = 1;
    } else if base_path.is_dir() {
        let glob_re = if !glob_filter.is_empty() {
            glob_to_regex(glob_filter)
        } else {
            None
        };
        walk_and_search(base_path, &re, &glob_re, context, max, &mut matches, &mut files_searched);
    } else {
        return ToolResult::err(format!("Path not found: {path}"));
    }

    if matches.is_empty() {
        ToolResult::ok(format!(
            "No matches for '{}' in {} ({} files searched)",
            pattern, base, files_searched
        ))
    } else {
        let count = matches.len();
        ToolResult::ok(format!(
            "{count} match(es) ({files_searched} files searched):\n\n{}",
            matches.join("\n")
        ))
    }
}

fn search_file(
    path: &Path,
    re: &regex::Regex,
    context: usize,
    max: usize,
    matches: &mut Vec<String>,
) {
    if matches.len() >= max {
        return;
    }

    // Skip files that are too large to avoid OOM.
    if let Ok(meta) = fs::metadata(path) {
        if meta.len() > MAX_FILE_SIZE {
            return;
        }
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };

    // Collect lines via BufReader to avoid loading the entire file as a String.
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(|l| l.ok())
        .collect();

    let path_str = path.display().to_string();

    for (i, line) in lines.iter().enumerate() {
        if matches.len() >= max {
            matches.push("... (max results reached)".into());
            return;
        }

        if re.is_match(line) {
            if context > 0 {
                let start = i.saturating_sub(context);
                let end = (i + context + 1).min(lines.len());
                matches.push(format!("{}:", path_str));
                for j in start..end {
                    let marker = if j == i { ">" } else { " " };
                    matches.push(format!("{} {:>5}| {}", marker, j + 1, lines[j]));
                }
                matches.push(String::new());
            } else {
                matches.push(format!("{}:{}: {}", path_str, i + 1, line));
            }
        }
    }
}

fn walk_and_search(
    dir: &Path,
    re: &regex::Regex,
    glob_re: &Option<regex::Regex>,
    context: usize,
    max: usize,
    matches: &mut Vec<String>,
    files_searched: &mut u32,
) {
    if matches.len() >= max {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if matches.len() >= max {
            return;
        }

        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        // Skip hidden and common non-text directories
        if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
            continue;
        }

        if path.is_dir() {
            walk_and_search(&path, re, glob_re, context, max, matches, files_searched);
        } else if path.is_file() {
            // Apply glob filter
            if let Some(filter) = glob_re {
                if !filter.is_match(&name) {
                    continue;
                }
            }

            // Skip likely binary files
            let ext = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if is_binary_ext(&ext) {
                continue;
            }

            *files_searched += 1;
            search_file(&path, re, context, max, matches);
        }
    }
}

fn glob_to_regex(glob: &str) -> Option<regex::Regex> {
    let pattern = glob
        .replace('.', "\\.")
        .replace('*', ".*")
        .replace('?', ".");
    regex::Regex::new(&format!("^{}$", pattern)).ok()
}

fn is_binary_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "webp" | "svg"
            | "mp3" | "mp4" | "avi" | "mkv" | "wav" | "flac"
            | "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar"
            | "pdf" | "doc" | "docx" | "xls" | "xlsx"
            | "exe" | "dll" | "so" | "dylib" | "o" | "a"
            | "wasm" | "pyc" | "class"
            | "ttf" | "otf" | "woff" | "woff2" | "eot"
    )
}

