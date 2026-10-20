//! OPC package access: reading zip parts with decompression limits, relationships and part-name
//! resolution; writing a zip package deterministically.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use crate::IoError;
use crate::xml::{self, El};

use crate::limits::LIMITS;
pub use crate::limits::Limits;

/// Most entries accepted in the zip directory.
pub const MAX_ENTRIES: usize = LIMITS.entries;

/// Inflation is read in steps this large, so every limit is checked on what actually came out
/// rather than on the sizes the zip headers declare.
const CHUNK: usize = 64 * 1024;
/// Most capacity reserved up front from a declared size, which a hostile file can inflate.
const MAX_PREALLOC: u64 = 16 * 1024 * 1024;

pub struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    /// Lower-case normalized part name → zip index.
    names: HashMap<String, usize>,
    total: u64,
    limits: Limits,
    /// Why the package was refused. Sticky: callers that skip an unreadable optional part must
    /// not turn a bomb into a partly opened workbook.
    refused: Option<String>,
}

/// A relationship.
#[derive(Clone, Debug)]
pub struct Rel {
    pub id: String,
    /// Last path segment of the relationship type (`worksheet`, `styles`, …): the same for the
    /// transitional and strict namespaces.
    pub kind: String,
    /// Resolved part name (no leading `/`), or the raw target for external relationships.
    pub target: String,
    pub external: bool,
}

pub fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8]) -> Result<Package<'a>, IoError> {
        Package::open_with(bytes, LIMITS)
    }

    pub fn open_with(bytes: &'a [u8], limits: Limits) -> Result<Package<'a>, IoError> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| IoError::Zip(e.to_string()))?;
        if zip.len() > limits.entries {
            return Err(IoError::TooLarge(format!("{} zip entries, more than {}", zip.len(), limits.entries)));
        }
        let mut names = HashMap::new();
        for i in 0..zip.len() {
            if let Some(n) = zip.name_for_index(i) {
                names.entry(normalize(n)).or_insert(i);
            }
        }
        Ok(Package { zip, names, total: 0, limits, refused: None })
    }

    pub fn has(&self, part: &str) -> bool {
        self.names.contains_key(&normalize(part))
    }

    /// Why the package was refused, once a part broke a package limit; readers return it even
    /// when the part that broke it was optional.
    pub fn refusal(&self) -> Option<IoError> {
        self.refused.clone().map(IoError::TooLarge)
    }

    fn refuse(&mut self, reason: String) -> IoError {
        IoError::TooLarge(self.refused.get_or_insert(reason).clone())
    }

    /// Reads a part's bytes; `Ok(None)` when it doesn't exist.
    pub fn read(&mut self, part: &str) -> Result<Option<Vec<u8>>, IoError> {
        self.read_limited(part, self.limits.entry)
    }

    /// Reads a small metadata part without allowing it to expand to the general part limit.
    /// Going over `maximum` is the caller's to handle; going over a package limit refuses the
    /// whole package (see [`Package::refusal`]).
    pub fn read_limited(&mut self, part: &str, maximum: u64) -> Result<Option<Vec<u8>>, IoError> {
        if let Some(e) = self.refusal() {
            return Err(e);
        }
        let limits = self.limits;
        let maximum = maximum.min(limits.entry);
        let Some(&idx) = self.names.get(&normalize(part)) else {
            return Ok(None);
        };
        let mut f = self.zip.by_index(idx).map_err(|e| IoError::Zip(format!("{part}: {e}")))?;
        let declared = f.size();
        // The zip crate reads at most this much of the archive for the entry, so it is a true
        // bound on the input even when the header lies about the output.
        let compressed = f.compressed_size().max(1);
        let ratio_exceeded = |inflated: u64| inflated > limits.ratio_floor && inflated > compressed.saturating_mul(limits.ratio);
        let remaining = limits.total.saturating_sub(self.total);
        let limit = maximum.min(remaining);
        let refusal = |inflated: u64| -> Option<String> {
            if ratio_exceeded(inflated) {
                Some(format!("{part} is compressed more than {}:1", limits.ratio))
            } else if inflated > remaining && remaining < maximum {
                Some(format!("the package expands to more than {} bytes", limits.total))
            } else if inflated > limits.entry {
                Some(format!("{part} expands to more than {} bytes", limits.entry))
            } else {
                None
            }
        };
        // Past a package limit by the header alone, refuse too: a skipped required part would be
        // reported as a corrupt workbook rather than a too large one. Nothing is inflated.
        if declared > limit {
            drop(f);
            let what = format!("{part} declares {declared} bytes");
            return Err(if declared > remaining {
                self.refuse(format!("{what}, more than the package may expand to ({} bytes)", limits.total))
            } else if declared > limits.entry {
                self.refuse(format!("{what}, more than {} bytes", limits.entry))
            } else {
                IoError::TooLarge(format!("{what}, more than {maximum}"))
            });
        }
        let mut out: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; CHUNK];
        let stop = loop {
            let n = match f.read(&mut chunk) {
                Ok(0) => break None,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => break Some(Stop::Fail(IoError::Zip(format!("{part}: {e}")))),
            };
            let inflated = (out.len() as u64).saturating_add(n as u64);
            if inflated > limit || ratio_exceeded(inflated) {
                break Some(match refusal(inflated) {
                    Some(reason) => Stop::Refuse(reason),
                    None => Stop::Fail(IoError::TooLarge(format!("{part} expands to more than {maximum} bytes"))),
                });
            }
            if reserve(&mut out, n, declared, limit).is_err() {
                break Some(Stop::Refuse(format!("not enough memory to read {part}")));
            }
            out.extend_from_slice(chunk.get(..n).unwrap_or_default());
        };
        drop(f);
        match stop {
            None => {}
            Some(Stop::Refuse(reason)) => return Err(self.refuse(reason)),
            Some(Stop::Fail(e)) => return Err(e),
        }
        self.total = self.total.saturating_add(out.len() as u64);
        Ok(Some(out))
    }

    pub fn read_xml(&mut self, part: &str) -> Result<Option<El>, IoError> {
        match self.read(part)? {
            Some(b) => xml::parse(&b).map(Some).map_err(|e| prefix(part, e)),
            None => Ok(None),
        }
    }

    /// Relationships of a part (`xl/workbook.xml` → `xl/_rels/workbook.xml.rels`). Missing or
    /// broken rels parts give an empty list.
    pub fn rels(&mut self, part: &str) -> Result<Vec<Rel>, IoError> {
        let (dir, file) = split_dir(part);
        let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let root = match self.read(&rels_name)? {
            Some(b) => match xml::parse(&b) {
                Ok(r) => r,
                Err(_) => return Ok(vec![]),
            },
            None => return Ok(vec![]),
        };
        Ok(root
            .kids("Relationship")
            .filter_map(|r| {
                let id = r.attr("Id")?.to_string();
                let ty = r.attr("Type").unwrap_or("");
                let kind = ty.rsplit('/').next().unwrap_or("").to_string();
                let target_raw = r.attr("Target")?;
                let external = r.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
                let target = if external { target_raw.to_string() } else { resolve(dir, target_raw) };
                Some(Rel { id, kind, target, external })
            })
            .collect())
    }
}

