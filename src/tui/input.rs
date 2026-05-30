use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use super::app::{App, AppMode, ChatMessage, ModelSelectState, ProviderSelectState, SettingsState};

/// Handle a bracketed paste event (triggered by Ctrl+Shift+V or middle-click).
/// Inserts the pasted text at the cursor, normalising line endings so Windows
/// clipboard `\r\n` doesn't inject bare `\r` into the input.
pub fn handle_paste(app: &mut App, text: &str) {
    // Normalise: \r\n → \n, lone \r → \n, strip NUL and other control chars
    // except \n and \t.
    let clean: String = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect();

    if clean.is_empty() {
        return;
    }

    // Insert character-by-character at the cursor position.
    for ch in clean.chars() {
        app.input.insert(app.cursor_pos, ch);
        app.cursor_pos += ch.len_utf8();
    }
}

/// Handle a key event. Returns true if the event was consumed.
pub async fn handle_key(
    app: &mut App,
    key: KeyEvent,
    agent_tx: &mpsc::UnboundedSender<String>,
) -> bool {
    // Smart Ctrl+C: first press cancels/clears; second press within 2 s quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if let Some(last) = app.last_ctrl_c {
            if last.elapsed().as_millis() <= 2000 {
                app.should_quit = true;
                return true;
            }
        }
        app.last_ctrl_c = Some(Instant::now());
        match &app.mode {
            AppMode::Processing => {
                app.request_cancel();
                app.set_status("Interrupting… (Ctrl+C again to quit)");
            }
            _ => {
                app.input.clear();
                app.cursor_pos = 0;
                app.set_status("Ctrl+C again to quit");
            }
        }
        return true;
    }

    match &app.mode {
        AppMode::Processing => {
            handle_processing_key(app, key, agent_tx).await;
            true
        }
        AppMode::Settings(_) => {
            handle_settings_key(app, key);
            true
        }
        AppMode::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                app.mode = AppMode::Normal;
            }
            true
        }
        AppMode::ModelSelect(_) => {
            handle_model_select_key(app, key);
            true
        }
        AppMode::ProviderSelect(_) => {
            handle_provider_select_key(app, key);
            true
        }
        AppMode::Confirm(_) => {
            handle_confirm_key(app, key);
            true
        }
        AppMode::Normal => handle_normal_key(app, key, agent_tx).await,
    }
}

/// Delete the word before the cursor (Ctrl+Backspace behavior).
fn delete_prev_word(app: &mut App) {
    if app.cursor_pos == 0 {
        return;
    }

    let input_bytes = app.input.as_bytes();
    let mut pos = app.cursor_pos;

    // Skip any whitespace before cursor
    while pos > 0 && input_bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }

    // Delete the word (non-whitespace)
    while pos > 0 && !input_bytes[pos - 1].is_ascii_whitespace() {
        pos -= 1;
    }

    // Remove characters from pos to cursor_pos
    for _ in pos..app.cursor_pos {
        app.input.remove(pos);
    }
    app.cursor_pos = pos;
}

fn handle_confirm_key(app: &mut App, key: KeyEvent) {
    let allow = match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => true,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => false,
        _ => return,
    };
    if let Some(tx) = app.pending_confirm.take() {
        let _ = tx.send(allow);
    }
    // Return to Processing (agent continues after confirmation).
    app.mode = AppMode::Processing;
}

