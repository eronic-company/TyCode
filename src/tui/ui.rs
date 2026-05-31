use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::app::{App, AppMode, AddKeyStep, ChatMessage, ConfirmState, KeyManageState, KeySelectState, ModelSelectState, ProviderSelectState, SettingsState};
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

    // Dynamic input height: grows with content up to 6 rows.
    let inner_width = size.width.saturating_sub(2) as usize;
    let input_rows = if app.input.is_empty() || inner_width == 0 {
        1u16
    } else {
        let mut rows: usize = 0;
        for segment in app.input.split('\n') {
            let w = UnicodeWidthStr::width(segment);
            rows += ((w.max(1) - 1) / inner_width) + 1;
        }
        rows.min(6) as u16
    };
    let input_height = (input_rows + 2).max(3); // +2 for borders

    // Compact inline viewport: live in-progress block (Min, flexible), the
    // input box, and the status bar. Everything finalized lives in the
    // terminal's own scrollback above the viewport.
    let header = header_lines(size.width);
    let header_h = header.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),     // pinned header box
            Constraint::Min(1),               // live block / confirm prompt
            Constraint::Length(input_height), // input
            Constraint::Length(1),            // status
        ])
        .split(size);

    f.render_widget(Paragraph::new(Text::from(header)), chunks[0]);
    render_live(f, app, chunks[1]);
    render_input(f, app, chunks[2]);
    render_status(f, app, chunks[3]);

    // Overlays (settings/help/model/provider) render over the viewport.
    match &app.mode {
        AppMode::Settings(state) => render_settings_overlay(f, state.clone(), size),
        AppMode::Help => render_help_overlay(f, size),
        AppMode::ModelSelect(state) => render_model_select_overlay(f, state.clone(), size),
        AppMode::ProviderSelect(state) => render_provider_select_overlay(f, state.clone(), size),
        AppMode::KeySelect(state) => render_key_select_overlay(f, state.clone(), size),
        AppMode::KeyManage(state) => render_key_manage_overlay(f, state.clone(), size),
        _ => {}
    }
}

// ── Live block ───────────────────────────────────────────────────────────────

/// Render the in-progress block (streaming reply / running tool / thinking
/// spinner) or, in confirm mode, the dangerous-command prompt — bottom-aligned
/// so it sits just above the input box.
fn render_live(f: &mut Frame, app: &App, area: Rect) {
    if let AppMode::Confirm(state) = &app.mode {
        let lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                format!("  ⚠ {}", state.reason),
                Style::default().fg(TOOL_FAIL).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::styled(
                format!("  $ {}", state.command),
                Style::default().fg(Color::White),
            )]),
            Line::from(vec![Span::styled(
                "  Run this command?  [y/N]",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )]),
        ];
        let para = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        f.render_widget(para, area);
        return;
    }

    let width = area.width.saturating_sub(1);
    let mut lines: Vec<Line<'static>> = Vec::new();

    if let Some(live) = &app.live {
        lines.extend(message_lines(live, width));
    } else if matches!(app.mode, AppMode::Processing) {
        let dots = ".".repeat((app.thinking_dots % 4) + 1);
        lines.push(Line::from(vec![Span::styled(
            format!("  Thinking{dots}"),
            Style::default().fg(Color::Magenta).add_modifier(Modifier::ITALIC),
        )]));
    }

    if lines.is_empty() {
        return;
    }

    // Bottom-align: show the tail that fits in the live area.
    let para = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .scroll((0, 0));
    f.render_widget(para, area);
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

// ── Header ───────────────────────────────────────────────────────────────────

