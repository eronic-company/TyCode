use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Convert markdown text to styled ratatui Lines.
/// Supports: bold, italic, inline code, code blocks, headers, lists.
pub fn markdown_to_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut code_lang = String::new();

    for raw_line in text.lines() {
        if raw_line.starts_with("```") {
            if in_code_block {
                // End code block
                in_code_block = false;
                // Render accumulated code block
                if !code_block_lines.is_empty() {
                    let header = if code_lang.is_empty() {
                        " code ".to_string()
                    } else {
                        format!(" {} ", code_lang)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  ╭─{header}{}╮", "─".repeat(60usize.saturating_sub(header.len() + 4))),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                    for cl in &code_block_lines {
                        lines.push(Line::from(vec![
                            Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                cl.clone(),
                                Style::default().fg(Color::Green),
                            ),
                        ]));
                    }
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  ╰{}╯", "─".repeat(60usize.saturating_sub(2))),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                code_block_lines.clear();
                code_lang.clear();
            } else {
                // Start code block
                in_code_block = true;
                code_lang = raw_line.trim_start_matches('`').trim().to_string();
            }
            continue;
        }

        if in_code_block {
            code_block_lines.push(raw_line.to_string());
            continue;
        }

        // Headers
        if raw_line.starts_with("### ") {
            lines.push(Line::from(vec![Span::styled(
                raw_line[4..].to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )]));
            continue;
        }
        if raw_line.starts_with("## ") {
            lines.push(Line::from(vec![Span::styled(
                raw_line[3..].to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]));
            continue;
        }
        if raw_line.starts_with("# ") {
            lines.push(Line::from(vec![Span::styled(
                raw_line[2..].to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )]));
            continue;
        }

        // Horizontal rule
        if raw_line.trim() == "---" || raw_line.trim() == "***" || raw_line.trim() == "___" {
            lines.push(Line::from(vec![Span::styled(
                "─".repeat(60),
                Style::default().fg(Color::DarkGray),
            )]));
            continue;
        }

        // List items
        if raw_line.starts_with("- ") || raw_line.starts_with("* ") {
            let content = &raw_line[2..];
            let mut spans = vec![Span::styled(
                "  • ".to_string(),
                Style::default().fg(Color::Cyan),
            )];
            spans.extend(parse_inline(content));
            lines.push(Line::from(spans));
            continue;
        }

        // Numbered list
        if let Some(rest) = try_strip_numbered_list(raw_line) {
            let prefix_len = raw_line.len() - rest.len();
            let prefix = &raw_line[..prefix_len];
            let mut spans = vec![Span::styled(
                format!("  {prefix}"),
                Style::default().fg(Color::Cyan),
            )];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // Normal line with inline formatting
        if raw_line.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(parse_inline(raw_line)));
        }
    }

    // Handle unterminated code block
    if in_code_block && !code_block_lines.is_empty() {
        for cl in &code_block_lines {
            lines.push(Line::from(vec![Span::styled(
                format!("  {cl}"),
                Style::default().fg(Color::Green),
            )]));
        }
    }

    lines
}

/// Parse inline markdown formatting: **bold**, *italic*, `code`.
fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut current = String::new();

    let default_style = Style::default();
    let bold_style = Style::default().add_modifier(Modifier::BOLD);
    let italic_style = Style::default().add_modifier(Modifier::ITALIC);
    let code_style = Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 40));

    while let Some((_i, ch)) = chars.next() {
        match ch {
            '`' => {
                // Inline code
                if !current.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut current), default_style));
                }
                let mut code = String::new();
                let mut found_end = false;
                for (_, c) in chars.by_ref() {
                    if c == '`' {
                        found_end = true;
                        break;
                    }
                    code.push(c);
                }
                if found_end {
                    spans.push(Span::styled(code, code_style));
                } else {
                    current.push('`');
                    current.push_str(&code);
                }
            }
            '*' => {
                // Check for ** (bold) or * (italic)
                if chars.peek().map(|(_, c)| *c) == Some('*') {
                    chars.next(); // consume second *
                    if !current.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut current), default_style));
                    }
                    let mut bold_text = String::new();
                    let mut found_end = false;
                    while let Some((_, c)) = chars.next() {
                        if c == '*' && chars.peek().map(|(_, c)| *c) == Some('*') {
                            chars.next();
                            found_end = true;
                            break;
                        }
                        bold_text.push(c);
                    }
                    if found_end {
                        spans.push(Span::styled(bold_text, bold_style));
                    } else {
                        current.push_str("**");
                        current.push_str(&bold_text);
                    }
                } else {
                    // Italic
                    if !current.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut current), default_style));
                    }
                    let mut italic_text = String::new();
                    let mut found_end = false;
                    for (_, c) in chars.by_ref() {
                        if c == '*' {
                            found_end = true;
                            break;
                        }
                        italic_text.push(c);
                    }
                    if found_end {
                        spans.push(Span::styled(italic_text, italic_style));
                    } else {
                        current.push('*');
                        current.push_str(&italic_text);
                    }
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, default_style));
    }

    if spans.is_empty() {
        spans.push(Span::raw(""));
    }

    spans
}

fn try_strip_numbered_list(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    // Match pattern: digits followed by ". "
    let digit_end = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .count();
    if digit_end == 0 {
        return None;
    }
    let after_digits = &trimmed[digit_end..];
    after_digits.strip_prefix(". ")
}
