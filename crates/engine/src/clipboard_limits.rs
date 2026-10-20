//! Allocation bounds checked before the existing clipboard HTML parser runs.

pub const MAX_HTML_BYTES: usize = 8 * 1024 * 1024;
const MAX_CELLS: usize = 20_000;
const MAX_DEPTH: usize = 64;

/// Rejects markup whose table spans or nesting could exceed clipboard limits.
pub fn accepts_html(html: &str) -> bool {
    if html.len() > MAX_HTML_BYTES {
        return false;
    }
    let mut rest = html;
    let mut cells = 0usize;
    let mut rows = 0usize;
    let mut open: Vec<String> = Vec::new();
    while let Some(start) = rest.find('<') {
        rest = rest.get(start..).unwrap_or_default();
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").and_then(|i| comment.get(i + 3..)).unwrap_or_default();
            continue;
        }
        let Some(end) = tag_end(rest) else { break };
        let tag = rest.get(1..end).unwrap_or_default();
        rest = rest.get(end + 1..).unwrap_or_default();
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/');
        let name_end = body.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(body.len());
        let name = body.get(..name_end).unwrap_or_default().to_ascii_lowercase();
        if name.is_empty() || name.starts_with(['!', '?']) {
            continue;
        }
        if closing {
            if let Some(i) = open.iter().rposition(|n| n == &name) {
                open.truncate(i);
            }
            continue;
        }
        if name == "tr" {
            rows = rows.saturating_add(1);
        }
        if matches!(name.as_str(), "td" | "th") {
            let span = colspan(body.get(name_end..).unwrap_or_default());
            cells = cells.saturating_add(span);
        }
        if rows > MAX_CELLS || cells > MAX_CELLS {
            return false;
        }
        if !tag.ends_with('/') && !is_void(&name) {
            if open.len() >= MAX_DEPTH {
                return false;
            }
            open.push(name);
        }
    }
    true
}

fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (i, ch) in tag.char_indices() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn is_void(name: &str) -> bool {
    matches!(name, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr")
}

fn colspan(mut attrs: &str) -> usize {
    while !attrs.is_empty() {
        attrs = attrs.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let end = attrs.find(|c: char| c.is_whitespace() || c == '=' || c == '/').unwrap_or(attrs.len());
        let key = attrs.get(..end).unwrap_or_default();
        attrs = attrs.get(end..).unwrap_or_default().trim_start();
        if let Some(value) = attrs.strip_prefix('=') {
            let value = value.trim_start();
            let (text, tail) = if let Some(q @ ('\'' | '"')) = value.chars().next() {
                let inner = value.get(1..).unwrap_or_default();
                let end = inner.find(q).unwrap_or(inner.len());
                (inner.get(..end).unwrap_or_default(), inner.get(end + 1..).unwrap_or_default())
            } else {
                let end = value.find(char::is_whitespace).unwrap_or(value.len());
                (value.get(..end).unwrap_or_default(), value.get(end..).unwrap_or_default())
            };
            if key.eq_ignore_ascii_case("colspan") {
                return text.trim().parse::<usize>().unwrap_or(1).clamp(1, gridcraft_core::MAX_COLS as usize);
            }
            attrs = tail;
        } else {
            if key.eq_ignore_ascii_case("colspan") {
                return 1;
            }
            if key.is_empty() {
                let mut chars = attrs.chars();
                chars.next();
                attrs = chars.as_str();
            }
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_cell_and_row_limits_are_checked_before_parsing() {
        let cells = "<td>x</td>".repeat(MAX_CELLS);
        assert!(accepts_html(&format!("<table><tr>{cells}</tr></table>")));
        assert!(!accepts_html(&format!("<table><tr>{cells}<td>x</td></tr></table>")));
        assert!(!accepts_html(&"<tr></tr>".repeat(MAX_CELLS + 1)));
    }

    #[test]
    fn html_spans_count_towards_the_cell_budget() {
        assert!(accepts_html("<table><tr><td colspan='2'>merged</td></tr></table>"));
        for attrs in ["colspan=16384", "COLSPAN = '16384'", "class='colspan=1' colspan=16384", "title='>' colspan=16384", "colspan=999999999"] {
            assert!(!accepts_html(&format!("<table><tr><td {attrs}>one</td></tr><tr><td {attrs}>two</td></tr></table>")), "{attrs}");
        }
        assert!(accepts_html("<table><tr><td colspan='1' colspan='999999'>first wins</td></tr></table>"));
        assert!(accepts_html("<table><tr><td colspan='bad'>one</td></tr></table>"));
    }

    #[test]
    fn html_nesting_and_byte_limits_are_inclusive() {
        assert!(accepts_html(&format!("{}text{}", "<div>".repeat(MAX_DEPTH), "</div>".repeat(MAX_DEPTH))));
        assert!(!accepts_html(&"<div>".repeat(MAX_DEPTH + 1)));
        assert!(accepts_html(&"x".repeat(MAX_HTML_BYTES)));
        assert!(!accepts_html(&"x".repeat(MAX_HTML_BYTES + 1)));
        assert!(accepts_html(&format!("<!--{}-->{}", "<div>".repeat(MAX_DEPTH + 1), "<br>".repeat(MAX_DEPTH + 1))));
    }
}
