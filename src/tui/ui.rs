use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, Padding, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::app::{App, AppMode, ChatMessage, ConfirmState, ModelSelectState, ProviderSelectState, SettingsState};
use super::markdown;

// ── Colors ───────────────────────────────────────────────────────────────────

const USER_COLOR: Color = Color::Cyan;
const TOOL_COLOR: Color = Color::Yellow;
const TOOL_SUCCESS: Color = Color::Green;
const TOOL_FAIL: Color = Color::Red;
const ERROR_COLOR: Color = Color::Red;
const SYSTEM_COLOR: Color = Color::DarkGray;
const HEADER_BG: Color = Color::Rgb(30, 30, 50);
const STATUS_BG: Color = Color::Rgb(30, 30, 50);
const BORDER_COLOR: Color = Color::Rgb(80, 80, 100);
const DIM: Color = Color::DarkGray;

// ── Main render ──────────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Dynamic input height: grows with content (and newlines) up to 8 rows.
    let inner_width = size.width.saturating_sub(2) as usize;
    let input_rows = if app.input.is_empty() || inner_width == 0 {
        1u16
    } else {
        // Count explicit newlines plus display-width wrapping.
        let mut rows: usize = 0;
        for segment in app.input.split('\n') {
            let w = UnicodeWidthStr::width(segment);
            rows += ((w.max(1) - 1) / inner_width) + 1;
        }
        rows.min(8) as u16
    };
    let input_height = (input_rows + 2).max(3); // +2 for borders

    // Layout: header (1), chat (flex), input (dynamic), status (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),            // header
            Constraint::Min(5),               // chat
            Constraint::Length(input_height), // input
            Constraint::Length(1),            // status
        ])
        .split(size);

    render_header(f, app, chunks[0]);
    render_chat(f, app, chunks[1]);
    render_input(f, app, chunks[2]);
    render_status(f, app, chunks[3]);

    // Overlay screens
    match &app.mode {
        AppMode::Settings(state) => render_settings_overlay(f, state.clone(), size),
        AppMode::Help => render_help_overlay(f, size),
        AppMode::ModelSelect(state) => render_model_select_overlay(f, state.clone(), size),
        AppMode::ProviderSelect(state) => render_provider_select_overlay(f, state.clone(), size),
        AppMode::Confirm(state) => render_confirm_overlay(f, state.clone(), size),
        _ => {}
    }
}

// ── Header ───────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let provider_info = app.config.provider_display();
    let cwd_short = shorten_path(&app.cwd, (area.width as usize).saturating_sub(provider_info.len() + 20));

    let tycode_width = " ◈ TyCode ".len();
    let provider_width = format!(" {} ", provider_info).len();
    let cwd_width = format!(" {} ", cwd_short).len();
    let total_used = tycode_width + provider_width + cwd_width;
    let padding = (area.width as usize).saturating_sub(total_used);

    let header = Line::from(vec![
        Span::styled(
            " ◈ TyCode ",
            Style::default()
                .fg(Color::White)
                .bg(Color::Rgb(100, 60, 180))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", provider_info),
            Style::default().fg(Color::Cyan).bg(HEADER_BG),
        ),
        Span::styled(
            " ".repeat(padding),
            Style::default().bg(HEADER_BG),
        ),
        Span::styled(
            format!(" {} ", cwd_short),
            Style::default().fg(DIM).bg(HEADER_BG),
        ),
    ]);

    let header_widget = Paragraph::new(header).style(Style::default().bg(HEADER_BG));
    f.render_widget(header_widget, area);
}

// ── Chat area ────────────────────────────────────────────────────────────────