async fn handle_processing_key(
    app: &mut App,
    key: KeyEvent,
    _agent_tx: &mpsc::UnboundedSender<String>,
) {
    match key.code {
        KeyCode::Esc => {
            // Interrupt the running agent: abort the in-flight stream/turn and
            // clear any queued prompts.
            app.input.clear();
            app.cursor_pos = 0;
            app.request_cancel();
            app.set_status("Interrupting…");
        }
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                // Shift+Enter or Alt+Enter: insert newline
                app.input.insert(app.cursor_pos, '\n');
                app.cursor_pos += 1;
            } else {
                let input = app.input.trim().to_string();
                if !input.is_empty() {
                    app.add_to_history(input.clone());
                    app.input_queue.push_back(input.clone());
                    app.input.clear();
                    app.cursor_pos = 0;
                    app.commit(ChatMessage::System(format!("⏱ Queued: {}", input)));
                }
            }
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'h' {
                delete_prev_word(app);
            } else if c == '\n' {
                // Handle newline from Shift+Enter
                app.input.insert(app.cursor_pos, '\n');
                app.cursor_pos += 1;
            } else if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.input.insert(app.cursor_pos, c);
                app.cursor_pos += 1;
            }
        }
        KeyCode::Backspace => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                delete_prev_word(app);
            } else if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
        }
        KeyCode::Delete => {
            if app.cursor_pos < app.input.len() {
                app.input.remove(app.cursor_pos);
            }
        }
        KeyCode::Left => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
            }
        }
        KeyCode::Right => {
            if app.cursor_pos < app.input.len() {
                app.cursor_pos += 1;
            }
        }
        KeyCode::Home => {
            app.cursor_pos = 0;
        }
        KeyCode::End => {
            app.cursor_pos = app.input.len();
        }
        KeyCode::Up => {
            app.history_up();
        }
        KeyCode::Down => {
            app.history_down();
        }
        _ => {}
    }
}

async fn handle_normal_key(
    app: &mut App,
    key: KeyEvent,
    agent_tx: &mpsc::UnboundedSender<String>,
) -> bool {
    match key.code {
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                // Shift+Enter or Alt+Enter inserts a newline without submitting.
                app.input.insert(app.cursor_pos, '\n');
                app.cursor_pos += 1;
                return true;
            }

            let input = app.input.trim().to_string();
            if input.is_empty() {
                return true;
            }

            app.add_to_history(input.clone());
            app.input.clear();
            app.cursor_pos = 0;

            if input.starts_with('/') {
                handle_command(app, &input).await;
            } else {
                app.commit(ChatMessage::User(input.clone()));
                app.mode = AppMode::Processing;
                let _ = agent_tx.send(input);
            }
            true
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'h' {
                delete_prev_word(app);
            } else if c == '\n' {
                // Handle newline from Shift+Enter
                app.input.insert(app.cursor_pos, '\n');
                app.cursor_pos += 1;
            } else if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                app.input.insert(app.cursor_pos, c);
                app.cursor_pos += 1;
            }
            true
        }
        KeyCode::Backspace => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                delete_prev_word(app);
            } else if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
                app.input.remove(app.cursor_pos);
            }
            true
        }
        KeyCode::Delete => {
            if app.cursor_pos < app.input.len() {
                app.input.remove(app.cursor_pos);
            }
            true
        }
        KeyCode::Left => {
            if app.cursor_pos > 0 {
                app.cursor_pos -= 1;
            }
            true
        }
        KeyCode::Right => {
            if app.cursor_pos < app.input.len() {
                app.cursor_pos += 1;
            }
            true
        }
        KeyCode::Home => {
            app.cursor_pos = 0;
            true
        }
        KeyCode::End => {
            app.cursor_pos = app.input.len();
            true
        }
        KeyCode::Up => {
            app.history_up();
            true
        }
        KeyCode::Down => {
            app.history_down();
            true
        }
        KeyCode::Esc => {
            app.input.clear();
            app.cursor_pos = 0;
            true
        }
        KeyCode::Tab => {
            if app.input.starts_with('/') {
                let commands = [
                    "/help", "/import", "/model", "/provider", "/settings",
                    "/clear", "/cache", "/system", "/exit",
                ];
                let matches: Vec<&&str> = commands
                    .iter()
                    .filter(|c| c.starts_with(&app.input))
                    .collect();
                if matches.len() == 1 {
                    app.input = matches[0].to_string();
                    app.cursor_pos = app.input.len();
                }
            }
            true
        }
        _ => false,
    }
}

fn is_overlay_open(mode: &AppMode) -> bool {
    matches!(
        mode,
        AppMode::Help
            | AppMode::Settings(_)
            | AppMode::ModelSelect(_)
            | AppMode::ProviderSelect(_)
    )
}

