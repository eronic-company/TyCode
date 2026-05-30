#![allow(dead_code)]

mod agent;
mod config;
mod provider;
mod tools;
mod tui;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind, EnableMouseCapture, DisableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::tui::app::{App, AppMode, ChatMessage};
use crate::tui::{input, ui};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load();

    // Ensure provider is ready (e.g. start Ollama, pull model)
    println!("Initializing {} / {}...", config.provider, config.model);
    if let Err(e) = provider::ensure_provider_ready(&config).await {
        eprintln!("Warning: Provider initialization failed: {e}");
        println!("Press Enter to continue anyway...");
        let mut buf = String::new();
        let _ = std::io::stdin().read_line(&mut buf);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, config).await;

    // Always restore terminal, even on error or signal.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    if let Err(ref e) = result {
        eprintln!("Error: {e}");
    }

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    config: Config,
) -> Result<()> {
    let mut app = App::new(config);

    // Inject TYCODE.md and README.md from cwd into agent context.
    {
        let cwd = app.cwd.clone();
        let mut agent = app.shared_agent.lock().await;
        agent.inject_project_files(&cwd);
    }

    // Channel: user prompts → agent spawner
    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<String>();

    // Channel: agent events → TUI
    let (agent_event_tx, mut agent_event_rx) =
        mpsc::unbounded_channel::<crate::agent::AgentEvent>();

    // Channel: model list fetcher → TUI
    let (model_tx, mut model_rx) = mpsc::unbounded_channel::<Vec<String>>();

    // Channel: OS signals → quit
    let (signal_tx, mut signal_rx) = mpsc::unbounded_channel::<()>();

    // Spawn a task that watches for SIGHUP / SIGTERM and forwards to the main loop.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let tx = signal_tx.clone();
        tokio::spawn(async move {
            let mut sighup = signal(SignalKind::hangup()).expect("SIGHUP handler");
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
            tokio::select! {
                _ = sighup.recv() => {}
                _ = sigterm.recv() => {}
            }
            let _ = tx.send(());
        });
    }

    let mut tick_count: u64 = 0;

    loop {
        // ── Render ───────────────────────────────────────────────────────
        terminal.draw(|f| ui::render(f, &mut app))?;

        // ── Poll input ───────────────────────────────────────────────────
        let timeout = if matches!(app.mode, AppMode::Processing | AppMode::Confirm(_)) {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    input::handle_key(&mut app, key, &user_tx).await;
                }
                Event::Mouse(mouse) => {
                    input::handle_mouse(&mut app, mouse);
                }
                _ => {}
            }
        }

        // ── Drain agent events ───────────────────────────────────────────
        while let Ok(evt) = agent_event_rx.try_recv() {
            app.handle_agent_event(evt);
        }

        // ── Drain model list results ─────────────────────────────────────
        while let Ok(models) = model_rx.try_recv() {
            if let AppMode::ModelSelect(ref mut state) = app.mode {
                state.models = models;
                state.loading = false;
                state.selected = 0;
            }
        }

        // ── Process queued messages after task completes ──────────────────
        if matches!(app.mode, AppMode::Normal) && !app.input_queue.is_empty() {
            if let Some(prompt) = app.input_queue.pop_front() {
                app.messages.push(ChatMessage::User(prompt.clone()));
                app.mode = AppMode::Processing;
                app.scroll_to_bottom();
                let _ = user_tx.send(prompt);
            }
        }

        // ── Spawn agent task on new user prompt ──────────────────────────
        while let Ok(prompt) = user_rx.try_recv() {
            let config = app.config.clone();
            let event_tx = agent_event_tx.clone();
            let agent_ref = Arc::clone(&app.shared_agent);

            // Clear any stale cancel from a previous run before starting.
            app.cancel_flag.store(false, std::sync::atomic::Ordering::SeqCst);
            let cancel = Arc::clone(&app.cancel_flag);
            let cancel_notify = Arc::clone(&app.cancel_notify);

            tokio::spawn(async move {
                let mut agent = agent_ref.lock().await;
                let _ = agent.run(prompt, &config, event_tx, cancel, cancel_notify).await;
            });
        }

        // ── Fetch models when selector opens ─────────────────────────────
        if let AppMode::ModelSelect(ref mut state) = app.mode {
            if state.loading && state.models.is_empty() {
                let config = app.config.clone();
                let tx = model_tx.clone();
                tokio::spawn(async move {
                    let models = match provider::create_provider(&config) {
                        Ok(p) => {
                            let models = p.available_models().await;
                            if models.is_empty() {
                                vec!["(no models available)".into()]
                            } else {
                                models
                            }
                        }
                        Err(e) => vec![format!("(failed to connect: {})", e)],
                    };
                    let _ = tx.send(models);
                });
                state.models = vec!["loading...".into()];
            }
        }

        // ── Animate thinking dots ────────────────────────────────────────
        if matches!(app.mode, AppMode::Processing | AppMode::Confirm(_)) {
            tick_count += 1;
            if tick_count % 3 == 0 {
                app.thinking_dots = (app.thinking_dots + 1) % 4;
            }
        }

        // ── Check status message expiry ──────────────────────────────────
        app.update_status_expiry();

        // ── OS signal → clean exit ───────────────────────────────────────
        if signal_rx.try_recv().is_ok() {
            app.should_quit = true;
        }

        if app.should_quit {
            return Ok(());
        }
    }
}