/// Bordered welcome box (Claude-Code style) with a block-art shark mascot in a
/// dark blue / dark purple palette. Printed once into scrollback at startup.
pub fn header_lines(width: u16) -> Vec<Line<'static>> {
    let w = (width as usize).max(46);
    let inner = w - 2;
    let dw = |s: &str| UnicodeWidthStr::width(s);

    let border = Style::default().fg(Color::Rgb(60, 70, 130));
    let body = Style::default().fg(Color::Rgb(95, 75, 140));
    let belly = Style::default().fg(Color::Rgb(140, 120, 185));
    let eye = Style::default().fg(Color::Rgb(200, 200, 230));
    let title = Style::default().fg(Color::Rgb(150, 130, 205)).add_modifier(Modifier::BOLD);
    let text = Style::default().fg(Color::Rgb(120, 130, 180));

    // (shark-segment, color) for the left column, and right-column info text.
    let left_w = 18usize;
    let shark: Vec<Vec<(&str, Style)>> = vec![
        vec![("     ▄▟▛▙▄", body)],
        vec![("   ▟███████▙▖", body)],
        vec![("  █ ", body), ("◣", eye), ("██████▙▖", body)],
        vec![(" ▜███", body), ("▚▚▚", belly), ("█████◣", body)],
        vec![("   ▀▜█▛▀▜█▛▀ ", body), ("〜", border)],
    ];
    let info: Vec<(&str, Style)> = vec![
        ("Welcome to TyCode", title),
        ("AI system agent · works with any LLM", text),
        ("", text),
        ("/help  /model  /settings  /clear", text),
        ("ctrl + c ×2 = quit/exit", text),
    ];

    let pad = |used: usize, target: usize| " ".repeat(target.saturating_sub(used));

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Top border with title (version pulled from Cargo at build time).
    let ttl = format!(" TyCode v{} ", env!("CARGO_PKG_VERSION"));
    let ttl = ttl.as_str();
    let dash = inner.saturating_sub(1 + dw(ttl));
    lines.push(Line::from(vec![
        Span::styled(format!("╭─{ttl}"), border),
        Span::styled("─".repeat(dash), border),
        Span::styled("╮", border),
    ]));

    for row in 0..shark.len().max(info.len()) {
        let mut spans: Vec<Span<'static>> = vec![Span::styled("│ ", border)];
        // Left column (shark), measured by display width.
        let mut lw = 0usize;
        if let Some(seg) = shark.get(row) {
            for (s, st) in seg {
                spans.push(Span::styled((*s).to_string(), *st));
                lw += dw(s);
            }
        }
        spans.push(Span::styled(pad(lw, left_w), border));
        spans.push(Span::styled("│ ", border));
        // Right column (info), padded to fill the box. Inner cols consumed so
        // far: leading space (1) + left_w + divider (1) + space (1) = 3 + left_w.
        let (itxt, ist) = info.get(row).copied().unwrap_or(("", text));
        let used = 3 + left_w + dw(itxt);
        spans.push(Span::styled(itxt.to_string(), ist));
        spans.push(Span::styled(pad(used, inner), border));
        spans.push(Span::styled("│", border));
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(vec![
        Span::styled("╰", border),
        Span::styled("─".repeat(inner), border),
        Span::styled("╯", border),
    ]));
    lines.push(Line::from(""));
    lines
}

// ── Message rendering ────────────────────────────────────────────────────────

/// Render a single chat message into styled lines. Used both to commit a
/// finished message into the terminal scrollback and to draw the live block.
pub fn message_lines(msg: &ChatMessage, _width: u16) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    match msg {
        ChatMessage::User(text) => {
            out.push(Line::from(""));
            out.push(Line::from(vec![Span::styled(
                "  You ",
                Style::default().fg(Color::Black).bg(USER_COLOR).add_modifier(Modifier::BOLD),
            )]));
            for line in text.lines() {
                out.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(line.to_string(), Style::default().fg(USER_COLOR)),
                ]));
            }
        }
        ChatMessage::AssistantText(text) => {
            out.push(Line::from(""));
            for line in markdown::markdown_to_lines(text) {
                let mut prefixed: Vec<Span<'static>> = vec![Span::raw("  ")];
                prefixed.extend(line.spans);
                out.push(Line::from(prefixed));
            }
        }
        ChatMessage::AssistantLive(text) => {
            out.push(Line::from(""));
            let md_lines = markdown::markdown_to_lines(text);
            let n = md_lines.len();
            for (i, line) in md_lines.into_iter().enumerate() {
                let mut prefixed: Vec<Span<'static>> = vec![Span::raw("  ")];
                prefixed.extend(line.spans);
                if i + 1 == n {
                    prefixed.push(Span::styled(
                        "\u{258a}",
                        Style::default().fg(Color::Magenta).add_modifier(Modifier::SLOW_BLINK),
                    ));
                }
                out.push(Line::from(prefixed));
            }
        }
        ChatMessage::ToolCall { name, input_summary, success, output } => {
            let (status_color, status) = match success {
                Some(true) => (TOOL_SUCCESS, "\u{2713}"),
                Some(false) => (TOOL_FAIL, "\u{2717}"),
                None => (TOOL_COLOR, "\u{25cf}"),
            };
            out.push(Line::from(vec![
                Span::styled("  \u{2022} ", Style::default().fg(DIM)),
                Span::styled(name.clone(), Style::default().fg(TOOL_COLOR).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {input_summary}"), Style::default().fg(DIM)),
                Span::raw("  "),
                Span::styled(status.to_string(), Style::default().fg(status_color)),
            ]));
            if matches!(success, Some(false)) {
                if let Some(out_text) = output {
                    for line in out_text.lines().take(20) {
                        out.push(Line::from(vec![Span::styled(
                            format!("     {line}"),
                            Style::default().fg(DIM),
                        )]));
                    }
                }
            }
        }
        ChatMessage::System(text) => {
            for line in text.lines() {
                out.push(Line::from(vec![Span::styled(
                    format!("  {line}"),
                    Style::default().fg(SYSTEM_COLOR).add_modifier(Modifier::ITALIC),
                )]));
            }
        }
        ChatMessage::Error(text) => {
            out.push(Line::from(vec![Span::styled(
                format!("  Error: {text}"),
                Style::default().fg(ERROR_COLOR).add_modifier(Modifier::BOLD),
            )]));
        }
    }
    out
}

