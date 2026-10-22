use gridcraft_model::{VbaSignaturePart, VbaSignatures, Workbook};
use sha2::{Digest, Sha256};

use crate::package::{resolve, split_dir};
use crate::read::Ctx;
use crate::{IoError, xml};

pub const LEGACY_SIGNATURE: &str = "The macro digital signature in the .xls file is not preserved when saving as .xlsm; the macros will be unsigned.";
pub const DROPPED_SIGNATURE: &str = "Some VBA signature relationships could not be preserved; the saved macros may be unsigned.";

/// The writer always puts the project at `xl/vbaProject.bin` and emits no other `.bin` beside it,
/// so a kept signature part there can neither collide nor lose its relative target.
pub(crate) fn is_signature_part(name: &str) -> bool {
    let (dir, file) = split_dir(name);
    dir == "xl"
        && file.len() > 4
        && file.get(file.len() - 4..).is_some_and(|ext| ext.eq_ignore_ascii_case(".bin"))
        && !file.eq_ignore_ascii_case("vbaProject.bin")
}

pub fn read(cx: &mut Ctx<'_>, wb: &mut Workbook, project: &str) -> Result<(), IoError> {
    let (dir, file) = split_dir(project);
    let rels_part = format!("{dir}/_rels/{file}.rels");
    let Some(bytes) = cx.pkg.read_limited(&rels_part, 16 * 1024 * 1024)? else { return Ok(()) };
    let root = xml::parse(&bytes)?;
    if root.name != "Relationships" {
        wb.vba_signature_loss = true;
        cx.warn(DROPPED_SIGNATURE);
        return Ok(());
    }
    let types = cx.pkg.read_xml("[Content_Types].xml")?.unwrap_or_default();
    let mut parts = Vec::new();
    let mut kept = String::from("<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
    let mut dropped = false;
    for (index, r) in root.kids("Relationship").enumerate() {
        if index >= 10_000 {
            return Err(IoError::TooLarge("too many VBA signature parts".into()));
        }
        let target = r.attr("Target").unwrap_or("");
        let name = resolve(dir, target);
        let (pd, pf) = split_dir(&name);
        let content_type = types
            .kids("Override")
            .find(|e| e.attr("PartName").is_some_and(|p| p.trim_start_matches('/').eq_ignore_ascii_case(&name)))
            .and_then(|e| e.attr("ContentType"))
            .or_else(|| types.kids("Default").find(|e| e.attr("Extension") == name.rsplit('.').next()).and_then(|e| e.attr("ContentType")));
        // Only leaf parts are safe; never follow external targets or arbitrary graphs.
        if r.attr("Id").is_none_or(str::is_empty)
            || r.attr("Type").is_none_or(str::is_empty)
            || r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"))
            || target.contains([':', '#'])
            || name == project
            || !is_signature_part(&name)
            || cx.pkg.has(&format!("{pd}/_rels/{pf}.rels"))
            || content_type.is_none()
        {
            dropped = true;
            continue;
        }
        let Some(data) = cx.pkg.read(&name)? else {
            dropped = true;
            continue;
        };
        let ty = r.attr("Type").unwrap_or("").to_string();
        kept.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"/>",
            xml::esc_attr(r.attr("Id").unwrap_or("")),
            xml::esc_attr(&ty),
            xml::esc_attr(target)
        ));
        parts.push(VbaSignaturePart { name, relationship_type: ty, content_type: content_type.unwrap_or_default().into(), data });
    }
    kept.push_str("</Relationships>");
    if dropped {
        wb.vba_signature_loss = true;
        cx.warn(DROPPED_SIGNATURE);
    }
    if parts.is_empty() {
        return Ok(());
    }
    wb.vba_signatures = Some(VbaSignatures {
        project_part: project.into(),
        project_sha256: Sha256::digest(wb.vba_project.as_deref().unwrap_or_default()).to_vec(),
        relationships: if dropped { kept.into_bytes() } else { bytes },
        parts,
    });
    Ok(())
}