async fn handle_command(app: &mut App, cmd: &str) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let command = parts[0].to_lowercase();
    let args = parts.get(1).unwrap_or(&"").trim();

    match command.as_str() {
        "/exit" | "/quit" => {
            app.should_quit = true;
        }
        "/help" => {
            if !is_overlay_open(&app.mode) {
                app.mode = AppMode::Help;
            }
        }
        "/clear" => {
            app.reveal = None;
            app.stream_buffer.clear();
            app.live = None;
            {
                let mut agent = app.shared_agent.lock().await;
                agent.clear_history();
                agent.inject_context(
                    "current_directory",
                    &format!("You are working in directory: {}", app.cwd),
                );
                agent.reinject_project_files();
            }
            app.commit(ChatMessage::System("Chat cleared. Starting fresh.".into()));
            app.set_status("Chat cleared — Ready");
        }
        "/cache" => {
            {
                let mut agent = app.shared_agent.lock().await;
                agent.clear_history();
                agent.inject_context(
                    "current_directory",
                    &format!("You are working in directory: {}", app.cwd),
                );
                agent.reinject_project_files();
            }
            app.commit(ChatMessage::System("Agent cache cleared. Project context re-injected.".into()));
            app.set_status("Cache cleared — Ready");
        }
        "/settings" => {
            if !is_overlay_open(&app.mode) {
                let state = SettingsState::from_config(&app.config);
                app.mode = AppMode::Settings(state);
            }
        }
        "/model" => {
            if !args.is_empty() {
                app.config.model = args.to_string();
                let _ = app.config.save();
                app.commit(ChatMessage::System(format!("Model set to: {}", args)));
                app.set_status(&format!("Model: {}", args));
            } else if !is_overlay_open(&app.mode) {
                app.mode = AppMode::ModelSelect(ModelSelectState {
                    models: vec![],
                    selected: 0,
                    loading: true,
                    return_to_settings: false,
                });
            }
        }
        "/provider" => {
            if !is_overlay_open(&app.mode) {
                app.mode = AppMode::ProviderSelect(ProviderSelectState::new(
                    &app.config.provider,
                    false,
                ));
            }
        }
        "/system" => {
            if !args.is_empty() {
                let prompt = args.to_string();
                {
                    let mut agent = app.shared_agent.lock().await;
                    agent.set_system_prompt(prompt);
                }
                app.commit(ChatMessage::System("Custom system prompt set.".into()));
            } else {
                app.commit(ChatMessage::System(
                    "Usage: /system <your custom system prompt>".into(),
                ));
            }
        }
        "/import" => {
            if args.is_empty() {
                app.commit(ChatMessage::System(
                    "Usage: /import <file_path>  — injects file contents into agent context".into(),
                ));
                return;
            }
            let path = crate::tools::shellexpand(args);
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    const MAX_IMPORT_BYTES: usize = 100 * 1024;
                    if content.len() > MAX_IMPORT_BYTES {
                        app.commit(ChatMessage::Error(format!(
                            "File too large to import ({} KB). Maximum is 100 KB.",
                            content.len() / 1024
                        )));
                        return;
                    }
                    let lines = content.lines().count();
                    let bytes = content.len();
                    {
                        let mut agent = app.shared_agent.lock().await;
                        agent.inject_context(args, &content);
                    }
                    app.commit(ChatMessage::System(format!(
                        "Imported: {args} ({lines} lines, {bytes} bytes) — now in context."
                    )));
                }
                Err(e) => {
                    app.commit(ChatMessage::Error(format!(
                        "Cannot import '{args}': {e}"
                    )));
                }
            }
        }
        _ => {
            app.commit(ChatMessage::Error(format!(
                "Unknown command: {}. Type /help for available commands.",
                command
            )));
        }
    }
}

