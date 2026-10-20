//! The deliberately small CSS vocabulary shared by HTML clipboard import and export.

use std::fmt::Write as _;

use gridcraft_model::{BorderLine, BorderStyle, Color, Fill, HAlign, NumFmt, Style, Theme, Underline, VAlign};

pub(crate) const MAX_STYLE: usize = 8 * 1024;

#[derive(Default)]
pub(crate) struct Rules(Vec<(String, String)>);

impl Rules {
    pub fn add(&mut self, css: &str) {
        // Bound selector matching work as well as the size of each declaration block.
        for block in css.split('}').take(256usize.saturating_sub(self.0.len())) {
            let Some((selectors, body)) = block.split_once('{') else { continue };
            if body.len() > MAX_STYLE || unsafe_css(body) {
                continue;
            }
            for selector in selectors.split(',') {
                let selector = selector.trim();
                let name = selector.strip_prefix('.').unwrap_or(selector);
                if !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && self.0.len() < 256 {
                    self.0.push((selector.to_string(), body.to_string()));
                }
            }
        }
    }

    pub fn apply(&self, tag: &str, classes: &str, style: &mut Style, hidden: &mut bool, preserve: &mut bool) {
        // Class specificity wins over tag selectors, independent of source order.
        for class_rule in [false, true] {
            for (selector, body) in &self.0 {
                let matches = match selector.strip_prefix('.') {
                    Some(class) => class_rule && classes.split_whitespace().any(|c| c == class),
                    None => !class_rule && selector.eq_ignore_ascii_case(tag),
                };
                if matches {
                    apply(body, style, hidden, preserve);
                }
            }
        }
    }
}

fn unsafe_css(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    lower.contains("url(") || lower.contains('@') && lower.contains("import") || lower.contains("/*") || lower.contains("expression(")
}

pub(crate) fn apply(css: &str, style: &mut Style, hidden: &mut bool, preserve: &mut bool) {
    if css.len() > MAX_STYLE || unsafe_css(css) {
        return;
    }
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;
    for (i, ch) in css.char_indices().chain(std::iter::once((css.len(), ';'))) {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, ';') => {
                if let Some((key, value)) = css.get(start..i).and_then(|s| s.split_once(':')) {
                    property(key.trim(), value.trim(), style, hidden, preserve);
                }
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
}

fn property(key: &str, value: &str, s: &mut Style, hidden: &mut bool, preserve: &mut bool) {
    let key = key.to_ascii_lowercase();
    let lower = value.to_ascii_lowercase();
    match key.as_str() {
        "display" => *hidden = lower == "none",
        "font-weight" => s.font.bold = lower == "bold" || lower.parse::<u16>().is_ok_and(|n| n >= 600),
        "font-style" => s.font.italic = matches!(lower.as_str(), "italic" | "oblique"),
        "text-decoration" | "text-decoration-line" => {
            s.font.underline = if lower.split_whitespace().any(|v| v == "underline") { Underline::Single } else { Underline::None };
            s.font.strike = lower.split_whitespace().any(|v| v == "line-through");
        }
        "font-family" => {
            let name = first_family(value);
            if !name.is_empty() && name.len() <= 256 {
                s.font.name = name;
            }
        }
        "font-size" => {
            if let Some(points) = length(&lower).filter(|n| *n > 0.0 && *n <= 4096.0) {
                s.font.size = points;
            }
        }
        "color" => {
            if let Some(c) = color(value) {
                s.font.color = c;
            }
        }
        "background" | "background-color" => {
            if lower == "transparent" {
                s.fill = Fill::default();
            } else if let Some(c) = color(value) {
                s.fill = Fill::solid(c);
            }
        }
        "text-align" => {
            s.align.h = match lower.as_str() {
                "left" => HAlign::Left,
                "center" => HAlign::Center,
                "right" => HAlign::Right,
                "justify" => HAlign::Justify,
                _ => HAlign::General,
            }
        }
        "mso-text-align" if lower == "general" => s.align.h = HAlign::General,
        "text-decoration-style" if lower == "double" && s.font.underline != Underline::None => s.font.underline = Underline::Double,
        "vertical-align" => {
            s.align.v = match lower.as_str() {
                "top" => VAlign::Top,
                "middle" => VAlign::Center,
                _ => VAlign::Bottom,
            }
        }
        "white-space" => {
            s.align.wrap = matches!(lower.as_str(), "normal" | "pre-wrap" | "pre-line" | "break-spaces");
            *preserve = matches!(lower.as_str(), "pre" | "pre-wrap" | "break-spaces");
        }
        "mso-number-format" => {
            if let Some(code) = number_format(value) {
                s.num_fmt = NumFmt::new(&code);
            }
        }
        _ if key == "border" || key.starts_with("border-") => border_property(&key, value, s),
        _ => {}
    }
}

fn length(value: &str) -> Option<f32> {
    let (number, scale) = if let Some(n) = value.strip_suffix("pt") {
        (n, 1.0)
    } else if let Some(n) = value.strip_suffix("px") {
        (n, 0.75)
    } else {
        (value, 0.75)
    };
    number.trim().parse::<f32>().ok().filter(|n| n.is_finite()).map(|n| n * scale)
}

fn first_family(value: &str) -> String {
    let mut quote = None;
    let mut escaped = false;
    let mut end = value.len();
    for (i, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match (quote, ch) {
            (Some(q), c) if q == c => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(ch),
            (None, ',') => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let name = value.get(..end).unwrap_or_default().trim();
    let name = match name.chars().next() {
        Some(q @ ('\'' | '"')) if name.len() >= 2 && name.ends_with(q) => name.get(1..name.len() - 1).unwrap_or_default(),
        _ => name,
    };
    unescape(name)
}

fn color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 3 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Color::from_hex(&hex.chars().flat_map(|c| [c, c]).collect::<String>());
        }
        return (hex.len() == 6).then(|| Color::from_hex(hex)).flatten();
    }
    if let Some(rgb) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<_> = rgb.split(',').map(|s| s.trim().parse::<u8>().ok()).collect();
        if let [Some(r), Some(g), Some(b)] = parts.as_slice() {
            return Some(Color::rgb(*r, *g, *b));
        }
    }
    Some(Color::Rgb(match value.as_str() {
        "black" => 0,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "yellow" => 0xffff00,
        "gray" | "grey" => 0x808080,
        "silver" => 0xc0c0c0,
        "navy" => 0x000080,
        "maroon" => 0x800000,
        "purple" => 0x800080,
        "teal" => 0x008080,
        "lime" => 0x00ff00,
        "aqua" => 0x00ffff,
        "fuchsia" => 0xff00ff,
        "olive" => 0x808000,
        _ => return None,
    }))
}

