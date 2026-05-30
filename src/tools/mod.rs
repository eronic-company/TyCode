pub mod file_ops;
pub mod http_ops;
pub mod process;
pub mod search;
pub mod shell;
pub mod system;
pub mod todo;

pub(crate) fn shellexpand(path: &str) -> String {
    if path.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── Tool Schema ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema object
}

// ── Tool Result ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self { success: true, output: output.into() }
    }
    pub fn err(error: impl Into<String>) -> Self {
        Self { success: false, output: error.into() }
    }
}

// ── Dispatcher ───────────────────────────────────────────────────────────────

pub fn execute_tool(name: &str, input: &Value) -> ToolResult {
    let get_str = |key: &str| -> String {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_i64 = |key: &str, default: i64| -> i64 {
        input
            .get(key)
            .and_then(|v| v.as_i64())
            .unwrap_or(default)
    };
    let get_u64 = |key: &str, default: u64| -> u64 {
        input
            .get(key)
            .and_then(|v| v.as_u64())
            .unwrap_or(default)
    };
    let get_bool = |key: &str, default: bool| -> bool {
        input
            .get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    };

    match name {
        // File operations
        "file_read" => file_ops::file_read(
            &get_str("path"),
            get_u64("offset", 0) as usize,
            get_u64("limit", 2000) as usize,
        ),
        "file_write" => file_ops::file_write(&get_str("path"), &get_str("content")),
        "file_edit" => file_ops::file_edit(
            &get_str("file_path"),
            &get_str("old_string"),
            &get_str("new_string"),
            get_bool("replace_all", false),
        ),
        "multi_edit" => file_ops::multi_edit(&get_str("file_path"), input.get("edits")),
        "file_list" => file_ops::file_list(&get_str("path")),
        "file_delete" => file_ops::file_delete(&get_str("path")),

        // Search
        "glob_search" => search::glob_search(&get_str("pattern"), &get_str("path")),
        "grep" => search::grep(
            &get_str("pattern"),
            &get_str("path"),
            &get_str("glob"),
            get_bool("case_insensitive", false),
            get_u64("context", 0) as usize,
            get_u64("max_results", 50) as usize,
        ),

        // Shell
        "bash" => shell::bash_execute(
            &get_str("command"),
            get_u64("timeout", 30) as u64,
            &get_str("cwd"),
        ),

        // Process
        "process_list" => process::process_list(&get_str("filter")),
        "process_kill" => process::process_kill(get_i64("pid", 0) as u32),
        "process_start" => process::process_start(&get_str("command")),

        // HTTP
        "http_request" => http_ops::http_request(
            &get_str("method"),
            &get_str("url"),
            input.get("headers"),
            &get_str("body"),
        ),
        "http_download" => http_ops::http_download(&get_str("url"), &get_str("output_path")),
        "web_fetch" => http_ops::web_fetch(&get_str("url")),

        // Task tracking
        "todo_write" => todo::todo_write(input.get("todos")),
        "todo_read" => todo::todo_read(),

        // System
        "system_info" => system::system_info(),
        "directory_tree" => system::directory_tree(
            &get_str("path"),
            get_u64("max_depth", 3) as usize,
        ),

        _ => ToolResult::err(format!("Unknown tool: {name}")),
    }
}

// ── All Tool Schemas ─────────────────────────────────────────────────────────

pub fn all_tool_schemas() -> Vec<ToolSchema> {
    vec![
        // ── File Operations ──────────────────────────────────────────────
        ToolSchema {
            name: "file_read".into(),
            description: "Read file contents with line numbers. Supports offset and limit for large files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the file to read"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Line number to start reading from (0-based, default: 0)"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read (default: 2000)"
                    }
                },
                "required": ["path"]
            }),
        },
        ToolSchema {
            name: "file_write".into(),
            description: "Write content to a file. Creates the file and parent directories if they don't exist. Overwrites existing content.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        },
        ToolSchema {
            name: "file_edit".into(),
            description: "Edit a file by replacing exact string matches. The old_string must match exactly (including whitespace and indentation).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Absolute path to the file to edit"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "The exact text to find and replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "The replacement text"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default: false, only first)"
                    }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
        },
        ToolSchema {
            name: "multi_edit".into(),
            description: "Apply multiple exact-string edits to a single file in one atomic operation. Edits run in order; if any fails, the file is left unchanged. Prefer this over several file_edit calls on the same file.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Absolute path to the file to edit"
                    },
                    "edits": {
                        "type": "array",
                        "description": "Ordered list of edits to apply",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string", "description": "Exact text to find" },
                                "new_string": { "type": "string", "description": "Replacement text" },
                                "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
                            },
                            "required": ["old_string", "new_string"]
                        }
                    }
                },
                "required": ["file_path", "edits"]
            }),
        },
        ToolSchema {
            name: "file_list".into(),
            description: "List files and directories at a path with details (size, modified time, type).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (default: current directory)"
                    }
                },
                "required": []
            }),
        },
        ToolSchema {
            name: "file_delete".into(),
            description: "Delete a file or directory tree.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to delete"
                    }
                },
                "required": ["path"]
            }),
        },

        // ── Search ───────────────────────────────────────────────────────
        ToolSchema {
            name: "glob_search".into(),
            description: "Find files matching a glob pattern (e.g. '**/*.rs', 'src/**/*.toml'). Returns matching file paths.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match files against"
                    },
                    "path": {
                        "type": "string",
                        "description": "Base directory to search from (default: current directory)"
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolSchema {
            name: "grep".into(),
            description: "Search file contents using regex patterns. Returns matching lines with file paths and line numbers.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for"
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search in (default: current directory)"
                    },
                    "glob": {
                        "type": "string",
                        "description": "Glob filter for files (e.g. '*.rs', '*.py')"
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Case insensitive search (default: false)"
                    },
                    "context": {
                        "type": "integer",
                        "description": "Lines of context before and after each match (default: 0)"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of matching lines to return (default: 50)"
                    }
                },
                "required": ["pattern"]
            }),
        },

        // ── Shell ────────────────────────────────────────────────────────
        ToolSchema {
            name: "bash".into(),
            description: "Execute a shell command and return stdout, stderr, and exit code. Use for system operations, git commands, builds, etc.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default: 30)"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Working directory for the command (default: current directory)"
                    }
                },
                "required": ["command"]
            }),
        },

        // ── Process Management ───────────────────────────────────────────
        ToolSchema {
            name: "process_list".into(),
            description: "List running processes with PID, name, CPU%, and memory usage. Optionally filter by name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Filter processes by name substring (optional)"
                    }
                },
                "required": []
            }),
        },
        ToolSchema {
            name: "process_kill".into(),
            description: "Terminate a process by PID.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pid": {
                        "type": "integer",
                        "description": "Process ID to terminate"
                    }
                },
                "required": ["pid"]
            }),
        },
        ToolSchema {
            name: "process_start".into(),
            description: "Start a new process in the background.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Command to run in the background"
                    }
                },
                "required": ["command"]
            }),
        },

        // ── HTTP ─────────────────────────────────────────────────────────
        ToolSchema {
            name: "http_request".into(),
            description: "Make an HTTP request (GET, POST, PUT, DELETE, etc.) and return the response.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "description": "HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD)"
                    },
                    "url": {
                        "type": "string",
                        "description": "Request URL"
                    },
                    "headers": {
                        "type": "object",
                        "description": "HTTP headers as key-value pairs (optional)"
                    },
                    "body": {
                        "type": "string",
                        "description": "Request body (optional)"
                    }
                },
                "required": ["method", "url"]
            }),
        },
        ToolSchema {
            name: "http_download".into(),
            description: "Download a file from a URL and save it to disk.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to download from"
                    },
                    "output_path": {
                        "type": "string",
                        "description": "Local file path to save the download"
                    }
                },
                "required": ["url", "output_path"]
            }),
        },

        ToolSchema {
            name: "web_fetch".into(),
            description: "Fetch a web page and return its readable text content with HTML stripped. Use to read documentation, articles, or API references from a URL.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch (http/https; bare domains are upgraded to https)"
                    }
                },
                "required": ["url"]
            }),
        },

        // ── Task Tracking ────────────────────────────────────────────────
        ToolSchema {
            name: "todo_write".into(),
            description: "Create or update a structured task list for the current work. Use it to plan multi-step tasks and track progress: break the work into items, mark exactly one as in_progress while you work it, and flip items to completed as you finish. Keeps you and the user aligned on what's done and what's left.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "description": "The full task list (replaces any existing list)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string", "description": "Description of the task" },
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"],
                                    "description": "Current status; at most one item may be in_progress"
                                }
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
        },
        ToolSchema {
            name: "todo_read".into(),
            description: "Read the current task list and the status of each item.".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },

        // ── System ───────────────────────────────────────────────────────
        ToolSchema {
            name: "system_info".into(),
            description: "Get system information: OS, hostname, CPU, memory, disk usage.".into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolSchema {
            name: "directory_tree".into(),
            description: "Show a directory tree structure with files and subdirectories.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Root directory path"
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum depth to traverse (default: 3)"
                    }
                },
                "required": ["path"]
            }),
        },
    ]
}