/// Number of terminal rows `lines` occupy when wrapped at `width`.
pub fn wrapped_height(lines: &[Line], width: u16) -> u16 {
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
    total.max(1)
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
            "Type your prompt or /help for commands... (Shift+Enter for newline)",
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
    if !matches!(app.mode, AppMode::Settings(_) | AppMode::Help | AppMode::ModelSelect(_) | AppMode::ProviderSelect(_) | AppMode::KeySelect(_) | AppMode::KeyManage(_)) {
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
    let commands = " /help  /model  /settings  /clear  /import │ ctrl + c ×2 = quit/exit ";

    // Session token usage display.
    let token_info = if app.session_in > 0 || app.session_out > 0 {
        format!(
            "↑{} ↓{} tok | turn ↑{} ↓{}",
            fmt_k(app.session_in),
            fmt_k(app.session_out),
            fmt_k(app.last_turn_in),
            fmt_k(app.last_turn_out),
        )
    } else {
        "ready".to_string()
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
            Style::default().fg(Color::Cyan).bg(STATUS_BG),
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

        let is_api_key_field = (field.key.ends_with("_api_key") || field.key == "google_api_key")
            && !field.key.starts_with("manage_keys");
        let value_display = if field.key == "provider" {
            format!("{}  ◀▶", field.value)
        } else if field.key.starts_with("manage_keys") {
            format!("{}  →", field.value)
        } else if is_api_key_field && !field.value.is_empty() && !(state.editing && is_selected) {
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
        Line::from(vec![Span::styled("  Mouse wheel       ", cmd), Span::styled("Scroll chat", desc)]),
        Line::from(vec![Span::styled("  Drag scrollbar    ", cmd), Span::styled("Click/drag the right-edge bar to scrub", desc)]),
        Line::from(vec![Span::styled("  Click-drag        ", cmd), Span::styled("Select text (2×=word, 3×=line)", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Shift+C      ", cmd), Span::styled("Copy selection to clipboard", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Shift+V      ", cmd), Span::styled("Paste (terminal native)", desc)]),
        Line::from(vec![Span::styled("  Ctrl+Home         ", cmd), Span::styled("Jump to top of chat", desc)]),
        Line::from(vec![Span::styled("  Ctrl+End          ", cmd), Span::styled("Jump to bottom of chat", desc)]),
        Line::from(vec![Span::styled("  Tab               ", cmd), Span::styled("Auto-complete slash commands", desc)]),
        Line::from(vec![Span::styled("  Esc               ", cmd), Span::styled("Close overlay / clear input", desc)]),
        Line::from(""),
        Line::from(vec![Span::styled("  While Processing", section)]),
        Line::from(""),
        Line::from(vec![Span::styled("  Enter             ", cmd), Span::styled("Queue additional instructions", desc)]),
        Line::from(vec![Span::styled("  Shift/Alt+Enter   ", cmd), Span::styled("Insert newline in queued message", desc)]),
        Line::from(vec![Span::styled("  Esc               ", cmd), Span::styled("Interrupt the agent (abort stream + clear queue)", desc)]),
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

// ── Key select overlay ───────────────────────────────────────────────────────

fn render_key_select_overlay(f: &mut Frame, state: KeySelectState, area: Rect) {
    let width = 52u16.min(area.width.saturating_sub(4));
    let height = (state.entries.len() as u16 + 6)
        .min(area.height.saturating_sub(4))
        .max(8);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let title = format!(" 🔑  Select Key — {} ", state.provider);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" ↑↓", Style::default().fg(Color::Yellow)),
            Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("↵", Style::default().fg(Color::Yellow)),
            Span::styled("select  ", Style::default().fg(Color::DarkGray)),
            Span::styled("esc", Style::default().fg(Color::Yellow)),
            Span::styled("=cancel ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    if state.entries.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No keys registered. Add keys in Settings → Manage Keys.",
            Style::default().fg(Color::DarkGray),
        )))
        .block(block)
        .style(Style::default().bg(POPUP_BG));
        f.render_widget(empty, popup_area);
        return;
    }

    let items: Vec<ListItem> = state.entries.iter().enumerate().map(|(i, entry)| {
        let masked = format!("••••••••  ({})", entry.label);
        if i == state.selected {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("► {masked}"),
                    Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD),
                ),
            ]))
        } else {
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {masked}"), Style::default().fg(Color::Rgb(180, 180, 220))),
            ]))
        }
    }).collect();

    let list = List::new(items).block(block).style(Style::default().bg(POPUP_BG));
    f.render_widget(list, popup_area);
}