fn render_chat(f: &mut Frame, app: &mut App, area: Rect) {
    let mut all_lines: Vec<Line<'static>> = Vec::new();

    for (msg_idx, msg) in app.messages.iter().enumerate() {
        let msg_start_line = all_lines.len();
        let is_selected = app.selected_message == Some(msg_idx);

        match msg {
            ChatMessage::User(text) => {
                all_lines.push(Line::from(""));
                all_lines.push(Line::from(vec![
                    Span::styled(
                        "  You ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(USER_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                for line in text.lines() {
                    all_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(line.to_string(), Style::default().fg(USER_COLOR)),
                    ]));
                }
            }
            ChatMessage::AssistantText(text) => {
                all_lines.push(Line::from(""));
                let md_lines = markdown::markdown_to_lines(text);
                for line in md_lines {
                    let mut prefixed: Vec<Span<'static>> = vec![Span::raw("  ")];
                    prefixed.extend(line.spans);
                    all_lines.push(Line::from(prefixed));
                }
            }
            ChatMessage::AssistantLive(text) => {
                all_lines.push(Line::from(""));
                let md_lines = markdown::markdown_to_lines(text);
                for (i, line) in md_lines.iter().enumerate() {
                    let mut prefixed: Vec<Span<'static>> = vec![Span::raw("  ")];
                    prefixed.extend(line.spans.clone());
                    // Append live cursor to last line.
                    if i == md_lines.len() - 1 {
                        prefixed.push(Span::styled(
                            "▊",
                            Style::default().fg(Color::Magenta).add_modifier(Modifier::SLOW_BLINK),
                        ));
                    }
                    all_lines.push(Line::from(prefixed));
                }
            }
            ChatMessage::ToolCall {
                name,
                input_summary,
                success,
                output,
                expanded,
            } => {
                let status_color = match success {
                    Some(true) => TOOL_SUCCESS,
                    Some(false) => TOOL_FAIL,
                    None => TOOL_COLOR,
                };
                let status = match success {
                    Some(true) => "✓",
                    Some(false) => "✗",
                    None => "●",
                };
                let toggle_indicator = if *expanded { "▼" } else { "▶" };
                all_lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {toggle_indicator}  "),
                        Style::default().fg(DIM),
                    ),
                    Span::styled(
                        name.clone(),
                        Style::default()
                            .fg(TOOL_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {input_summary}"),
                        Style::default().fg(DIM),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        status.to_string(),
                        Style::default().fg(status_color),
                    ),
                ]));
                // Show output only if expanded
                if *expanded {
                    if let Some(out) = output {
                        let out_lines: Vec<&str> = out.lines().collect();
                        for line in out_lines {
                            all_lines.push(Line::from(vec![
                                Span::styled(
                                    format!("     {}", line),
                                    Style::default().fg(DIM),
                                ),
                            ]));
                        }
                    }
                }
            }
            ChatMessage::System(text) => {
                for line in text.lines() {
                    all_lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {line}"),
                            Style::default().fg(SYSTEM_COLOR).add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
            }
            ChatMessage::Error(text) => {
                all_lines.push(Line::from(vec![
                    Span::styled(
                        format!("  Error: {text}"),
                        Style::default()
                            .fg(ERROR_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
        }

        // Add selection highlight gutter to first line of selected message
        if is_selected && msg_start_line < all_lines.len() {
            let line = all_lines[msg_start_line].clone();
            let gutter = Span::styled("▌ ", Style::default().fg(Color::Rgb(100, 100, 160)));
            let mut new_spans = vec![gutter];
            new_spans.extend(line.spans);
            all_lines[msg_start_line] = Line::from(new_spans);
        }
    }

    // Thinking indicator (only when not streaming).
    let is_streaming = app.messages.last().map(|m| matches!(m, ChatMessage::AssistantLive(_))).unwrap_or(false);
    if matches!(app.mode, AppMode::Processing | AppMode::Confirm(_)) && !is_streaming {
        let dots = ".".repeat((app.thinking_dots % 4) + 1);
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(vec![
            Span::styled(
                format!("  Thinking{dots}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let content_width = area.width.saturating_sub(1);
    let total_lines = compute_wrapped_height(&all_lines, content_width);
    let visible_height = area.height;

    // Auto-scroll: sentinel u16::MAX → scroll to actual bottom.
    if app.scroll_offset == u16::MAX {
        app.scroll_offset = total_lines.saturating_sub(visible_height);
    }
    app.scroll_offset = app
        .scroll_offset
        .min(total_lines.saturating_sub(visible_height));

    let text = Text::from(all_lines);
    let chat_widget = Paragraph::new(text)
        .scroll((app.scroll_offset, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(chat_widget, area);

    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(total_lines as usize)
                .position(app.scroll_offset as usize)
                .viewport_content_length(visible_height as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(BORDER_COLOR));
        f.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

// ── Input area ───────────────────────────────────────────────────────────────

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let is_processing = matches!(app.mode, AppMode::Processing | AppMode::Confirm(_));

    let border_color = if is_processing {
        Color::DarkGray
    } else {
        BORDER_COLOR
    };

    // Count newlines for the line badge.
    let line_count = app.input.chars().filter(|&c| c == '\n').count() + 1;
    let line_badge = if line_count > 1 {
        format!(" · {} lines", line_count)
    } else {
        String::new()
    };

    let title = if is_processing {
        if app.input_queue.is_empty() {
            " Processing... (ESC to clear queue) ".to_string()
        } else {
            format!(" Processing... {} queued ", app.input_queue.len())
        }
    } else {
        let history_pos = app.get_history_position_text();
        format!(" >{}{} ", history_pos, line_badge)
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(if is_processing {
            Color::DarkGray
        } else {
            Color::Cyan
        }));

    let input_widget = if app.input.is_empty() && !is_processing {
        Paragraph::new(Line::from(Span::styled(
            "Type your prompt or /help for commands... (Shift/Alt+Enter for newline)",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )))
        .block(input_block)
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false })
    } else {
        let input_text = if app.input.contains('\n') {
            // Multi-line: no ghost hint, just render as-is.
            Line::from(Span::raw(app.input.clone()))
        } else if let Some(hint) = get_command_hints(&app.input) {
            let typed = &app.input;
            let untyped = &hint[typed.len()..];
            Line::from(vec![
                Span::styled(typed.to_string(), Style::default().fg(Color::White)),
                Span::styled(untyped.to_string(), Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
            ])
        } else {
            Line::from(Span::raw(app.input.clone()))
        };
        Paragraph::new(input_text)
            .block(input_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false })
    };

    f.render_widget(input_widget, area);

    // Cursor position — account for physical newlines and display wrapping.
    if !matches!(app.mode, AppMode::Settings(_) | AppMode::Help | AppMode::ModelSelect(_) | AppMode::ProviderSelect(_)) {
        let inner_width = area.width.saturating_sub(2) as usize;
        if inner_width > 0 {
            let text_before = &app.input[..app.cursor_pos.min(app.input.len())];
            let segments: Vec<&str> = text_before.split('\n').collect();
            let mut cursor_row: usize = 0;
            let mut cursor_col: usize = 0;
            for (i, segment) in segments.iter().enumerate() {
                let w = UnicodeWidthStr::width(*segment);
                if i < segments.len() - 1 {
                    cursor_row += w / inner_width + 1;
                } else {
                    cursor_row += w / inner_width;
                    cursor_col = w % inner_width;
                }
            }

            let cursor_x = area.x + 1 + cursor_col as u16;
            let cursor_y = area.y + 1 + cursor_row as u16;
            if cursor_x < area.x + area.width - 1 && cursor_y < area.y + area.height - 1 {
                f.set_cursor_position((cursor_x, cursor_y));
            }
        }
    }
}

// ── Status bar ───────────────────────────────────────────────────────────────

fn fmt_k(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}K", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let commands = " /help  /model  /settings  /clear  /import │ PgUp/Dn C-c×2=quit ";

    // Context token estimate with color.
    let ctx_estimate = app.context_token_estimate();
    let ctx_color = if ctx_estimate > 90_000 {
        Color::Red
    } else if ctx_estimate > 60_000 {
        Color::Yellow
    } else {
        Color::Green
    };
    let ctx_str = fmt_k(ctx_estimate as u32);

    // Token usage display.
    let token_info = if app.last_turn_in > 0 || app.last_turn_out > 0 {
        format!(
            "ctx ~{}K | ↑{} ↓{} tok",
            ctx_str,
            fmt_k(app.last_turn_in),
            fmt_k(app.last_turn_out),
        )
    } else {
        format!("ctx ~{}K", ctx_str)
    };

    let status = &app.status_message;
    let status_with_indicator = if let Some(ts) = app.status_timestamp {
        let elapsed = ts.elapsed().as_millis() as u64;
        let remaining = (3000u64).saturating_sub(elapsed);
        let dots = if remaining > 2000 { "●" } else if remaining > 1000 { "○" } else { "·" };
        format!("{} {}", status, dots)
    } else {
        status.clone()
    };

    let right_section = format!("{} │ {} ", token_info, status_with_indicator);
    let padding = (area.width as usize)
        .saturating_sub(commands.len() + right_section.len());

    let status_line = Line::from(vec![
        Span::styled(commands, Style::default().fg(DIM).bg(STATUS_BG)),
        Span::styled(" ".repeat(padding), Style::default().bg(STATUS_BG)),
        Span::styled(
            format!("{} │ ", token_info),
            Style::default().fg(ctx_color).bg(STATUS_BG),
        ),
        Span::styled(
            format!("{} ", status_with_indicator),
            Style::default().fg(Color::Green).bg(STATUS_BG),
        ),
    ]);

    let status_widget = Paragraph::new(status_line).style(Style::default().bg(STATUS_BG));
    f.render_widget(status_widget, area);
}

// ── Confirm overlay ──────────────────────────────────────────────────────────

fn render_confirm_overlay(f: &mut Frame, state: ConfirmState, area: Rect) {
    let width = 70u16.min(area.width.saturating_sub(4));
    let height = 10u16.min(area.height.saturating_sub(4));
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ⚠  Dangerous Command ")
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" Y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("/Enter=Allow  ", Style::default().fg(Color::DarkGray)),
            Span::styled("N", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled("/Esc=Deny ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(Color::Rgb(30, 10, 10)));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let lines = vec![
        Line::from(vec![
            Span::styled("Reason: ", Style::default().fg(Color::DarkGray)),
            Span::styled(state.reason.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Command: ", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}", state.command),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Allow this command to run?",
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
            ),
        ]),
    ];

    let widget = Paragraph::new(lines)
        .style(Style::default().bg(Color::Rgb(30, 10, 10)))
        .wrap(Wrap { trim: false });
    f.render_widget(widget, inner);
}

// ── Settings overlay ─────────────────────────────────────────────────────────

const POPUP_BG: Color = Color::Rgb(18, 18, 32);
const POPUP_BORDER: Color = Color::Rgb(100, 100, 160);
const POPUP_TITLE: Color = Color::Rgb(130, 180, 255);

fn render_settings_overlay(f: &mut Frame, state: SettingsState, area: Rect) {
    let width = 72u16.min(area.width.saturating_sub(4));
    let height = (state.fields.len() as u16 + 8).min(area.height.saturating_sub(4));
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ⚙  Settings ")
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" Enter", Style::default().fg(Color::Cyan)),
            Span::styled("=edit/pick  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::styled("=next  ", Style::default().fg(Color::DarkGray)),
            Span::styled("S", Style::default().fg(Color::Cyan)),
            Span::styled("=save  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::styled("=close ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, field) in state.fields.iter().enumerate() {
        let is_selected = i == state.selected_field;
        let cursor = if is_selected { "▶ " } else { "  " };

        let label_style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(60, 100, 200))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(180, 180, 220))
        };

        let value_display = if field.key == "provider" {
            format!("{}  ◀▶", field.value)
        } else if field.key.contains("key") && !field.value.is_empty() && !(state.editing && is_selected) {
            let visible = field.value.len().min(4);
            format!("{}...", &field.value[..visible])
        } else {
            field.value.clone()
        };

        let value_style = if is_selected && state.editing {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::UNDERLINED)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(Color::White).bg(Color::Rgb(60, 100, 200))
        } else {
            Style::default().fg(Color::Rgb(100, 220, 130))
        };

        lines.push(Line::from(vec![
            Span::styled(cursor.to_string(), label_style),
            Span::styled(format!("{:<22}", field.label), label_style),
            Span::styled(value_display, value_style),
        ]));
    }

    let settings_widget = Paragraph::new(lines).style(Style::default().bg(POPUP_BG));
    f.render_widget(settings_widget, inner);
}

// ── Help overlay ─────────────────────────────────────────────────────────────

fn render_help_overlay(f: &mut Frame, area: Rect) {
    let width = 70u16.min(area.width.saturating_sub(4));
    let height = 32u16.min(area.height.saturating_sub(4));
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ?  Help ")
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" Esc", Style::default().fg(Color::Cyan)),
            Span::styled("=close ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let cmd = Style::default().fg(Color::Rgb(130, 200, 255));
    let desc = Style::default().fg(Color::Rgb(200, 200, 220));
    let section = Style::default().fg(Color::Rgb(255, 200, 80)).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);

    let help_text = vec![
        Line::from(vec![Span::styled("  Commands", section)]),
        Line::from(""),
        Line::from(vec![Span::styled("  /help        ", cmd), Span::styled("Show this help screen", desc)]),
        Line::from(vec![Span::styled("  /model       ", cmd), Span::styled("Switch model (fetches available models)", desc)]),
        Line::from(vec![Span::styled("  /provider    ", cmd), Span::styled("Pick provider from list", desc)]),
        Line::from(vec![Span::styled("  /settings    ", cmd), Span::styled("Open settings editor", desc)]),
        Line::from(vec![Span::styled("  /clear       ", cmd), Span::styled("Clear chat + agent context (re-injects project files)", desc)]),
        Line::from(vec![Span::styled("  /cache       ", cmd), Span::styled("Reset agent memory only (keep chat display)", desc)]),
        Line::from(vec![Span::styled("  /copy        ", cmd), Span::styled("Copy last 50 messages to /tmp/tycode_copy.txt", desc)]),
        Line::from(vec![Span::styled("  /system      ", cmd), Span::styled("Set custom system prompt", desc)]),
        Line::from(vec![Span::styled("  /import      ", cmd), Span::styled("/import <path>  inject file into agent context", desc)]),
        Line::from(""),
        Line::from(vec![Span::styled("  Keyboard & Navigation", section)]),
        Line::from(""),
        Line::from(vec![Span::styled("  Enter             ", cmd), Span::styled("Send message", desc)]),
        Line::from(vec![Span::styled("  Shift/Alt+Enter   ", cmd), Span::styled("Insert newline (multiline input)", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Backspace    ", cmd), Span::styled("Delete previous word", desc)]),
        Line::from(vec![Span::styled("  Ctrl+C            ", cmd), Span::styled("Cancel/clear (×2 within 2s to quit)", desc)]),
        Line::from(vec![Span::styled("  Up / Down         ", cmd), Span::styled("Navigate input history", desc)]),
        Line::from(vec![Span::styled("  PgUp / PgDown     ", cmd), Span::styled("Scroll chat (10 lines)", desc)]),
        Line::from(vec![Span::styled("  Shift+Drag        ", cmd), Span::styled("Select text (terminal native)", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Shift+C      ", cmd), Span::styled("Copy selected text (terminal native)", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Home         ", cmd), Span::styled("Jump to top of chat", desc)]),
        Line::from(vec![Span::styled("  Ctrl+End          ", cmd), Span::styled("Jump to bottom of chat", desc)]),
        Line::from(vec![Span::styled("  Tab               ", cmd), Span::styled("Auto-complete slash commands", desc)]),
        Line::from(vec![Span::styled("  Esc               ", cmd), Span::styled("Close overlay / clear input", desc)]),
        Line::from(""),
        Line::from(vec![Span::styled("  While Processing", section)]),
        Line::from(""),
        Line::from(vec![Span::styled("  Enter             ", cmd), Span::styled("Queue additional instructions", desc)]),
        Line::from(vec![Span::styled("  Shift/Alt+Enter   ", cmd), Span::styled("Insert newline in queued message", desc)]),
        Line::from(vec![Span::styled("  Esc               ", cmd), Span::styled("Clear queue (agent task continues)", desc)]),
        Line::from(""),
        Line::from(vec![Span::styled("  Dangerous commands prompt for Y/N confirmation before executing.", dim)]),
    ];

    let help_widget = Paragraph::new(help_text)
        .block(block)
        .style(Style::default().bg(POPUP_BG));
    f.render_widget(help_widget, popup_area);
}

// ── Model select overlay ─────────────────────────────────────────────────────

fn render_model_select_overlay(f: &mut Frame, state: ModelSelectState, area: Rect) {
    let width = 55u16.min(area.width.saturating_sub(4));
    let height = (state.models.len() as u16 + 6)
        .min(area.height.saturating_sub(4))
        .max(8);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let title = if state.loading { " ◌  Loading models... " } else { " ◈  Select Model " };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(if state.loading {
            Line::from(vec![
                Span::styled(" ↵", Style::default().fg(Color::Yellow)),
                Span::styled("cancel ", Style::default().fg(Color::DarkGray)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(Color::Yellow)),
                Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("↵", Style::default().fg(Color::Yellow)),
                Span::styled("select  ", Style::default().fg(Color::DarkGray)),
                Span::styled("esc", Style::default().fg(Color::Yellow)),
                Span::styled("=exit ", Style::default().fg(Color::DarkGray)),
            ])
        })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    if state.loading {
        let loading = Paragraph::new(Line::from(vec![
            Span::styled("  ⏳ ", Style::default().fg(Color::Yellow)),
            Span::styled("Fetching available models...", Style::default().fg(Color::DarkGray)),
        ]))
        .block(block)
        .style(Style::default().bg(POPUP_BG));
        f.render_widget(loading, popup_area);
        return;
    }

    let items: Vec<ListItem> = state.models.iter().enumerate().map(|(i, model)| {
        if i == state.selected {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("► {} ", model),
                    Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD),
                ),
            ]))
        } else {
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {} ", model), Style::default().fg(Color::Rgb(180, 180, 220))),
            ]))
        }
    }).collect();

    let list = List::new(items).block(block).style(Style::default().bg(POPUP_BG));
    f.render_widget(list, popup_area);
}

