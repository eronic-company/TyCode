use std::fs;
use std::path::Path;

use super::ToolResult;

fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp_name = format!(".{}.tmp", uuid::Uuid::new_v4());
    let tmp_path = parent.join(tmp_name);
    fs::write(&tmp_path, content)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Read a file with line numbers, supporting offset and limit.
pub fn file_read(path: &str, offset: usize, limit: usize) -> ToolResult {
    let p = super::shellexpand(path);
    let path_ref = Path::new(&p);

    if !path_ref.exists() {
        return ToolResult::err(format!("File not found: {path}"));
    }
    if path_ref.is_dir() {
        return ToolResult::err(format!("{path} is a directory, not a file. Use file_list instead."));
    }

    match fs::read_to_string(path_ref) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();
            let start = offset.min(total);
            let end = (start + limit).min(total);
            let selected = &lines[start..end];

            let mut output = String::new();
            for (i, line) in selected.iter().enumerate() {
                let line_num = start + i + 1;
                output.push_str(&format!("{line_num:>5}\t{line}\n"));
            }

            if end < total {
                output.push_str(&format!(
                    "\n... ({} more lines, {} total)",
                    total - end,
                    total
                ));
            }

            ToolResult::ok(output)
        }
        Err(e) => {
            // Try reading as binary and report size
            match fs::metadata(path_ref) {
                Ok(meta) => ToolResult::err(format!(
                    "Cannot read as text ({}). Binary file, {} bytes.",
                    e,
                    meta.len()
                )),
                Err(_) => ToolResult::err(format!("Failed to read {path}: {e}")),
            }
        }
    }
}

/// Write content to a file, creating parent directories as needed.
pub fn file_write(path: &str, content: &str) -> ToolResult {
    let p = super::shellexpand(path);
    let path_ref = Path::new(&p);

    if let Some(parent) = path_ref.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return ToolResult::err(format!("Failed to create directories: {e}"));
        }
    }

    match atomic_write(path_ref, content.as_bytes()) {
        Ok(()) => {
            let lines = content.lines().count();
            let bytes = content.len();
            ToolResult::ok(format!("Wrote {bytes} bytes ({lines} lines) to {path}"))
        }
        Err(e) => ToolResult::err(format!("Failed to write {path}: {e}")),
    }
}

/// Edit a file by exact string replacement (like Claude Code's Edit tool).
pub fn file_edit(path: &str, old_string: &str, new_string: &str, replace_all: bool) -> ToolResult {
    let p = super::shellexpand(path);
    let path_ref = Path::new(&p);

    if !path_ref.exists() {
        return ToolResult::err(format!("File not found: {path}"));
    }

    let content = match fs::read_to_string(path_ref) {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to read {path}: {e}")),
    };

    if old_string == new_string {
        return ToolResult::err("old_string and new_string are identical");
    }

    if !content.contains(old_string) {
        return ToolResult::err(format!(
            "old_string not found in {path}. Make sure it matches exactly including whitespace."
        ));
    }

    let count = content.matches(old_string).count();
    if !replace_all && count > 1 {
        return ToolResult::err(format!(
            "old_string has {count} matches in {path}. Provide more context to make it unique, or set replace_all=true."
        ));
    }

    let new_content = if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    };

    match atomic_write(path_ref, new_content.as_bytes()) {
        Ok(()) => {
            let replaced = if replace_all { count } else { 1 };
            ToolResult::ok(format!("Replaced {replaced} occurrence(s) in {path}"))
        }
        Err(e) => ToolResult::err(format!("Failed to write {path}: {e}")),
    }
}

