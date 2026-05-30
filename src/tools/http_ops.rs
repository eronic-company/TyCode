use serde_json::Value;
use std::io::copy;

use super::ToolResult;

/// Make an HTTP request.
pub fn http_request(method: &str, url: &str, headers: Option<&Value>, body: &str) -> ToolResult {
    if url.is_empty() {
        return ToolResult::err("No URL provided");
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to create HTTP client: {e}")),
    };

    let method_upper = method.to_uppercase();
    let http_method = match method_upper.as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        "PATCH" => reqwest::Method::PATCH,
        "HEAD" => reqwest::Method::HEAD,
        "OPTIONS" => reqwest::Method::OPTIONS,
        _ => return ToolResult::err(format!("Unsupported HTTP method: {method}")),
    };

    let mut request = client.request(http_method, url);

    // Add headers
    if let Some(hdrs) = headers {
        if let Some(obj) = hdrs.as_object() {
            for (key, val) in obj {
                if let Some(v) = val.as_str() {
                    request = request.header(key, v);
                }
            }
        }
    }

    // Add body
    if !body.is_empty() {
        request = request.body(body.to_string());
    }

    match request.send() {
        Ok(response) => {
            let status = response.status();
            let headers_map: Vec<String> = response
                .headers()
                .iter()
                .take(20)
                .map(|(k, v)| format!("  {}: {}", k, v.to_str().unwrap_or("?")))
                .collect();

            let body_text = response.text().unwrap_or_default();
            let truncated = body_text.len() > 8192;
            let body_display = if truncated {
                format!("{}...\n(truncated at 8KB)", &body_text[..8192])
            } else {
                body_text
            };

            let output = format!(
                "HTTP {} {}\nHeaders:\n{}\n\nBody:\n{}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                headers_map.join("\n"),
                body_display
            );

            if status.is_success() {
                ToolResult::ok(output)
            } else {
                ToolResult { success: false, output }
            }
        }
        Err(e) => ToolResult::err(format!("HTTP request failed: {e}")),
    }
}

/// Fetch a web page and return its readable text content with HTML stripped —
/// the equivalent of Claude Code's WebFetch. Scripts, styles, and tags are
/// removed and whitespace collapsed so the model sees prose, not markup.
pub fn web_fetch(url: &str) -> ToolResult {
    if url.is_empty() {
        return ToolResult::err("No URL provided");
    }
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("TyCode/0.1 (+https://github.com/AlphaGlider25/TyCode)")
        .build()
    {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to create HTTP client: {e}")),
    };

    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("Fetch failed: {e}")),
    };

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let raw = response.text().unwrap_or_default();

    let text = if content_type.contains("html") || raw.trim_start().starts_with('<') {
        html_to_text(&raw)
    } else {
        raw
    };

    let truncated = text.len() > 16384;
    let body = if truncated {
        format!("{}\n...\n(truncated at 16KB)", &text[..16384])
    } else {
        text
    };

    let out = format!("URL: {url}\nHTTP {}\n\n{body}", status.as_u16());
    if status.is_success() {
        ToolResult::ok(out)
    } else {
        ToolResult { success: false, output: out }
    }
}

/// Crude but dependency-free HTML → text: drop script/style blocks, strip tags,
/// decode the most common entities, and collapse runaway whitespace.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;

    while i < bytes.len() {
        if let Some(close) = skip_until {
            if lower[i..].starts_with(close) {
                i += close.len();
                skip_until = None;
            } else {
                i += 1;
            }
            continue;
        }
        let c = bytes[i] as char;
        if c == '<' {
            if lower[i..].starts_with("<script") {
                skip_until = Some("</script>");
                i += 7;
                continue;
            }
            if lower[i..].starts_with("<style") {
                skip_until = Some("</style>");
                i += 6;
                continue;
            }
            // Block-ish tags become line breaks for readability.
            if lower[i..].starts_with("<br")
                || lower[i..].starts_with("</p")
                || lower[i..].starts_with("</div")
                || lower[i..].starts_with("</li")
                || lower[i..].starts_with("</h")
            {
                out.push('\n');
            }
            in_tag = true;
            i += 1;
            continue;
        }
        if c == '>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if !in_tag {
            out.push(c);
        }
        i += 1;
    }

    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");

    // Collapse 3+ newlines and trailing spaces.
    let mut result = String::with_capacity(decoded.len());
    let mut blank = 0;
    for line in decoded.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank += 1;
            if blank <= 1 {
                result.push('\n');
            }
        } else {
            blank = 0;
            result.push_str(trimmed);
            result.push('\n');
        }
    }
    result.trim().to_string()
}

/// Download a file from URL to disk.
pub fn http_download(url: &str, output_path: &str) -> ToolResult {
    if url.is_empty() || output_path.is_empty() {
        return ToolResult::err("URL and output_path are required");
    }

    let expanded = super::shellexpand(output_path);
    let path = std::path::Path::new(&expanded);

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return ToolResult::err(format!("Failed to create directories: {e}"));
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Failed to create HTTP client: {e}")),
    };

    match client.get(url).send() {
        Ok(mut response) => {
            if !response.status().is_success() {
                return ToolResult::err(format!("Download failed: HTTP {}", response.status()));
            }

            match std::fs::File::create(path) {
                Ok(mut file) => match copy(&mut response, &mut file) {
                    Ok(bytes_written) => {
                        ToolResult::ok(format!("Downloaded {bytes_written} bytes to {output_path}"))
                    }
                    Err(e) => ToolResult::err(format!("Failed to write file: {e}")),
                },
                Err(e) => ToolResult::err(format!("Failed to create file: {e}")),
            }
        }
        Err(e) => ToolResult::err(format!("Download failed: {e}")),
    }
}

