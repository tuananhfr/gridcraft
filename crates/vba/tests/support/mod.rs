use std::io::{Cursor, Write};

pub fn record(out: &mut Vec<u8>, id: u16, bytes: &[u8]) {
    out.extend(id.to_le_bytes());
    out.extend((bytes.len() as u32).to_le_bytes());
    out.extend(bytes);
}

pub fn unicode(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

pub fn compress(input: &[u8]) -> Vec<u8> {
    let mut result = vec![1];
    for chunk in input.chunks(4096) {
        if chunk.len() == 4096 {
            result.extend(0x3fffu16.to_le_bytes());
            result.extend(chunk);
            continue;
        }
        let mut data = Vec::new();
        let mut pos = 0usize;
        while pos < chunk.len() {
            let flag_pos = data.len();
            data.push(0);
            for bit in 0..8 {
                if pos == chunk.len() {
                    break;
                }
                let bits = (usize::BITS - pos.saturating_sub(1).leading_zeros()).max(4);
                let max_length = usize::from(u16::MAX >> bits) + 3;
                let mut best = (0, 0);
                for offset in 1..=pos {
                    let mut length = 0;
                    while length < max_length && pos + length < chunk.len() && chunk[pos + length] == chunk[pos + length - offset] {
                        length += 1;
                    }
                    if length > best.1 {
                        best = (offset, length);
                    }
                }
                if best.1 >= 3 {
                    data[flag_pos] |= 1 << bit;
                    let token = (((best.0 - 1) as u16) << (16 - bits)) | (best.1 - 3) as u16;
                    data.extend(token.to_le_bytes());
                    pos += best.1;
                } else {
                    data.push(chunk[pos]);
                    pos += 1;
                }
            }
        }
        assert!(data.len() <= 4096);
        result.extend((0xb000 | (data.len() as u16 - 1)).to_le_bytes());
        result.extend(data);
    }
    result
}

pub struct FixtureModule<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    pub source: Vec<u8>,
    pub private: bool,
}

impl<'a> FixtureModule<'a> {
    pub fn new(name: &'a str, kind: &'a str, source: &str) -> Self {
        Self { name, kind, source: compress(source.as_bytes()), private: false }
    }
}

pub fn fixture(modules: &[FixtureModule<'_>], page: u16, protection: &str, references: &[u8]) -> Vec<u8> {
    let mut dir = Vec::new();
    record(&mut dir, 1, &1u32.to_le_bytes());
    record(&mut dir, 2, &0x409u32.to_le_bytes());
    record(&mut dir, 0x14, &0x409u32.to_le_bytes());
    record(&mut dir, 3, &page.to_le_bytes());
    record(&mut dir, 4, b"Synthetic");
    record(&mut dir, 5, b"");
    record(&mut dir, 0x40, b"");
    record(&mut dir, 6, b"");
    record(&mut dir, 0x3d, b"");
    record(&mut dir, 7, &0u32.to_le_bytes());
    record(&mut dir, 8, &0u32.to_le_bytes());
    record(&mut dir, 9, &1u32.to_le_bytes());
    dir.extend(0u16.to_le_bytes());
    record(&mut dir, 0xc, b"");
    record(&mut dir, 0x3c, b"");
    dir.extend(references);
    record(&mut dir, 0xf, &(modules.len() as u16).to_le_bytes());
    record(&mut dir, 0x13, &0xffffu16.to_le_bytes());
    let mut project = format!("ID=\"Synthetic\"\r\n{protection}");
    let mut streams = Vec::new();
    for (i, module) in modules.iter().enumerate() {
        let stream = format!("Stream{i}");
        record(&mut dir, 0x19, module.name.as_bytes());
        record(&mut dir, 0x47, &unicode(module.name));
        record(&mut dir, 0x1a, stream.as_bytes());
        record(&mut dir, 0x32, &unicode(&stream));
        record(&mut dir, 0x1c, b"");
        record(&mut dir, 0x48, b"");
        record(&mut dir, 0x31, &4u32.to_le_bytes());
        record(&mut dir, 0x1e, &0u32.to_le_bytes());
        record(&mut dir, 0x2c, &0xffffu16.to_le_bytes());
        record(&mut dir, if module.kind == "Module" { 0x21 } else { 0x22 }, b"");
        record(&mut dir, 0x25, b"");
        if module.private {
            record(&mut dir, 0x28, b"");
        }
        record(&mut dir, 0x2b, b"");
        let suffix = if module.kind == "Document" { "/&H00000000" } else { "" };
        project.push_str(&format!("{}={}{suffix}\r\n", module.kind, module.name));
        let mut source = vec![0; 4];
        source.extend(&module.source);
        streams.push((format!("/VBA/{stream}"), source));
    }
    // Later sections reuse property-like keys; the analyzer must stop at the first section header.
    project.push_str("\r\n[Workspace]\r\nGC=\"0\"\r\nModule=0, 0, 0, 0, C\r\n");
    record(&mut dir, 0x10, b"");
    let mut cfb = cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).unwrap();
    cfb.create_storage("/VBA").unwrap();
    streams.extend([
        ("/PROJECT".into(), project.into_bytes()),
        ("/VBA/dir".into(), compress(&dir)),
        ("/VBA/_VBA_PROJECT".into(), vec![0xcc, 0x61, 0xff, 0xff, 0, 0, 0]),
    ]);
    for (name, bytes) in streams {
        cfb.create_stream(name).unwrap().write_all(&bytes).unwrap();
    }
    cfb.into_inner().into_inner()
}

/// MS-OVBA §2.4.3 data encryption of the PROJECT stream CMG/GC/DPB values.
pub fn encrypt(seed: u8, data: &[u8]) -> String {
    let project_key = 0x5a;
    let (version_enc, key_enc) = (seed ^ 2, seed ^ project_key);
    let mut out = vec![seed, version_enc, key_enc];
    let (mut plain, mut enc1, mut enc2) = (project_key, key_enc, version_enc);
    let ignored = vec![0u8; usize::from((seed & 6) / 2)];
    for &byte in ignored.iter().chain(&(data.len() as u32).to_le_bytes()).chain(data) {
        let byte_enc = byte ^ enc2.wrapping_add(plain);
        out.push(byte_enc);
        enc2 = enc1;
        enc1 = byte_enc;
        plain = byte;
    }
    format!("\"{}\"", out.iter().map(|b| format!("{b:02X}")).collect::<String>())
}