/// Why reading a part stopped early.
enum Stop {
    /// A package limit: the whole package is refused.
    Refuse(String),
    /// This part only.
    Fail(IoError),
}

/// Makes room for `n` more bytes. Capacity doubles but never past `limit` (where the read stops
/// anyway), and an allocation failure is an error rather than an abort of the whole tab.
fn reserve(out: &mut Vec<u8>, n: usize, declared: u64, limit: u64) -> Result<(), std::collections::TryReserveError> {
    let needed = out.len().saturating_add(n);
    if needed <= out.capacity() {
        return Ok(());
    }
    let first = usize::try_from(declared.min(MAX_PREALLOC)).unwrap_or(usize::MAX);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let target = out.capacity().saturating_mul(2).max(first).min(limit).max(needed);
    out.try_reserve_exact(target.saturating_sub(out.len()))
}

fn prefix(part: &str, e: IoError) -> IoError {
    match e {
        IoError::Xml(m) => IoError::Xml(format!("{part}: {m}")),
        o => o,
    }
}

/// `xl/worksheets/sheet1.xml` → (`xl/worksheets`, `sheet1.xml`).
pub fn split_dir(part: &str) -> (&str, &str) {
    let p = part.trim_start_matches('/');
    match p.rfind('/') {
        Some(i) => (p.get(..i).unwrap_or(""), p.get(i + 1..).unwrap_or("")),
        None => ("", p),
    }
}

/// Resolves a relationship target against the source part's directory.
pub fn resolve(dir: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    let target = target.split('#').next().unwrap_or("").to_string();
    let mut segs: Vec<&str> = if target.starts_with('/') { vec![] } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
    for s in target.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

// ---------------------------------------------------------------- writing

/// How hard a package's parts are deflated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packing {
    /// What saved files use.
    Normal,
    /// The fastest deflate level: on a large workbook about five times faster to zip and two
    /// thirds larger, for copies that are only ever read back by GridCraft.
    Fast,
}

impl Packing {
    fn level(self) -> i64 {
        match self {
            Packing::Normal => 6,
            Packing::Fast => 1,
        }
    }
}

pub struct ZipOut {
    zip: zip::ZipWriter<Cursor<Vec<u8>>>,
    packing: Packing,
}

impl ZipOut {
    pub fn new(packing: Packing) -> ZipOut {
        ZipOut { zip: zip::ZipWriter::new(Cursor::new(Vec::new())), packing }
    }
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), IoError> {
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(self.packing.level()))
            .last_modified_time(zip::DateTime::default())
            .large_file(data.len() as u64 >= u32::MAX as u64);
        self.zip.start_file(name, opts).map_err(|e| IoError::Zip(e.to_string()))?;
        self.zip.write_all(data).map_err(|e| IoError::Zip(e.to_string()))?;
        Ok(())
    }
    pub fn finish(self) -> Result<Vec<u8>, IoError> {
        self.zip.finish().map(|c| c.into_inner()).map_err(|e| IoError::Zip(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolving() {
        assert_eq!(resolve("xl", "worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("xl/worksheets", "../drawings/d1.xml"), "xl/drawings/d1.xml");
        assert_eq!(resolve("xl/worksheets", "/xl/tables/t.xml"), "xl/tables/t.xml");
        assert_eq!(resolve("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(resolve("a", "../../../x"), "x");
        assert_eq!(split_dir("xl/workbook.xml"), ("xl", "workbook.xml"));
        assert_eq!(split_dir("x.xml"), ("", "x.xml"));
    }
}