fn border_property(key: &str, value: &str, s: &mut Style) {
    let suffix = key.strip_prefix("border-").unwrap_or_default();
    let (side, part) = suffix.split_once('-').unwrap_or((suffix, ""));
    let part = if matches!(side, "width" | "style" | "color") { side } else { part };
    for (name, line) in
        [("left", &mut s.borders.left), ("right", &mut s.borders.right), ("top", &mut s.borders.top), ("bottom", &mut s.borders.bottom)]
    {
        if matches!(side, "left" | "right" | "top" | "bottom") && name != side {
            continue;
        }
        if !matches!(part, "" | "width" | "style" | "color") {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        if part == "color" {
            if let Some(c) = color(value) {
                line.color = c;
            }
            continue;
        }
        if part.is_empty() {
            *line = BorderLine { style: BorderStyle::Thin, color: Color::Auto };
        }
        let width = lower.split_whitespace().find_map(length).or_else(|| (part == "style").then_some(line.style.width() * 0.75));
        let tokens: Vec<_> = lower.split_whitespace().collect();
        let kind = tokens.iter().find(|v| matches!(**v, "none" | "hidden" | "solid" | "double" | "dotted" | "dashed"));
        line.style = match kind.copied() {
            Some("none" | "hidden") => BorderStyle::None,
            Some("double") => BorderStyle::Double,
            Some("dotted") => BorderStyle::Dotted,
            Some("dashed") if width.is_some_and(|w| w > 1.0) => BorderStyle::MediumDashed,
            Some("dashed") => BorderStyle::Dashed,
            _ if width == Some(0.0) => BorderStyle::None,
            _ if width.is_some_and(|w| w >= 2.0) => BorderStyle::Thick,
            _ if width.is_some_and(|w| w > 1.0) => BorderStyle::Medium,
            _ if kind == Some(&"solid") || part == "width" => BorderStyle::Thin,
            _ => line.style,
        };
        if part.is_empty() {
            if let Some(c) = color(value).or_else(|| tokens.iter().rev().find_map(|v| color(v))) {
                line.color = c;
            }
            if let Some(pos) = lower.find("rgb(").and_then(|p| value.get(p..))
                && let Some(c) = color(pos)
            {
                line.color = c;
            }
        }
    }
}

pub(crate) fn number_format(value: &str) -> Option<String> {
    let value = value.trim();
    let value = match value.chars().next() {
        Some(q @ ('\'' | '"')) if value.len() >= 2 && value.ends_with(q) => value.get(1..value.len() - 1)?,
        _ => value,
    };
    let value = unescape(value);
    if value.len() > 255 || value.is_empty() {
        return None;
    }
    if value.eq_ignore_ascii_case("general") {
        return Some("General".into());
    }
    if value.eq_ignore_ascii_case("short date") {
        return Some("m/d/yyyy".into());
    }
    for id in 0..=49 {
        if let Some(code) = gridcraft_numfmt::builtin_format(id)
            && value == code
        {
            return Some(value);
        }
    }
    let mut quoted = false;
    for ch in value.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted && !"0#?.,%;+-()/ :ymdhsYMDHSeE@".contains(ch) {
            return None;
        }
    }
    (!quoted).then_some(value)
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            if let Some(c) = chars.next() {
                hex.push(c);
            }
        }
        if hex.is_empty() {
            if let Some(c) = chars.next() {
                out.push(c);
            }
        } else {
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push(c);
            }
            if chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
        }
    }
    out
}

