use sysinfo::System;
use std::fs;
use std::path::Path;

use super::ToolResult;

/// Get system information.
pub fn system_info() -> ToolResult {
    let mut sys = System::new_all();
    sys.refresh_all();

    let os_name = System::name().unwrap_or_else(|| "Unknown".into());
    let os_version = System::os_version().unwrap_or_else(|| "?".into());
    let hostname = System::host_name().unwrap_or_else(|| "?".into());
    let kernel = System::kernel_version().unwrap_or_else(|| "?".into());

    let total_mem = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let used_mem = sys.used_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    let cpu_count = sys.cpus().len();

    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());

    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into());

    let output = format!(
        "System Information\n\
         ──────────────────\n\
         OS:        {os_name} {os_version}\n\
         Kernel:    {kernel}\n\
         Hostname:  {hostname}\n\
         User:      {user}\n\
         CPUs:      {cpu_count}\n\
         Memory:    {used_mem:.1} / {total_mem:.1} GB ({:.0}%)\n\
         CWD:       {cwd}",
        (used_mem / total_mem) * 100.0,
    );

    ToolResult::ok(output)
}

/// Show a directory tree.
pub fn directory_tree(path: &str, max_depth: usize) -> ToolResult {
    let expanded = shellexpand(path);
    let root = Path::new(&expanded);

    if !root.exists() {
        return ToolResult::err(format!("Path not found: {path}"));
    }

    if !root.is_dir() {
        return ToolResult::err(format!("{path} is not a directory"));
    }

    let mut output = vec![format!("{}/", root.display())];
    let mut count = 0;
    build_tree(root, "", max_depth, 0, &mut output, &mut count);

    if count > 500 {
        output.push(format!("... ({count} total entries, showing first 500)"));
    }

    ToolResult::ok(output.join("\n"))
}

fn build_tree(
    dir: &Path,
    prefix: &str,
    max_depth: usize,
    depth: usize,
    output: &mut Vec<String>,
    count: &mut usize,
) {
    if depth >= max_depth || *count > 500 {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        b_dir.cmp(&a_dir).then(a.file_name().cmp(&b.file_name()))
    });

    let total = items.len();
    for (i, entry) in items.iter().enumerate() {
        if *count > 500 {
            return;
        }
        *count += 1;

        let name = entry.file_name().to_string_lossy().to_string();
        let is_last = i == total - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and common noise
            if name.starts_with('.') || name == "node_modules" || name == "target" || name == "__pycache__" {
                output.push(format!("{prefix}{connector}{name}/ (skipped)"));
                continue;
            }
            output.push(format!("{prefix}{connector}{name}/"));
            build_tree(
                &path,
                &format!("{prefix}{child_prefix}"),
                max_depth,
                depth + 1,
                output,
                count,
            );
        } else {
            output.push(format!("{prefix}{connector}{name}"));
        }
    }
}

fn shellexpand(path: &str) -> String {
    if path.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}