// ── Provider select overlay ──────────────────────────────────────────────────

fn render_provider_select_overlay(f: &mut Frame, state: ProviderSelectState, area: Rect) {
    let width = 42u16.min(area.width.saturating_sub(4));
    let height = (state.providers.len() as u16 + 6)
        .min(area.height.saturating_sub(4))
        .max(8);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ◈  Select Provider ")
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" ↑↓", Style::default().fg(Color::Yellow)),
            Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("↵", Style::default().fg(Color::Yellow)),
            Span::styled("select  ", Style::default().fg(Color::DarkGray)),
            Span::styled("esc", Style::default().fg(Color::Yellow)),
            Span::styled("=exit ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let items: Vec<ListItem> = state.providers.iter().enumerate().map(|(i, provider)| {
        if i == state.selected {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("► {} ", provider),
                    Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD),
                ),
            ]))
        } else {
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {} ", provider), Style::default().fg(Color::Rgb(180, 180, 220))),
            ]))
        }
    }).collect();

    let list = List::new(items).block(block).style(Style::default().bg(POPUP_BG));
    f.render_widget(list, popup_area);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_command_hints(input: &str) -> Option<&'static str> {
    let commands = [
        "/help", "/model", "/settings", "/clear", "/import", "/system", "/provider", "/exit",
    ];
    if input.starts_with('/') {
        for cmd in &commands {
            if cmd.starts_with(input) {
                return Some(cmd);
            }
        }
    }
    None
}

fn compute_wrapped_height(lines: &[Line], width: u16) -> u16 {
    if width == 0 {
        return lines.len() as u16;
    }
    let mut total: u16 = 0;
    for line in lines {
        let line_width: usize = line.spans.iter().map(|s| s.content.width()).sum();
        if line_width == 0 {
            total = total.saturating_add(1);
        } else {
            total = total.saturating_add(((line_width as u16).saturating_sub(1)) / width + 1);
        }
    }
    total
}

fn shorten_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }
    let home = dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let shortened = if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };
    if shortened.len() <= max_len {
        shortened
    } else {
        format!("...{}", &shortened[shortened.len().saturating_sub(max_len - 3)..])
    }
}
