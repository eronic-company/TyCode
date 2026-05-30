use sysinfo::System;

use super::ToolResult;

/// List running processes, optionally filtered by name.
pub fn process_list(filter: &str) -> ToolResult {
    let mut sys = System::new_all();
    sys.refresh_all();

    let mut entries: Vec<String> = Vec::new();
    entries.push(format!(
        "{:<8} {:<6} {:<6} {}",
        "PID", "CPU%", "MEM MB", "NAME"
    ));
    entries.push("-".repeat(50));

    let mut procs: Vec<(&sysinfo::Pid, &sysinfo::Process)> = sys.processes().iter().collect();
    procs.sort_by(|a, b| b.1.cpu_usage().partial_cmp(&a.1.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal));

    for (pid, process) in &procs {
        let name = process.name().to_str().unwrap_or("?").to_string();

        if !filter.is_empty() && !name.to_lowercase().contains(&filter.to_lowercase()) {
            continue;
        }

        let cpu = process.cpu_usage();
        let mem_mb = process.memory() as f64 / (1024.0 * 1024.0);

        entries.push(format!(
            "{:<8} {:<6.1} {:<6.1} {}",
            pid.as_u32(),
            cpu,
            mem_mb,
            name
        ));

        if entries.len() > 102 {
            entries.push("... (truncated, use filter to narrow results)".into());
            break;
        }
    }

    ToolResult::ok(entries.join("\n"))
}

/// Kill a process by PID.
pub fn process_kill(pid: u32) -> ToolResult {
    let sys = System::new_all();
    let pid = sysinfo::Pid::from_u32(pid);

    match sys.process(pid) {
        Some(process) => {
            if process.kill() {
                ToolResult::ok(format!("Terminated process {}", pid))
            } else {
                ToolResult::err(format!("Failed to terminate process {}", pid))
            }
        }
        None => ToolResult::err(format!("Process {} not found", pid)),
    }
}

/// Start a background process.
pub fn process_start(command: &str) -> ToolResult {
    if command.is_empty() {
        return ToolResult::err("No command provided");
    }

    match std::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => ToolResult::ok(format!("Started process PID {}", child.id())),
        Err(e) => ToolResult::err(format!("Failed to start process: {e}")),
    }
}