pub(crate) fn to_css(s: &Style, theme: &Theme) -> String {
    let mut css = String::new();
    if s.font.bold {
        css.push_str("font-weight:bold;");
    }
    if s.font.italic {
        css.push_str("font-style:italic;");
    }
    let underline = s.font.underline != Underline::None;
    if underline || s.font.strike {
        let _ = write!(css, "text-decoration:{}{};", if underline { "underline " } else { "" }, if s.font.strike { "line-through" } else { "" });
    }
    if matches!(s.font.underline, Underline::Double | Underline::DoubleAccounting) {
        css.push_str("text-decoration-style:double;");
    }
    let name = s.font.name.replace('\\', "\\\\").replace('\'', "\\'").replace(['\r', '\n'], " ");
    let _ = write!(css, "font-family:'{name}';font-size:{}pt;", s.font.size);
    if let Some(c) = s.font.color.hex(theme) {
        let _ = write!(css, "color:{c};");
    }
    if s.fill.pattern != gridcraft_model::PatternType::None
        && let Some(c) = s.fill.fg.hex(theme)
    {
        let _ = write!(css, "background:{c};");
    }
    let h = match s.align.h {
        HAlign::Left => "left",
        HAlign::Right => "right",
        HAlign::Center | HAlign::CenterAcross => "center",
        HAlign::Justify | HAlign::Distributed => "justify",
        _ => "start",
    };
    let v = match s.align.v {
        VAlign::Top => "top",
        VAlign::Center => "middle",
        _ => "bottom",
    };
    let _ = write!(css, "text-align:{h};vertical-align:{v};white-space:{};", if s.align.wrap { "pre-wrap" } else { "pre" });
    let code = s.num_fmt.as_str().replace('\\', "\\\\").replace('"', "\\000022");
    let _ = write!(css, "mso-number-format:\"{code}\";");
    for (side, border) in [("left", s.borders.left), ("right", s.borders.right), ("top", s.borders.top), ("bottom", s.borders.bottom)] {
        if border.is_none() {
            let _ = write!(css, "border-{side}:none;");
            continue;
        }
        let kind = match border.style {
            BorderStyle::Double => "double",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Dashed
            | BorderStyle::MediumDashed
            | BorderStyle::DashDot
            | BorderStyle::MediumDashDot
            | BorderStyle::DashDotDot
            | BorderStyle::MediumDashDotDot
            | BorderStyle::SlantDashDot => "dashed",
            _ => "solid",
        };
        let c = border.color.hex(theme).unwrap_or_else(|| "currentColor".into());
        let _ = write!(css, "border-{side}:{}px {kind} {c};", border.style.width());
    }
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_specificity_and_inline_overrides_are_deterministic() {
        let mut rules = Rules::default();
        rules.add(".a {color:blue} td {color:red} .a {font-weight:bold} div td {font-style:italic}");
        let mut style = Style::default();
        let (mut hidden, mut preserve) = (false, false);
        rules.apply("td", "other a", &mut style, &mut hidden, &mut preserve);
        assert_eq!(style.font.color, Color::Rgb(0x0000ff));
        assert!(style.font.bold);
        assert!(!style.font.italic);
        apply("color:#abc;font-family:'A, B',serif;white-space:pre", &mut style, &mut hidden, &mut preserve);
        assert_eq!(style.font.color, Color::Rgb(0xaabbcc));
        assert_eq!(style.font.name, "A, B");
        assert!(preserve);
        assert!(!style.align.wrap);
    }

    #[test]
    fn number_format_hints_only_accept_supported_codes() {
        for (hint, expected) in
            [(r#""0.00""#, "0.00"), (r#""\@""#, "@"), ("Short Date", "m/d/yyyy"), (r#""\000022Rp\000022#,##0.00""#, "\"Rp\"#,##0.00")]
        {
            assert_eq!(number_format(hint).as_deref(), Some(expected));
        }
        for hint in ["unknown-format", "[$unsupported]0", "url(bad)", "\"unclosed"] {
            assert_eq!(number_format(hint), None);
        }
        assert!(number_format(&"0".repeat(256)).is_none());
    }

    #[test]
    fn css_round_trip_keeps_supported_border_and_font_styles() {
        let mut style = Style::default();
        let (mut hidden, mut preserve) = (false, false);
        apply(
            "border:1px solid red;border-left:2px dashed blue;border-bottom:3px double #123456;font-family:'O\\'Brien, Sans';text-decoration:underline line-through;text-decoration-style:double;mso-number-format:'0.00'",
            &mut style,
            &mut hidden,
            &mut preserve,
        );
        let css = to_css(&style, &Theme::default());
        let mut back = Style::default();
        apply(&css, &mut back, &mut hidden, &mut preserve);
        assert_eq!(back, style);
    }
}
