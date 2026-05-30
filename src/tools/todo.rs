use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ToolResult;

// ── Todo State ───────────────────────────────────────────────────────────────
//
// A structured task list the agent maintains across a session — the same
// pattern Claude Code uses to plan and track multi-step work. State is held in
// a process-global list so the stateless tool dispatcher can read and update it.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

static TODOS: Mutex<Vec<TodoItem>> = Mutex::new(Vec::new());

/// Replace the entire todo list with a new set of items.
pub fn todo_write(items: Option<&Value>) -> ToolResult {
    let arr = match items.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return ToolResult::err("todos must be an array of {content, status} objects"),
    };

    let mut parsed: Vec<TodoItem> = Vec::with_capacity(arr.len());
    let mut in_progress_count = 0;
    for (i, item) in arr.iter().enumerate() {
        let content = match item.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.to_string(),
            _ => return ToolResult::err(format!("todo #{i} is missing a non-empty `content`")),
        };
        let status = match item.get("status").and_then(|v| v.as_str()).unwrap_or("pending") {
            "pending" => TodoStatus::Pending,
            "in_progress" => {
                in_progress_count += 1;
                TodoStatus::InProgress
            }
            "completed" => TodoStatus::Completed,
            other => {
                return ToolResult::err(format!(
                    "todo #{i} has invalid status `{other}` (use pending|in_progress|completed)"
                ))
            }
        };
        parsed.push(TodoItem { content, status });
    }

    if in_progress_count > 1 {
        return ToolResult::err(
            "Only one todo may be in_progress at a time — finish the current task first.",
        );
    }

    let rendered = render(&parsed);
    if let Ok(mut guard) = TODOS.lock() {
        *guard = parsed;
    }
    ToolResult::ok(format!("Todo list updated:\n{rendered}"))
}

/// Read the current todo list.
pub fn todo_read() -> ToolResult {
    let guard = match TODOS.lock() {
        Ok(g) => g,
        Err(_) => return ToolResult::err("todo state poisoned"),
    };
    if guard.is_empty() {
        return ToolResult::ok("(todo list is empty)");
    }
    ToolResult::ok(render(&guard))
}

fn render(items: &[TodoItem]) -> String {
    items
        .iter()
        .map(|t| {
            let mark = match t.status {
                TodoStatus::Pending => "[ ]",
                TodoStatus::InProgress => "[~]",
                TodoStatus::Completed => "[x]",
            };
            format!("{mark} {}", t.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Snapshot of the current list, for the TUI to render a live panel.
pub fn snapshot() -> Vec<TodoItem> {
    TODOS.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Clear the list (called on /clear).
pub fn clear() {
    if let Ok(mut g) = TODOS.lock() {
        g.clear();
    }
}