fn handle_settings_key(app: &mut App, key: KeyEvent) {
    let state = match &mut app.mode {
        AppMode::Settings(s) => s,
        _ => return,
    };

    if state.editing {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                state.editing = false;
            }
            KeyCode::Char(c) => {
                state.fields[state.selected_field].value.push(c);
            }
            KeyCode::Backspace => {
                state.fields[state.selected_field].value.pop();
            }
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = AppMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if state.selected_field > 0 {
                state.selected_field -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
            if state.selected_field < state.fields.len() - 1 {
                state.selected_field += 1;
            }
        }
        KeyCode::Enter => {
            let field_key = state.fields[state.selected_field].key.clone();
            match field_key.as_str() {
                "provider" => {
                    let current = state.fields[state.selected_field].value.clone();
                    app.mode = AppMode::ProviderSelect(ProviderSelectState::new(&current, true));
                }
                "model" => {
                    app.mode = AppMode::ModelSelect(ModelSelectState {
                        models: vec![],
                        selected: 0,
                        loading: true,
                        return_to_settings: true,
                    });
                }
                _ => {
                    let state = match &mut app.mode {
                        AppMode::Settings(s) => s,
                        _ => return,
                    };
                    state.editing = true;
                }
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            let state_clone = state.clone();
            state_clone.apply_to_config(&mut app.config);
            let _ = app.config.save();
            app.commit(ChatMessage::System(format!(
                "Settings saved. Provider: {}",
                app.config.provider_display()
            )));
            app.set_status(&format!("Settings saved — {}", app.config.provider_display()));
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}

fn handle_provider_select_key(app: &mut App, key: KeyEvent) {
    let state = match &mut app.mode {
        AppMode::ProviderSelect(s) => s.clone(),
        _ => return,
    };

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            if state.return_to_settings {
                app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
            } else {
                app.mode = AppMode::Normal;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::ProviderSelect(s) = &mut app.mode {
                if s.selected > 0 {
                    s.selected -= 1;
                }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::ProviderSelect(s) = &mut app.mode {
                if s.selected < s.providers.len().saturating_sub(1) {
                    s.selected += 1;
                }
            }
        }
        KeyCode::Enter => {
            if let Some(provider) = state.providers.get(state.selected) {
                app.config.provider = provider.clone();
                let _ = app.config.save();
                app.commit(ChatMessage::System(format!(
                    "Provider set to: {provider}"
                )));
                app.set_status(&format!("Provider: {}", app.config.provider_display()));
            }
            if state.return_to_settings {
                app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
            } else {
                app.mode = AppMode::Normal;
            }
        }
        _ => {}
    }
}

fn handle_model_select_key(app: &mut App, key: KeyEvent) {
    let (selected_model, return_to_settings) = match &mut app.mode {
        AppMode::ModelSelect(s) => {
            if s.loading {
                if key.code == KeyCode::Esc {
                    if s.return_to_settings {
                        app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
                    } else {
                        app.mode = AppMode::Normal;
                    }
                }
                return;
            }

            let return_to_settings = s.return_to_settings;
            let selected_model = match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    if return_to_settings {
                        app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
                    } else {
                        app.mode = AppMode::Normal;
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if s.selected > 0 {
                        s.selected -= 1;
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if s.selected < s.models.len().saturating_sub(1) {
                        s.selected += 1;
                    }
                    None
                }
                KeyCode::Enter => {
                    let model = s.models.get(s.selected).cloned();
                    if model.is_some() {
                        if return_to_settings {
                            app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
                        } else {
                            app.mode = AppMode::Normal;
                        }
                    }
                    model
                }
                _ => None,
            };
            (selected_model, return_to_settings)
        }
        _ => return,
    };

    if let Some(model) = selected_model {
        app.config.model = model.clone();
        let _ = app.config.save();

        if return_to_settings {
            if let AppMode::Settings(ref mut settings) = app.mode {
                if let Some(field) = settings.fields.iter_mut().find(|f| f.key == "model") {
                    field.value = model.clone();
                }
            }
        }

        app.commit(ChatMessage::System(format!("Model set to: {model}")));
        app.set_status(&format!("Model: {model}"));
    }
}
