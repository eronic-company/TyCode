use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::ToolResult;

/// Check if a shell command matches a known dangerous pattern.
/// Returns a human-readable reason string if dangerous, None otherwise.
pub fn is_dangerous(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_lowercase();
    let checks: &[(&str, &'static str)] = &[
        ("rm -rf", "recursive force delete"),
        ("rm -fr", "recursive force delete"),
        ("rm --recursive --force", "recursive force delete"),
        ("rm --force --recursive", "recursive force delete"),
        ("dd if=", "disk overwrite (dd)"),
        ("mkfs", "filesystem format"),
        ("drop table", "SQL: DROP TABLE"),
        ("drop database", "SQL: DROP DATABASE"),
        (":(){:|:&};:", "fork bomb"),
        ("git push --force", "force push"),
        ("git reset --hard", "hard reset (data loss)"),
        ("chmod -r 777", "world-writable recursive chmod"),
        ("chmod -r 0777", "world-writable recursive chmod"),
        ("sudo rm", "sudo delete"),
        ("sudo dd", "sudo disk overwrite"),
        ("truncate -s 0", "file truncation to zero"),
    ];
    for &(pat, reason) in checks {
        if lower.contains(pat) {
            return Some(reason);
        }
    }
    // git push -f (short flag, but not -ff which is fast-forward)
    if let Some(pos) = lower.find("git push -f") {
        let after = &lower[pos + "git push -f".len()..];
        if after.is_empty() || after.starts_with([' ', '\n', '\t', ';', '&', '|']) {
            return Some("force push");
        }
    }
    None
}

/// Execute a shell command with an enforced timeout and optional working directory.
pub fn bash_execute(command: &str, timeout_secs: u64, cwd: &str) -> ToolResult {
    if command.is_empty() {
        return ToolResult::err("No command provided");
    }

    let timeout = Duration::from_secs(if timeout_secs == 0 { 30 } else { timeout_secs });

    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command).stdout(Stdio::piped()).stderr(Stdio::piped());

    if !cwd.is_empty() {
        let expanded = super::shellexpand(cwd);
        let p = std::path::Path::new(&expanded);
        if p.is_dir() {
            cmd.current_dir(p);
        }
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to spawn command: {e}")),
    };

    // Drain stdout and stderr in background threads to prevent pipe-buffer deadlock.
    let stdout_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

    let stdout_pipe = child.stdout.take().expect("stdout was piped");
    let stderr_pipe = child.stderr.take().expect("stderr was piped");

    let stdout_clone = Arc::clone(&stdout_buf);
    let stdout_thread = thread::spawn(move || {
        let mut pipe = stdout_pipe;
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        *stdout_clone.lock().unwrap() = buf;
    });

    let stderr_clone = Arc::clone(&stderr_buf);
    let stderr_thread = thread::spawn(move || {
        let mut pipe = stderr_pipe;
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        *stderr_clone.lock().unwrap() = buf;
    });

    let deadline = Instant::now() + timeout;
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                break status;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return ToolResult::err(format!(
                        "Command timed out after {}s",
                        timeout.as_secs()
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return ToolResult::err(format!("Failed to wait for command: {e}")),
        }
    };

    let stdout_raw = stdout_buf.lock().unwrap().clone();
    let stderr_raw = stderr_buf.lock().unwrap().clone();

    let stdout = String::from_utf8_lossy(&stdout_raw);
    let stderr = String::from_utf8_lossy(&stderr_raw);
    let code = exit_status.code().unwrap_or(-1);

    let stdout_clean = strip_ansi(&stdout);
    let stderr_clean = strip_ansi(&stderr);

    let mut result = String::new();
    if !stdout_clean.is_empty() {
        result.push_str(&stdout_clean);
    }
    if !stderr_clean.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("stderr: ");
        result.push_str(&stderr_clean);
    }
    if result.is_empty() {
        result = format!("(no output, exit code {code})");
    } else if code != 0 {
        result.push_str(&format!("\n(exit code {code})"));
    }

    if result.len() > 65536 {
        let mut cut = 65536;
        while !result.is_char_boundary(cut) { cut -= 1; }
        result.truncate(cut);
        result.push_str("\n... (output truncated at 64KB)");
    }

    if code == 0 {
        ToolResult::ok(result)
    } else {
        ToolResult { success: false, output: result }
    }
}

fn strip_ansi(s: &str) -> String {
    let bytes = strip_ansi_escapes::strip(s);
    String::from_utf8_lossy(&bytes).to_string()
}
