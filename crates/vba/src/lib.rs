//! Bounded, read-only analysis of MS-OVBA projects. No VBA or p-code is executed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod classify;
mod compression;
mod dir;
mod protection;
mod source;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod testing;

use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_STREAM: usize = 16 * 1024 * 1024;
const MAX_MODULES: usize = 4096;
const MAX_LINES: usize = 500_000;
const MAX_PROCEDURES: usize = 16_384;
const MAX_REASONS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid VBA project: {0}")]
    Invalid(&'static str),
    #[error("VBA project exceeds the {0} safety limit")]
    Limit(&'static str),
    #[error("VBA compound file: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub code_page: u16,
    pub modules: Vec<Module>,
    pub references: Vec<Reference>,
    /// From the PROJECT stream protection and visibility flags; true when they cannot be decoded.
    pub locked: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub stream_name: String,
    pub kind: ModuleKind,
    pub source: String,
    pub private_module: bool,
    pub procedures: Vec<Procedure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleKind {
    Standard,
    Document,
    Class,
    Form,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Procedure {
    pub name: String,
    pub kind: ProcKind,
    pub scope: Scope,
    pub params: Vec<Param>,
    pub returns: Option<String>,
    pub line: u32,
    pub event: Option<String>,
    pub udf: bool,
    pub compatibility: Compatibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcKind {
    Sub,
    Function,
    PropertyGet,
    PropertyLet,
    PropertySet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Public,
    Private,
    Friend,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub by_val: bool,
    pub optional: bool,
    pub param_array: bool,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Compatibility {
    pub level: u8,
    pub reasons: Vec<Reason>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reason {
    pub code: String,
    pub detail: String,
    pub line: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub name: String,
    pub kind: String,
    pub libid: String,
}

/// Case-insensitive lookup of a public worksheet function in a standard module.
pub fn find_udf<'a>(p: &'a Project, name: &str) -> Option<(&'a Module, &'a Procedure)> {
    let name = name.to_lowercase();
    p.modules.iter().find_map(|m| m.procedures.iter().find(|f| f.udf && f.name.to_lowercase() == name).map(|f| (m, f)))
}

/// Analyze an OLE/CFB `vbaProject.bin`; never executes or modifies the project.
pub fn analyze(vba_project_bin: &[u8]) -> Result<Project, Error> {
    if vba_project_bin.len() > MAX_BYTES {
        return Err(Error::Limit("compound file bytes"));
    }
    let mut file = cfb::CompoundFile::open(Cursor::new(vba_project_bin))?;
    let mut budget = MAX_BYTES;
    let mut read_budget = MAX_BYTES;
    let compressed = read_stream(&mut file, "/VBA/dir", &mut read_budget)?;
    let bytes = compression::decompress(&compressed, &mut budget)?;
    let (mut project, offsets) = dir::parse(&bytes)?;
    let text = decode(&read_stream(&mut file, "/PROJECT", &mut read_budget)?, project.code_page, &mut project.warnings);
    let mut kinds = std::collections::BTreeMap::new();
    let mut protection = protection::Protection::default();
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_MODULES * 8 {
            return Err(Error::Limit("PROJECT text lines"));
        }
        let line = line.trim();
        // [Host Extender Info] and [Workspace] follow the properties and use module names as keys.
        if line.starts_with('[') {
            break;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let kind = match key.as_str() {
            "module" => Some(ModuleKind::Standard),
            "document" => Some(ModuleKind::Document),
            "class" => Some(ModuleKind::Class),
            "baseclass" => Some(ModuleKind::Form),
            _ => None,
        };
        if let Some(kind) = kind {
            let name = value.split('/').next().unwrap_or(value).trim().to_lowercase();
            if kinds.len() >= MAX_MODULES && !kinds.contains_key(&name) {
                return Err(Error::Limit("PROJECT module names"));
            }
            kinds.insert(name, kind);
        }
        protection.read(&key, value);
    }
    for module in &mut project.modules {
        if let Some(kind) = kinds.get(&module.name.to_lowercase()) {
            module.kind = *kind;
        }
    }
    match protection.locked() {
        Some(false) => {}
        Some(true) => {
            project.locked = true;
            project.warnings.push("Project is protected; protection was not bypassed".into());
        }
        None => {
            project.locked = true;
            project.warnings.push("Project protection state could not be decoded; treated as locked".into());
        }
    }
    for (module, offset) in project.modules.iter_mut().zip(offsets) {
        if module.stream_name.is_empty() || module.stream_name.contains(['/', '\\']) || matches!(module.stream_name.as_str(), "." | "..") {
            return Err(Error::Invalid("module stream name"));
        }
        let path = format!("/VBA/{}", module.stream_name);
        let data = if file.is_stream(&path) { Some(read_stream(&mut file, &path, &mut read_budget)?) } else { None };
        let result = data
            .as_deref()
            .and_then(|d| d.get(offset..))
            .ok_or(Error::Invalid("missing module source"))
            .and_then(|d| compression::decompress(d, &mut budget));
        match result {
            Ok(bytes) => module.source = decode(&bytes, project.code_page, &mut project.warnings),
            Err(Error::Limit(limit)) => return Err(Error::Limit(limit)),
            Err(error) => project.warnings.push(format!("{}: p-code only or source removed ({error})", module.name)),
        }
    }
    source::analyze(&mut project)?;
    Ok(project)
}

fn read_stream(file: &mut cfb::CompoundFile<Cursor<&[u8]>>, path: &str, budget: &mut usize) -> Result<Vec<u8>, Error> {
    let stream = file.open_stream(path)?;
    if stream.len() > MAX_STREAM as u64 {
        return Err(Error::Limit("stream bytes"));
    }
    *budget = budget.checked_sub(stream.len() as usize).ok_or(Error::Limit("total stream bytes"))?;
    let mut bytes = Vec::new();
    stream.take(MAX_STREAM as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_STREAM {
        return Err(Error::Limit("stream bytes"));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], code_page: u16, warnings: &mut Vec<String>) -> String {
    let encoding = match code_page {
        1200 => Some(encoding_rs::UTF_16LE),
        1201 => Some(encoding_rs::UTF_16BE),
        65001 => Some(encoding_rs::UTF_8),
        932 => Some(encoding_rs::SHIFT_JIS),
        936 => Some(encoding_rs::GBK),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        874 => Some(encoding_rs::WINDOWS_874),
        1250..=1258 => encoding_rs::Encoding::for_label(format!("windows-{code_page}").as_bytes()),
        _ => None,
    };
    if encoding.is_none() {
        let warning = format!("Unsupported code page {code_page}; decoded lossily as Windows-1252");
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    let (text, _, lossy) = encoding.unwrap_or(encoding_rs::WINDOWS_1252).decode(bytes);
    if lossy && !warnings.iter().any(|w| w == "Text contained invalid encoded characters") {
        warnings.push("Text contained invalid encoded characters".into());
    }
    text.into_owned()
}
