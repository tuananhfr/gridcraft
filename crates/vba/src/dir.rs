//! MS-OVBA §2.3.4.2. Special records are not generic size-prefixed records.
use crate::{Error, MAX_MODULES, Module, ModuleKind, Project, Reference, decode};

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8], Error> {
        if size > 1024 * 1024 {
            return Err(Error::Limit("record bytes"));
        }
        let end = self.pos.checked_add(size).ok_or(Error::Limit("record offset"))?;
        let bytes = self.bytes.get(self.pos..end).ok_or(Error::Invalid("truncated dir record"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let bytes: [u8; 2] = self.take(2)?.try_into().map_err(|_| Error::Invalid("u16"))?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| Error::Invalid("u32"))?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn sized(&mut self) -> Result<&'a [u8], Error> {
        let size = self.u32()? as usize;
        self.take(size)
    }

    fn peek(&self) -> Option<u16> {
        let bytes: [u8; 2] = self.bytes.get(self.pos..self.pos + 2)?.try_into().ok()?;
        Some(u16::from_le_bytes(bytes))
    }
}

fn number16(bytes: &[u8]) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(bytes.try_into().map_err(|_| Error::Invalid("record size (u16)"))?))
}

fn number32(bytes: &[u8]) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| Error::Invalid("record size (u32)"))?))
}

fn reference_name(r: &mut Reader<'_>, p: &mut Project) -> Result<String, Error> {
    let name = decode(r.sized()?, p.code_page, &mut p.warnings);
    if name.len() > 1024 {
        return Err(Error::Limit("reference name bytes"));
    }
    // Older producers can omit the Unicode name; do not consume the next reference.
    if r.peek() == Some(0x003e) {
        r.u16()?;
        let unicode = decode(r.sized()?, 1200, &mut p.warnings);
        if unicode.len() > 1024 {
            return Err(Error::Limit("reference name bytes"));
        }
        return Ok(unicode);
    }
    Ok(name)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<(Project, Vec<usize>), Error> {
    let mut r = Reader { bytes, pos: 0 };
    let mut p = Project { name: String::new(), code_page: 1252, modules: Vec::new(), references: Vec::new(), locked: false, warnings: Vec::new() };
    let mut count = None;
    let mut offsets = Vec::new();
    let mut current: Option<Module> = None;
    let mut offset = None;
    let mut ref_name = String::new();
    let mut terminated = false;
    for _ in 0..100_000 {
        if r.pos == bytes.len() {
            break;
        }
        let id = r.u16()?;
        if matches!(id, 0x000d | 0x000e | 0x002f | 0x0033) && p.references.len() >= MAX_MODULES {
            return Err(Error::Limit("reference count"));
        }
        match id {
            0x0016 => ref_name = reference_name(&mut r, &mut p)?,
            0x000d | 0x000e | 0x002f => {
                r.u32()?; // Spec: outer reference sizes MUST be ignored on read.
                let mut libid = decode(r.sized()?, p.code_page, &mut p.warnings);
                let kind = match id {
                    0x000d => {
                        r.take(6)?;
                        "registered"
                    }
                    0x000e => {
                        let relative = decode(r.sized()?, p.code_page, &mut p.warnings);
                        if libid.is_empty() {
                            libid = relative;
                        }
                        r.take(6)?;
                        "project"
                    }
                    _ => {
                        r.take(6)?;
                        if r.peek() == Some(0x0016) {
                            r.u16()?;
                            reference_name(&mut r, &mut p)?;
                        }
                        if r.u16()? != 0x0030 {
                            return Err(Error::Invalid("control reference extension"));
                        }
                        r.u32()?;
                        libid = decode(r.sized()?, p.code_page, &mut p.warnings);
                        r.take(26)?;
                        "control"
                    }
                };
                p.references.push(Reference { name: std::mem::take(&mut ref_name), kind: kind.into(), libid });
            }
            0x0033 => {
                let libid = decode(r.sized()?, p.code_page, &mut p.warnings);
                // A REFERENCEORIGINAL directly followed by 0x002F is the OriginalRecord of that REFERENCECONTROL.
                if r.peek() != Some(0x002f) {
                    p.references.push(Reference { name: std::mem::take(&mut ref_name), kind: "original".into(), libid });
                }
            }
            0x0009 => {
                if r.u32()? != 4 {
                    return Err(Error::Invalid("project version size"));
                }
                r.take(6)?;
            }
            _ => {
                let data = r.sized()?;
                match id {
                    0x0001 => {
                        number32(data)?;
                    }
                    0x0003 => p.code_page = number16(data)?,
                    0x0004 => p.name = decode(data, p.code_page, &mut p.warnings),
                    0x000f => {
                        let n = usize::from(number16(data)?);
                        if count.is_some() || n > MAX_MODULES {
                            return Err(Error::Limit("module count"));
                        }
                        count = Some(n);
                    }
                    0x0019 => {
                        if current.is_some() || count.is_none() || p.modules.len() >= MAX_MODULES {
                            return Err(Error::Invalid("module record order"));
                        }
                        current = Some(Module {
                            name: decode(data, p.code_page, &mut p.warnings),
                            stream_name: String::new(),
                            kind: ModuleKind::Class,
                            source: String::new(),
                            private_module: false,
                            procedures: Vec::new(),
                        });
                        offset = None;
                    }
                    0x0047 | 0x001a | 0x0032 | 0x0031 | 0x0021 | 0x0022 | 0x0025 | 0x0028 => {
                        let module = current.as_mut().ok_or(Error::Invalid("module record outside module"))?;
                        match id {
                            0x0047 => module.name = decode(data, 1200, &mut p.warnings),
                            0x001a => module.stream_name = decode(data, p.code_page, &mut p.warnings),
                            0x0032 => module.stream_name = decode(data, 1200, &mut p.warnings),
                            0x0031 => offset = Some(number32(data)? as usize),
                            0x0021 => module.kind = ModuleKind::Standard,
                            0x0028 => module.private_module = true,
                            _ => {}
                        }
                    }
                    0x002b => {
                        if !data.is_empty() {
                            return Err(Error::Invalid("module terminator"));
                        }
                        let module = current.take().ok_or(Error::Invalid("module terminator order"))?;
                        if module.name.len() > 1024 || module.stream_name.len() > 1024 {
                            return Err(Error::Limit("module name bytes"));
                        }
                        p.modules.push(module);
                        offsets.push(offset.take().ok_or(Error::Invalid("missing module offset"))?);
                    }
                    0x0010 => {
                        if !data.is_empty() || current.is_some() || count != Some(p.modules.len()) {
                            return Err(Error::Invalid("project module count or terminator"));
                        }
                        terminated = true;
                        break;
                    }
                    // Spec-defined size-prefixed fields and Unicode companion fields.
                    0x0002 | 0x0014 | 0x004a | 0x0005 | 0x0040 | 0x0006 | 0x003d | 0x0007 | 0x0008 | 0x000c | 0x003c | 0x0013 | 0x001c | 0x0048
                    | 0x001e | 0x002c => {}
                    _ => return Err(Error::Invalid("unknown dir record layout")),
                }
            }
        }
    }
    if !terminated || r.pos != bytes.len() {
        return Err(Error::Invalid("missing or trailing project terminator"));
    }
    Ok((p, offsets))
}
