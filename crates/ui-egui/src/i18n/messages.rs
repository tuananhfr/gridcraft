//! Messages that embed a file name or another message (the engine's "can't open" errors) can't be
//! looked up whole: they are matched against English templates, and the text inside them is
//! translated in turn. Untranslated parts stay in English.

use std::borrow::Cow;

use super::Language;

/// Placeholders holding text that is itself a message; any other placeholder (a file name, an
/// encryption scheme, a part name) is kept as it is.
const NESTED: [&str; 2] = ["reason", "detail"];

/// The errors `gridcraft_engine::io` gives when a file can't be opened or imported, and the reader
/// reasons inside them. Each has a catalog entry; `tests/open_refused.rs` opens real broken files
/// to keep them in step with the engine.
const TEMPLATES: &[&str] = &[
    "Missing {name}",
    "DAO/AutoCAD.{member} not supported in GridCraft (no disk/network/system access)",
    "Invalid DXF drawing: {detail}",
    "We can't open '{name}': {reason}",
    "We can't import '{name}': {reason}",
    "Couldn't save '{name}': {reason}",
    "'{name}' uses {scheme} encryption, which GridCraft can't open yet. Remove the password in Excel and try again.",
    "'{name}' is a legacy binary file but not an Excel workbook (no workbook stream). GridCraft can't open it.",
    "the file is not a valid zip package: {detail}",
    "the file contains invalid XML: {detail}",
    "the file is not a valid spreadsheet: {detail}",
    "the encrypted file is damaged ({detail}).",
    "not a GridCraft JSON workbook: {detail}",
    "missing part {part}",
    "unexpected root element <{element}> in workbook part",
    "XLS: {detail}",
    "XLSB: {detail}",
];

/// Nesting is at most three deep (error → reason → detail); the bound only guards against a loop.
const MAX_DEPTH: usize = 4;

pub(super) fn translate(lang: Language, text: &str) -> Cow<'_, str> {
    translate_at(lang, text, 0)
}

fn translate_at(lang: Language, text: &str, depth: usize) -> Cow<'_, str> {
    let whole = lang.tr(text);
    if whole != text || lang == Language::En || depth > MAX_DEPTH {
        return Cow::Borrowed(whole);
    }
    for template in TEMPLATES {
        let Some(values) = fill_in(template, text) else { continue };
        let mut out = String::new();
        for part in segments(lang.tr(template)) {
            match part {
                Segment::Text(s) => out.push_str(s),
                Segment::Slot(slot) => match values.iter().find(|(name, _)| *name == slot) {
                    Some((_, value)) if NESTED.contains(&slot) => out.push_str(&translate_at(lang, value, depth + 1)),
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(slot),
                },
            }
        }
        return Cow::Owned(out);
    }
    if text.contains('\n') && depth < MAX_DEPTH {
        return Cow::Owned(text.split('\n').map(|line| translate_at(lang, line, depth + 1)).collect::<Vec<_>>().join("\n"));
    }
    Cow::Borrowed(text)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Segment<'a> {
    Text(&'a str),
    /// A `{name}` placeholder, without the braces.
    Slot(&'a str),
}

fn segments(template: &str) -> Vec<Segment<'_>> {
    let mut out = vec![];
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(len) = rest[open + 1..].find('}') else { break };
        let name = &rest[open + 1..open + 1 + len];
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase()) {
            out.push(Segment::Text(&rest[..open + 1]));
            rest = &rest[open + 1..];
            continue;
        }
        if open > 0 {
            out.push(Segment::Text(&rest[..open]));
        }
        out.push(Segment::Slot(name));
        rest = &rest[open + len + 2..];
    }
    if !rest.is_empty() {
        out.push(Segment::Text(rest));
    }
    out
}

