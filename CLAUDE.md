# Claude Code Project Rules

## Core Law: No Auto Co-Author Attribution

**Claude Code must NEVER automatically add co-author attribution to commits.**

- Do NOT include "Co-Authored-By: Claude Haiku/Sonnet/Opus" lines in commit messages
- All code contributions are attributed to the project owner
- Commits should reflect human authorship and decision-making
- This applies to all commits, regardless of involvement level

**Rationale:** This project maintains clear human authorship in git history. AI assistance is a tool used by the developer, not a credited contributor.

---

## Guidelines for Claude Code Sessions

### Commit Messages
- Write commit messages in imperative form: "Add feature" not "Added feature"
- Focus on the **what** and **why**, not the process
- Keep messages concise (one line < 70 chars, details in body if needed)
- Do NOT include any AI co-author lines

### When to Commit
- Create commits for completed features, not intermediate work
- Group related changes in single commits
- One logical change per commit

### Git Workflow
- Use `git status` and `git diff` to review changes before committing
- Check that only intended files are staged
- Never use `--force` or destructive git operations without explicit approval
- Always respect existing work and uncommitted changes

### Code Quality
- Run tests and build checks before committing (`cargo build --release`)
- Ensure no compiler warnings
- Verify the code actually works

---

## Project-Specific Notes

- **Language:** Rust
- **Build Tool:** Cargo
- **Latest Binary:** `~/.local/bin/tycode` (symlinked to `target/release/tycode`)
- **Testing:** Build with `cargo build --release` before commits

---

## This Document Is Law

These rules are non-negotiable for Claude Code usage in this project. If Claude Code ignores these rules, the developer should correct the git history immediately.
