//! Logs never name the user's documents (File Access v3, spec §07): no log macro in the apps,
//! the UI or the engine interpolates a name, path, URL or title unless it goes through
//! `log_redact::file_label` or `log_redact::home_relative`.

use std::path::{Path, PathBuf};

const MACROS: [&str; 5] = ["log::error!(", "log::warn!(", "log::info!(", "log::debug!(", "log::trace!("];
// `p` and `f` are the usual short names for a path or a file; `doc`/`docId` name an online
// document (`erpHost`) and `bytes` its content.
const SUSPECT_WORDS: [&str; 15] =
    ["name", "path", "url", "uri", "file", "filename", "title", "target", "href", "dir", "p", "f", "doc", "docid", "bytes"];
const REDACTORS: [&str; 2] = ["file_label(", "home_relative("];

/// Arguments that look suspect but aren't a document: (source file, argument).
const ALLOWED: [(&str, &str); 4] = [
    // The app's own name.
    ("apps/gridcraft/src/main.rs", "PRODUCT_NAME"),
    // The erpAccount method that failed.
    ("apps/gridcraft-web/src/account.rs", "name"),
    // The GPU adapter's name.
    ("apps/gridcraft/src/graphics.rs", "info.name"),
    // A JS error's name, such as "AbortError".
    ("apps/gridcraft-web/src/web.rs", "err_name"),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The text inside the parentheses that open at `start` (just past the `(`), honouring strings.
fn invocation(text: &str, start: usize) -> &str {
    let bytes = text.as_bytes();
    let (mut depth, mut i, mut in_str) = (1usize, start, false);
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            match c {
                b'\\' => i += 1,
                b'"' => in_str = false,
                _ => {}
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &text[start..i];
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    &text[start..]
}

/// Top-level comma-separated arguments.
fn split_args(args: &str) -> Vec<String> {
    let (mut out, mut current, mut depth, mut in_str, mut escaped) = (Vec::new(), String::new(), 0i32, false, false);
    for c in args.chars() {
        if in_str {
            current.push(c);
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut current).trim().to_string());
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// The identifiers a format string interpolates inline (`{name}`, `{path:?}`).
fn inline_args(format: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = format.replace("{{", "").replace("}}", "");
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        let inner = &rest[open + 1..open + close];
        let ident = inner.split(':').next().unwrap_or("").trim();
        if !ident.is_empty() && !ident.chars().all(|c| c.is_ascii_digit()) {
            out.push(ident.to_string());
        }
        rest = rest[open + close + 1..].to_string();
    }
    out
}

fn suspect(arg: &str) -> bool {
    if REDACTORS.iter().any(|r| arg.contains(r)) {
        return false;
    }
    if arg.contains(".display()") {
        return true;
    }
    // A named argument is judged by its value.
    let value = arg.split_once('=').filter(|(k, v)| !k.contains('"') && !v.starts_with('=')).map_or(arg, |(_, v)| v);
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .flat_map(|token| token.split('_'))
        .any(|word| SUSPECT_WORDS.contains(&word.to_ascii_lowercase().as_str()))
}

#[test]
fn no_log_line_names_a_document() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["apps", "crates/ui-egui/src", "crates/engine/src"] {
        sources(&root.join(dir), &mut files);
    }
    let (mut found, mut offending, mut allowed_used) = (0, Vec::new(), [false; ALLOWED.len()]);
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        let rel = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        for m in MACROS {
            for (at, _) in text.match_indices(m) {
                found += 1;
                let args = split_args(invocation(&text, at + m.len()));
                let Some((format, positional)) = args.split_first() else { continue };
                let line = text[..at].lines().count() + 1;
                for arg in inline_args(format).iter().chain(positional) {
                    if !suspect(arg) {
                        continue;
                    }
                    match ALLOWED.iter().position(|(f, a)| *f == rel && a == arg) {
                        Some(i) => allowed_used[i] = true,
                        None => offending.push(format!("{rel}:{line}: {arg}")),
                    }
                }
            }
        }
    }
    assert!(found > 30, "the scan found only {found} log lines");
    assert!(offending.is_empty(), "log lines naming a document (use log_redact::file_label / home_relative):\n{}", offending.join("\n"));
    for (i, used) in allowed_used.iter().enumerate() {
        assert!(used, "allowlist entry no longer needed: {:?}", ALLOWED[i]);
    }
}

#[test]
fn the_scan_catches_names_paths_and_urls() {
    let args = split_args(invocation(r#"log::warn!("couldn't open {name}: {e}", e = err) x"#, "log::warn!(".len()));
    assert_eq!(inline_args(&args[0]), ["name", "e"]);
    assert!(suspect("name") && suspect("r.name") && suspect("path.display()") && suspect("d.display_title()") && suspect("url"));
    assert!(suspect("p") && suspect("f.path()") && suspect("source.display()"));
    assert!(suspect("doc_id") && suspect("docId") && suspect("doc.doc_id") && suspect("bytes") && !suspect("revision"));
    let args = split_args(invocation(r#"log::warn!("skipping font {p} (face {index})")"#, "log::warn!(".len()));
    assert!(inline_args(&args[0]).iter().any(|a| suspect(a)), "the old font path line is caught");
    assert!(!suspect("e") && !suspect("method") && !suspect("file_label(&name, Some(n))") && !suspect("home_relative(&path)"));
    assert_eq!(split_args(r#""a {}, {}", f(a, b), c"#), [r#""a {}, {}""#, "f(a, b)", "c"]);
}
