#![allow(dead_code)]

pub const SUPPORT: &str = "Support Reactions";
pub const COORDINATES: &str = "Joint Coordinates Data";
pub const CONTROL: &str = "Program Control";

pub fn etabs(jet4: bool) -> Vec<u8> {
    let size = if jet4 { 4096 } else { 2048 };
    let mut pages = vec![vec![0u8; size]; 13];
    pages[0][..4].copy_from_slice(&[0, 1, 0, 0]);
    pages[0][4..19].copy_from_slice(b"Standard Jet DB");
    pages[0][20] = u8::from(jet4);
    let key = [0xc7u8, 0xda, 0x39, 0x6b];
    let mut state = [0u8; 256];
    for (i, s) in state.iter_mut().enumerate() {
        *s = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + state[i] as usize + key[i % 4] as usize) % 256;
        state.swap(i, j);
    }
    let (mut i, mut j) = (0, 0);
    for b in &mut pages[0][0x18..0x98] {
        i = (i + 1) % 256;
        j = (j + state[i] as usize) % 256;
        state.swap(i, j);
        *b = state[(state[i] as usize + state[j] as usize) % 256];
    }
    // One inline usage-map row per table; the memo pages deliberately belong to no table.
    let mut maps = Vec::new();
    for page in [3usize, 5, 7, 9] {
        let mut map = vec![0u8; 8];
        map[5 + page / 8] |= 1 << (page % 8);
        maps.push(map);
    }
    pages[1] = data_page(size, jet4, maps.iter().map(|m| (m.clone(), 0)).collect());
    let catalog = [
        Col::new("Name", 10, false, 0, 0, 0),
        Col::new("Id", 4, true, 0, 0, 4),
        Col::new("Type", 3, true, 0, 4, 2),
        Col::new("Flags", 4, true, 0, 6, 4),
    ];
    pages[2] = definition(size, jet4, &catalog, 0);
    let catalog_rows: Vec<_> = [(SUPPORT, 4u32), (COORDINATES, 6), (CONTROL, 8), ("MSysHidden", 2)]
        .iter()
        .map(|(name, page)| {
            let fixed = [page.to_le_bytes().as_slice(), &1i16.to_le_bytes(), &0i32.to_le_bytes()].concat();
            (row(jet4, 4, &fixed, &[text(name, jet4)], 0b1111), 0)
        })
        .collect();
    pages[3] = data_page(size, jet4, catalog_rows);
    let columns = [
        Col::new("Joint", 10, false, 0, 0, 0),
        Col::new("Fz", 7, true, 0, 0, 8),
        Col::new("Selected", 1, true, 0, 8, 0),
        Col::new("Date", 8, true, 0, 8, 8),
        Col::new("Memo", 12, false, 1, 0, 0),
        Col::new("Byte", 2, true, 0, 16, 1),
        Col::new("Int", 3, true, 0, 17, 2),
        Col::new("Long", 4, true, 0, 19, 4),
        Col::new("Single", 6, true, 0, 23, 4),
        Col::new("Currency", 5, true, 0, 27, 8),
        Col::new("Numeric", 16, true, 0, 35, 17),
        Col::new("Nullable", 10, false, 2, 0, 0),
    ];
    pages[4] = definition(size, jet4, &columns, 1);
    let mut fixed = Vec::new();
    fixed.extend_from_slice(&(-123.5f64).to_le_bytes());
    fixed.extend_from_slice(&45000.5f64.to_le_bytes());
    fixed.push(255);
    fixed.extend_from_slice(&(-222i16).to_le_bytes());
    fixed.extend_from_slice(&333333333i32.to_le_bytes());
    fixed.extend_from_slice(&1.25f32.to_le_bytes());
    fixed.extend_from_slice(&(-123456i64).to_le_bytes());
    let mut numeric = vec![0u8; 17];
    numeric[0] = 0x80;
    numeric[13..17].copy_from_slice(&12345u32.to_le_bytes());
    fixed.extend_from_slice(&numeric);
    let memo_text = if jet4 { "Phản lực" } else { "Reaction" };
    let memo = text(memo_text, jet4);
    let mut inline = (0x8000_0000u32 | memo.len() as u32).to_le_bytes().to_vec();
    inline.extend_from_slice(&[0; 8]);
    inline.extend_from_slice(&memo);
    let single = [(0x4000_0000u32 | memo.len() as u32).to_le_bytes(), (10u32 << 8).to_le_bytes(), [0; 4]].concat();
    let multi = [(memo.len() as u32).to_le_bytes(), (11u32 << 8).to_le_bytes(), [0; 4]].concat();
    let mk = |memo: Vec<u8>| row(jet4, columns.len(), &fixed, &[text("J1", jet4), memo, Vec::new()], 0x07ff);
    pages[5] = data_page(size, jet4, vec![(mk(inline), 0), (mk(single), 0), (mk(multi), 0), (mk(vec![]), 0x8000)]);
    pages[6] = definition(size, jet4, &[Col::new("X", 7, true, 0, 0, 8)], 2);
    pages[7] = data_page(size, jet4, vec![(row(jet4, 1, &1.5f64.to_le_bytes(), &[], 1), 0)]);
    pages[8] = definition(size, jet4, &[Col::new("CurrUnits", 10, false, 0, 0, 0)], 3);
    pages[9] = data_page(size, jet4, vec![(row(jet4, 1, &[], &[text("tonf, m, C", jet4)], 1), 0)]);
    pages[10] = data_page(size, jet4, vec![(memo.clone(), 0)]);
    let middle = memo.len() / 2;
    pages[11] = data_page(size, jet4, vec![([(12u32 << 8).to_le_bytes().as_slice(), &memo[..middle]].concat(), 0)]);
    pages[12] = data_page(size, jet4, vec![([0u32.to_le_bytes().as_slice(), &memo[middle..]].concat(), 0)]);
    pages.concat()
}
struct Col<'a> {
    name: &'a str,
    kind: u8,
    fixed: bool,
    variable: u16,
    offset: u16,
    length: u16,
}
impl<'a> Col<'a> {
    fn new(name: &'a str, kind: u8, fixed: bool, variable: u16, offset: u16, length: u16) -> Self {
        Self { name, kind, fixed, variable, offset, length }
    }
}
fn put16(b: &mut [u8], p: usize, n: u16) {
    b[p..p + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes());
}
fn definition(size: usize, jet4: bool, columns: &[Col<'_>], map_row: u32) -> Vec<u8> {
    let mut p = vec![0; size];
    p[0] = 2;
    let (cp, vp, mp, mut pos, span, np, var, flag, off, len, scale) =
        if jet4 { (45, 43, 55, 63, 25, 5, 7, 15, 21, 23, 11) } else { (25, 23, 35, 43, 18, 1, 3, 13, 14, 16, 9) };
    put16(&mut p, cp, columns.len() as u16);
    put16(&mut p, vp, columns.iter().filter(|c| !c.fixed).count() as u16);
    put32(&mut p, mp, (1 << 8) | map_row);
    for (i, c) in columns.iter().enumerate() {
        p[pos] = c.kind;
        put16(&mut p, pos + np, i as u16);
        put16(&mut p, pos + var, c.variable);
        p[pos + flag] = u8::from(c.fixed);
        put16(&mut p, pos + off, c.offset);
        put16(&mut p, pos + len, c.length);
        if c.kind == 16 {
            p[pos + scale] = 2;
        }
        pos += span;
    }
    for c in columns {
        let name: Vec<u8> = if jet4 { c.name.encode_utf16().flat_map(u16::to_le_bytes).collect() } else { c.name.as_bytes().to_vec() };
        if jet4 {
            put16(&mut p, pos, name.len() as u16);
            pos += 2;
        } else {
            p[pos] = name.len() as u8;
            pos += 1;
        }
        p[pos..pos + name.len()].copy_from_slice(&name);
        pos += name.len();
    }
    p
}
fn data_page(size: usize, jet4: bool, rows: Vec<(Vec<u8>, u16)>) -> Vec<u8> {
    let mut page = vec![0; size];
    page[0] = 1;
    let count = if jet4 { 12 } else { 8 };
    put16(&mut page, count, rows.len() as u16);
    let mut end = size;
    for (i, (row, flags)) in rows.into_iter().enumerate() {
        end -= row.len();
        page[end..end + row.len()].copy_from_slice(&row);
        put16(&mut page, count + 2 + 2 * i, end as u16 | flags);
    }
    page
}
fn text(s: &str, jet4: bool) -> Vec<u8> {
    if jet4 {
        if s.is_ascii() { [vec![255, 254], s.as_bytes().to_vec()].concat() } else { s.encode_utf16().flat_map(u16::to_le_bytes).collect() }
    } else {
        s.as_bytes().to_vec()
    }
}
fn row(jet4: bool, columns: usize, fixed: &[u8], vars: &[Vec<u8>], mask: u16) -> Vec<u8> {
    let mut b = if jet4 { (columns as u16).to_le_bytes().to_vec() } else { vec![columns as u8] };
    b.extend_from_slice(fixed);
    let mut offsets = Vec::new();
    for v in vars {
        offsets.push(b.len());
        b.extend_from_slice(v);
    }
    if !vars.is_empty() {
        offsets.push(b.len());
        for offset in offsets.into_iter().rev() {
            if jet4 {
                b.extend_from_slice(&(offset as u16).to_le_bytes());
            } else {
                b.push(offset as u8);
            }
        }
        if jet4 {
            b.extend_from_slice(&(vars.len() as u16).to_le_bytes());
        } else {
            b.push(vars.len() as u8);
        }
    }
    b.extend_from_slice(&mask.to_le_bytes()[..columns.div_ceil(8)]);
    b
}