/// The placeholder values when `text` is `template` filled in; each value is non-empty.
fn fill_in<'t, 's>(template: &'t str, text: &'s str) -> Option<Vec<(&'t str, &'s str)>> {
    let parts = segments(template);
    let mut values = vec![];
    let mut rest = text;
    let mut slot: Option<&str> = None;
    for (i, part) in parts.iter().enumerate() {
        match *part {
            Segment::Slot(name) => slot = Some(name),
            Segment::Text(lit) => {
                let Some(name) = slot.take() else {
                    rest = rest.strip_prefix(lit)?;
                    continue;
                };
                // The text after the last placeholder ends the message, so that value runs up to it;
                // an earlier one (a file name) ends at the first match.
                let at = if i + 1 == parts.len() { rest.strip_suffix(lit).map(str::len)? } else { rest.find(lit)? };
                if at == 0 {
                    return None;
                }
                values.push((name, &rest[..at]));
                rest = &rest[at + lit.len()..];
            }
        }
    }
    match slot {
        Some(_) if rest.is_empty() => return None,
        Some(name) => values.push((name, rest)),
        None if !rest.is_empty() => return None,
        None => {}
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_has_a_vietnamese_entry_with_the_same_placeholders() {
        let slots = |s: &str| {
            let mut v: Vec<String> =
                segments(s).into_iter().filter_map(|p| if let Segment::Slot(n) = p { Some(n.to_string()) } else { None }).collect();
            v.sort();
            v
        };
        for template in TEMPLATES {
            let vi = super::super::vi::translate(template).unwrap_or_else(|| panic!("missing Vietnamese: {template}"));
            assert_eq!(slots(template), slots(vi), "{template}");
        }
    }

    #[test]
    fn a_filled_in_template_translates_its_parts() {
        let msg = "We can't open 'Bob's: book.xlsx': the file is not a valid spreadsheet: missing part xl/worksheets/sheet1.xml";
        assert_eq!(
            Language::Vi.tr_message(msg),
            "Không mở được 'Bob's: book.xlsx': tệp không phải là bảng tính hợp lệ: thiếu phần xl/worksheets/sheet1.xml"
        );
        assert_eq!(Language::En.tr_message(msg), msg);
        let xls = "We can't open 'a.xls': the file is not a valid spreadsheet: XLS: this .xls workbook is password-protected (encrypted). \
                   GridCraft can't open encrypted .xls files; remove the password in Excel and try again.";
        assert!(Language::Vi.tr_message(xls).starts_with("Không mở được 'a.xls': tệp không phải là bảng tính hợp lệ: XLS: sổ làm việc .xls này"));
    }

    #[test]
    fn unknown_text_inside_a_template_stays_as_it_is() {
        let msg = "We can't open 'a.xlsx': the file is not a valid zip package: invalid Zip archive: Could not find EOCD";
        assert_eq!(
            Language::Vi.tr_message(msg),
            "Không mở được 'a.xlsx': tệp không phải là gói zip hợp lệ: invalid Zip archive: Could not find EOCD"
        );
        assert_eq!(Language::Vi.tr_message("Something else entirely"), "Something else entirely");
        assert_eq!(Language::Vi.tr_message("Data"), "Dữ liệu", "a whole catalog entry still translates");
        assert_eq!(Language::Vi.tr_message("We can't open '': x"), "We can't open '': x", "an empty file name doesn't fit the template");
    }

    #[test]
    fn templates_split_at_their_placeholders() {
        assert_eq!(
            segments("the encrypted file is damaged ({detail})."),
            [Segment::Text("the encrypted file is damaged ("), Segment::Slot("detail"), Segment::Text(").")]
        );
        assert_eq!(segments("{a} {B} {}"), [Segment::Slot("a"), Segment::Text(" {"), Segment::Text("B} {"), Segment::Text("}")]);
        assert_eq!(
            fill_in("the encrypted file is damaged ({detail}).", "the encrypted file is damaged (bad (header))."),
            Some(vec![("detail", "bad (header)")])
        );
        assert_eq!(fill_in("XLS: {detail}", "XLSB: x"), None);
    }
}