// ── Key manage overlay ───────────────────────────────────────────────────────

fn render_key_manage_overlay(f: &mut Frame, state: KeyManageState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let content_rows = (state.entries.len() as u16 + 2).max(4);
    let height = (content_rows + 8).min(area.height.saturating_sub(4)).max(10);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let (title, hint_bottom) = match &state.add_step {
        None => (
            format!(" 🗝  Keys — {} ", state.provider),
            Line::from(vec![
                Span::styled(" n", Style::default().fg(Color::Yellow)),
                Span::styled("=add  ", Style::default().fg(Color::DarkGray)),
                Span::styled("d", Style::default().fg(Color::Yellow)),
                Span::styled("=delete  ", Style::default().fg(Color::DarkGray)),
                Span::styled("↵", Style::default().fg(Color::Yellow)),
                Span::styled("=activate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("esc", Style::default().fg(Color::Yellow)),
                Span::styled("=back ", Style::default().fg(Color::DarkGray)),
            ]),
        ),
        Some(AddKeyStep::EnterLabel(_)) => (
            format!(" 🗝  Add Key — {} ", state.provider),
            Line::from(vec![
                Span::styled(" Enter label for this key (e.g. work, personal)", Style::default().fg(Color::DarkGray)),
            ]),
        ),
        Some(AddKeyStep::EnterKey { .. }) => (
            format!(" 🗝  Add Key — {} ", state.provider),
            Line::from(vec![
                Span::styled(" Paste or type the API key value", Style::default().fg(Color::DarkGray)),
            ]),
        ),
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(hint_bottom)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    match &state.add_step {
        None => {
            if state.entries.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No keys registered. Press 'n' to add one.",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for (i, entry) in state.entries.iter().enumerate() {
                    let cursor = if i == state.selected { "► " } else { "  " };
                    let style = if i == state.selected {
                        Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(180, 180, 220))
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{}[{:<20}]  ••••••••", cursor, entry.label),
                        style,
                    )));
                }
            }
        }
        Some(AddKeyStep::EnterLabel(buf)) => {
            lines.push(Line::from(vec![
                Span::styled("  Label: ", Style::default().fg(Color::Rgb(130, 180, 255))),
                Span::styled(
                    format!("{buf}_"),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
                ),
            ]));
        }
        Some(AddKeyStep::EnterKey { label, key_buf }) => {
            lines.push(Line::from(vec![
                Span::styled(format!("  Label: {label}"), Style::default().fg(Color::Rgb(100, 220, 130))),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Key:   ", Style::default().fg(Color::Rgb(130, 180, 255))),
                Span::styled(
                    format!("{}_", "•".repeat(key_buf.len())),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
                ),
            ]));
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(POPUP_BG));
    f.render_widget(para, inner);
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
