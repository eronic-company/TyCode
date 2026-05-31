# Changelog

All notable changes to TyCode are documented here.

## [0.5.1] — 2026-05-30

### Distribution
- **CI release binaries**: a GitHub Actions workflow now builds and attaches
  binaries for Linux x86_64, macOS x86_64 (Intel), macOS aarch64 (Apple
  Silicon), and Windows x86_64 to each published release, so the npm package can
  download a ready-to-run binary.
- Fixed the npm wrapper (`bin/tycode.js`) looking for the binary under
  `darwin/x64` while the installer wrote it to `macos/x86_64` — the two now use
  the same platform/arch directory scheme.

## [0.5.0] — 2026-05-30

GitHub release: **v0.4.0**. (Versions 0.0.0–0.3.0 were consumed by earlier
publish/unpublish cycles on npm and cannot be reused, so this release lands on
0.4.0.)

### Model interaction & request handling
- **Interruptible agent**: `Esc` (or `Ctrl+C`) now aborts the in-flight model
  stream mid-token and stops the agent loop immediately. Partially streamed text
  is finalized and remaining tool calls get synthetic results so the transcript
  stays valid for the next turn.
- **Automatic retry with backoff**: transient provider failures (HTTP 429/5xx,
  rate limits, network/connection/timeout errors) retry up to 4 times with
  exponential backoff and jitter instead of killing the run. Each retry is shown
  inline and is itself cancellable.
- **Concurrent tools**: independent read-only tool calls (file_read, grep,
  glob_search, directory_tree, web_fetch, …) in a single batch run in parallel —
  a multi-read fan-out now completes in roughly the time of the slowest call.
  Mutating tools and dangerous shell commands stay sequential and gated on
  confirmation, preserving correctness and ordering.
- **Streaming efficiency**: SSE/NDJSON parsing in the Anthropic, OpenAI, and
  Ollama providers no longer reallocates the whole buffer per event
  (O(n²) → O(n) via `drain`); the OpenAI path also no longer risks
  double-parsing a record split across chunk boundaries.

### Tools (0.2.0 line, also in 0.1.0 → 0.2.0)
- `todo_write` / `todo_read`: structured, session-persistent task list.
- `multi_edit`: multiple exact-string edits to one file, applied atomically.
- `web_fetch`: fetch a URL and strip HTML to readable text.
- Project memory now auto-loads `CLAUDE.md` and `AGENTS.md` alongside
  `TYCODE.md` / `README.md`.

### Fixes
- `Ctrl+Enter` tool-call expand/collapse was unreachable (shadowed by plain
  `Enter`) — now works.
- Cleared all outstanding compiler warnings; the build is warning-free.

## [0.1.0] — 2026-05-10

Initial public release.

- Multi-provider agent: Ollama (local), Anthropic, OpenAI, Google Gemini.
- Tool suite: file read/write/edit, shell with real timeouts, grep, glob,
  HTTP, process management, system info, directory tree.
- Autonomous agent loop with streaming responses.
- TUI with markdown rendering, inline expand/collapse of tool output,
  mouse scroll, dangerous-command confirmation, and auto context compaction.