/// Apply a sequence of exact-string edits to a single file atomically.
/// Edits are applied in order; each `old_string` must be present (and unique
/// unless `replace_all`) when its turn comes. If any edit fails, the file is
/// left untouched — all-or-nothing, like Claude Code's MultiEdit.
pub fn multi_edit(path: &str, edits: Option<&serde_json::Value>) -> ToolResult {
    let p = super::shellexpand(path);
    let path_ref = Path::new(&p);

    if !path_ref.exists() {
        return ToolResult::err(format!("File not found: {path}"));
    }

    let edits = match edits.and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return ToolResult::err("`edits` must be a non-empty array of {old_string, new_string, replace_all?}"),
    };

    let mut content = match fs::read_to_string(path_ref) {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to read {path}: {e}")),
    };

    let mut applied = 0usize;
    for (i, edit) in edits.iter().enumerate() {
        let old_string = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
        let new_string = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
        let replace_all = edit.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        if old_string.is_empty() {
            return ToolResult::err(format!("edit #{i} has an empty old_string"));
        }
        if old_string == new_string {
            return ToolResult::err(format!("edit #{i}: old_string and new_string are identical"));
        }
        if !content.contains(old_string) {
            return ToolResult::err(format!(
                "edit #{i}: old_string not found (after {applied} prior edits). It must match exactly including whitespace."
            ));
        }
        let count = content.matches(old_string).count();
        if !replace_all && count > 1 {
            return ToolResult::err(format!(
                "edit #{i}: old_string has {count} matches. Add context to make it unique, or set replace_all=true."
            ));
        }
        content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };
        applied += 1;
    }

    match atomic_write(path_ref, content.as_bytes()) {
        Ok(()) => ToolResult::ok(format!("Applied {applied} edit(s) to {path}")),
        Err(e) => ToolResult::err(format!("Failed to write {path}: {e}")),
    }
}

/// List files and directories at a path with details.
pub fn file_list(path: &str) -> ToolResult {
    let p = if path.is_empty() { ".".to_string() } else { super::shellexpand(path) };
    let path_ref = Path::new(&p);

    if !path_ref.exists() {
        return ToolResult::err(format!("Path not found: {path}"));
    }

    if !path_ref.is_dir() {
        // Single file — show metadata
        return match fs::metadata(path_ref) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        Some(dt.format("%Y-%m-%d %H:%M").to_string())
                    })
                    .unwrap_or_else(|| "?".to_string());
                ToolResult::ok(format!(
                    "{}\n  size: {} bytes\n  modified: {modified}\n  type: file",
                    path_ref.display(),
                    meta.len()
                ))
            }
            Err(e) => ToolResult::err(format!("Failed to stat {path}: {e}")),
        };
    }

    let mut entries: Vec<String> = Vec::new();
    match fs::read_dir(path_ref) {
        Ok(rd) => {
            let mut items: Vec<_> = rd.filter_map(|e| e.ok()).collect();
            items.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

            for entry in items {
                let name = entry.file_name().to_string_lossy().to_string();
                let meta = entry.metadata();
                let (kind, size) = match &meta {
                    Ok(m) if m.is_dir() => ("dir ", String::new()),
                    Ok(m) => ("file", format_size(m.len())),
                    Err(_) => ("?   ", String::new()),
                };
                let modified = meta
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| {
                        let dt: chrono::DateTime<chrono::Local> = t.into();
                        Some(dt.format("%Y-%m-%d %H:%M").to_string())
                    })
                    .unwrap_or_else(|| "?".to_string());

                let suffix = if kind == "dir " { "/" } else { "" };
                entries.push(format!("  {kind}  {size:>8}  {modified}  {name}{suffix}"));
            }
        }
        Err(e) => return ToolResult::err(format!("Failed to read directory: {e}")),
    }

    let header = format!("{}/ ({} entries)\n", path_ref.display(), entries.len());
    ToolResult::ok(format!("{header}{}", entries.join("\n")))
}

/// Delete a file or directory tree.
pub fn file_delete(path: &str) -> ToolResult {
    let p = super::shellexpand(path);
    let path_ref = Path::new(&p);

    if !path_ref.exists() {
        return ToolResult::err(format!("Path not found: {path}"));
    }

    let result = if path_ref.is_dir() {
        fs::remove_dir_all(path_ref)
    } else {
        fs::remove_file(path_ref)
    };

    match result {
        Ok(()) => ToolResult::ok(format!("Deleted {path}")),
        Err(e) => ToolResult::err(format!("Failed to delete {path}: {e}")),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
