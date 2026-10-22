use gridcraft_model::{MacroButton, MacroButtonKind};

use crate::IoError;
use crate::read::Ctx;
use crate::xml::El;

const MAX_BUTTONS: usize = 65_536;

/// Best effort, like the notes' VML: the listing is read-only and the controls themselves are
/// kept (or reported) by the passthrough, so an unreadable part is skipped without a warning.
fn metadata(cx: &mut Ctx<'_>, part: &str) -> Option<El> {
    cx.pkg.read_limited(part, 16 * 1024 * 1024).ok().flatten().and_then(|bytes| crate::xml::parse(&bytes).ok())
}

fn walk<'a>(e: &'a El, name: &str, out: &mut Vec<&'a El>) {
    if out.len() > MAX_BUTTONS {
        return;
    }
    if e.name == name {
        out.push(e);
    }
    for c in &e.children {
        walk(c, name, out);
    }
}

fn text(e: &El) -> String {
    let mut out = e.text.clone();
    for c in &e.children {
        out.push_str(&text(c));
    }
    out
}

pub fn read(cx: &mut Ctx<'_>, root: &El, rels: &[crate::package::Rel]) -> Result<Vec<MacroButton>, IoError> {
    let sheet = 0;
    let mut buttons: Vec<(String, MacroButton)> = Vec::new();
    for r in rels.iter().filter(|r| !r.external && r.kind == "vmlDrawing") {
        let Some(root) = metadata(cx, &r.target) else { continue };
        let mut shapes = vec![];
        walk(&root, "shape", &mut shapes);
        if shapes.len() > MAX_BUTTONS {
            return Err(IoError::TooLarge("too many macro buttons".into()));
        }
        for shape in shapes {
            let Some(client) = shape.child("ClientData").filter(|c| c.attr("ObjectType") == Some("Button")) else { continue };
            let id = shape.attr("spid").or_else(|| shape.attr("id")).unwrap_or_default();
            let name = shape.attr("id").unwrap_or(id);
            let macro_name = client.child("FmlaMacro").map(|e| e.text.trim().to_string()).unwrap_or_default();
            let caption = shape.child("textbox").map(text).unwrap_or_default();
            if buttons.len() >= MAX_BUTTONS {
                return Err(IoError::TooLarge("too many macro buttons".into()));
            }
            buttons.push((
                id.trim_start_matches("_x0000_s").into(),
                MacroButton { sheet, name: name.into(), caption, macro_name, kind: MacroButtonKind::FormControl },
            ));
        }
    }
    {
        let mut controls = vec![];
        walk(root, "control", &mut controls);
        if controls.len() > MAX_BUTTONS {
            return Err(IoError::TooLarge("too many macro buttons".into()));
        }
        for control in controls {
            let name = control.attr("name").unwrap_or_default();
            let shape = control.attr("shapeId").unwrap_or_default();
            let id = control.attr("id").or_else(|| control.child("controlPr").and_then(|p| p.attr("id")));
            let Some(r) = id.and_then(|id| rels.iter().find(|r| r.id == id && !r.external)) else { continue };
            let Some(props) = metadata(cx, &r.target) else { continue };
            let active_x = r.kind == "control" && props.name == "ocx";
            if !active_x && props.attr("objectType") != Some("Button") {
                continue;
            }
            let existing = buttons.iter_mut().find(|(id, _)| !shape.is_empty() && id == shape);
            let mut button = existing.as_ref().map(|(_, b)| b.clone()).unwrap_or(MacroButton {
                sheet,
                name: name.into(),
                caption: String::new(),
                macro_name: String::new(),
                kind: MacroButtonKind::FormControl,
            });
            if !name.is_empty() {
                button.name = name.into();
            }
            if active_x {
                button.kind = MacroButtonKind::ActiveX;
                // A control name does not prove that its Click handler exists.
                button.macro_name.clear();
                button.caption =
                    props.kids("ocxPr").find(|p| p.attr("name") == Some("Caption")).and_then(|p| p.attr("value")).unwrap_or_default().into();
            } else if let Some(f) = props.attr("fmlaMacro") {
                button.macro_name = f.into();
            }
            if let Some((_, b)) = existing {
                *b = button;
            } else {
                if buttons.len() >= MAX_BUTTONS {
                    return Err(IoError::TooLarge("too many macro buttons".into()));
                }
                buttons.push((shape.into(), button));
            }
        }
    }
    Ok(buttons.into_iter().map(|(_, b)| b).collect())
}
