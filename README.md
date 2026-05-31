# TyCode

A fast, terminal-based AI coding agent that works with any LLM provider: Ollama, Anthropic, OpenAI, or Google Gemini. Built in Rust.

![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)
![License](https://img.shields.io/badge/license-GPL%20v3-blue)

---

## Features

- **Multi-provider**: Ollama (local), Anthropic Claude, OpenAI, Google Gemini
- **Full tool suite**: file read/write/edit, atomic multi-edit, shell execution, grep, glob search, HTTP, web fetch, process management
- **Task tracking**: the agent plans multi-step work with a live todo list (`todo_write`/`todo_read`), like Claude Code
- **Web fetch**: `web_fetch` pulls a URL and strips HTML to readable text so the agent can read docs and articles
- **Autonomous agent loop**: chains tool calls until the task is done, no hand-holding required
- **Interruptible**: `Esc` (or `Ctrl+C`) aborts the in-flight model stream and stops the agent mid-task, instantly
- **Concurrent tools**: independent read-only tool calls in a batch run in parallel; mutating tools stay ordered
- **Resilient streaming**: transient API errors (429 / 5xx / network) auto-retry with exponential backoff instead of failing the run
- **Project memory**: auto-loads `TYCODE.md`, `CLAUDE.md`, `AGENTS.md`, and `README.md` from the working directory as context
- **File import**: `/import <path>` injects any file into the agent's context
- **Atomic file writes**: temp-file + rename pattern; no corruption on power loss
- **Real shell timeouts**: commands are killed after their deadline, no hung threads
- **Typewriter reveal**: the response stays hidden behind the thinking spinner while it generates, then types out line-by-line (action-by-action) to mimic live generation
- **Wrapping input box**: grows as you type, no horizontal overflow
- **TUI with markdown rendering**: headers, bold, code blocks, lists in the chat view

---

## Install

**Prerequisites:** Rust toolchain (`rustup`)

```bash
git clone https://github.com/AlphaGlider25/TyCode
cd TyCode
cargo build --release
```

Then symlink the binary so `tycode` is available globally:

```bash
mkdir -p ~/.local/bin
ln -sf "$PWD/target/release/tycode" ~/.local/bin/tycode
```

Make sure `~/.local/bin` is in your `PATH` (add to `~/.bashrc` / `~/.zshrc` if needed):

```bash
export PATH="$HOME/.local/bin:$PATH"
```

---

## Usage

```bash
# Launch in the current directory; TyCode reads that folder as context
tycode

# From any project
cd ~/my-project
tycode
```

### Slash commands

| Command | Description |
|---|---|
| `/help` | Show all commands and key bindings |
| `/model [name]` | Switch model or open the model picker |
| `/provider <name>` | Switch provider (`ollama`, `anthropic`, `openai`, `gemini`) |
| `/settings` | Edit API keys, model, provider, limits |
| `/import <path>` | Inject a file into the agent's context |
| `/clear` | Clear chat history and agent context |
| `/system <prompt>` | Set a custom system prompt |
| `/exit` | Clean exit (also `Ctrl+C`) |

### Key bindings

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Up / Down` | Navigate input history |
| `PgUp / PgDn` · wheel | Scroll chat |
| Drag scrollbar | Click/drag the right-edge bar to scrub |
| `Ctrl+Home / Ctrl+End` | Jump to top / bottom |
| Click-drag (2×=word, 3×=line) | Select text |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy selection / paste |
| `Tab` | Autocomplete slash commands |
| `Esc` | Interrupt the running agent / clear input / close overlay |
| `Ctrl+C` | Interrupt the agent (press twice within 2 s to quit) |

---

## Configuration

Config is stored at `~/.tycode/config.json` and editable via `/settings` inside the TUI.

| Field | Default | Description |
|---|---|---|
| `provider` | `ollama` | Active provider |
| `model` | `gemma3` | Active model |
| `ollama_url` | `http://localhost:11434` | Ollama endpoint |
| `anthropic_api_key` | (empty) | Anthropic API key |
| `openai_api_key` | (empty) | OpenAI API key |
| `openai_base_url` | (empty) | Custom OpenAI-compatible endpoint |
| `google_api_key` | (empty) | Google Gemini API key |
| `max_iterations` | `100` | Max agent loop iterations per prompt |
| `max_tokens` | `8192` | Max tokens per model response |

API keys can also be set via environment variables: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`.

---

## Providers

### Ollama (default, runs locally)
Install [Ollama](https://ollama.com), then:
```bash
ollama pull gemma3
tycode
```
TyCode will start Ollama automatically if it isn't running.

### Anthropic
```bash
tycode
# inside TyCode:
/provider anthropic
/settings   # enter your API key
```

### OpenAI
```bash
/provider openai
/settings   # enter your API key
```

### Gemini
```bash
/provider gemini
/settings   # enter your Google API key
```

---

## License

GNU General Public License v3.0. See [LICENSE](LICENSE).
