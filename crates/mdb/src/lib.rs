//! Bounded, read-only Jet 3/4 decoding from bytes. No filesystem or SQL.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, BTreeSet};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid Access database: {0}")]
    Invalid(&'static str),
    #[error("Unsupported Access format (only unencrypted Jet 3 and Jet 4 are supported)")]
    Unsupported,
    #[error("Access database exceeds the {0} safety limit")]
    Limit(&'static str),
    #[error("Access table not found: {0}")]
    TableNotFound(String),
}

pub const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_COLUMNS: usize = 256;
const MAX_ROWS: usize = 1_048_575;
const MAX_STRING: usize = 4 * 1024 * 1024;
const MAX_WORK: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    Bool,
    Byte,
    Int,
    Long,
    Single,
    Double,
    Currency,
    DateTime,
    Text,
    Memo,
    Numeric,
    Binary,
}
#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub kind: ColumnType,
    number: usize,
    variable: usize,
    fixed: bool,
    offset: usize,
    length: usize,
    scale: u8,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Byte(u8),
    Int(i16),
    Long(i32),
    Single(f32),
    Double(f64),
    Currency(String),
    DateTime(f64),
    Text(String),
    Numeric(String),
    Binary(Vec<u8>),
}
#[derive(Clone, Debug)]
pub struct TableInfo {
    pub name: String,
    pub columns: Vec<Column>,
    pub row_count: usize,
}
struct Definition {
    columns: Vec<Column>,
    pages: Vec<usize>,
    has_variable: bool,
}
pub struct Database<'a> {
    bytes: &'a [u8],
    jet4: bool,
    page_size: usize,
    encoding: &'static encoding_rs::Encoding,
    tables: Vec<TableInfo>,
    definitions: BTreeMap<String, Definition>,
}
pub struct Table<'a, 'b> {
    db: &'a Database<'b>,
    definition: &'a Definition,
}
impl Table<'_, '_> {
    pub fn columns(&self) -> &[Column] {
        &self.definition.columns
    }
    pub fn rows(&self) -> impl Iterator<Item = Result<Vec<Value>>> + '_ {
        let mut work = 0usize;
        self.definition.pages.iter().flat_map(move |page| {
            let count = self.db.page(*page).and_then(|p| u16_at(p, self.db.row_offset())).map(usize::from);
            let mut items = Vec::new();
            match count {
                Err(e) => items.push(Err(e)),
                Ok(count) => {
                    for row in 0..count {
                        let parsed = (|| {
                            let p = self.db.page(*page)?;
                            let ptr = u16_at(p, self.db.row_offset() + 2 + row * 2)?;
                            if ptr & 0x8000 != 0 {
                                return Ok(None);
                            }
                            let raw = self.db.resolve_row(*page, row)?;
                            work = work.saturating_add(raw.len());
                            if work > MAX_WORK {
                                return Err(Error::Limit("row decoding"));
                            }
                            self.db.decode_row(raw, self.definition, &mut work).map(Some)
                        })();
                        match parsed {
                            Ok(None) => (),
                            Ok(Some(v)) => items.push(Ok(v)),
                            Err(e) => {
                                items.push(Err(e));
                                break;
                            }
                        }
                    }
                }
            }
            items
        })
    }
}
impl<'b> Database<'b> {
    pub fn open(bytes: &'b [u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Limit("128 MiB input"));
        }
        if bytes.get(0..4) != Some(&[0, 1, 0, 0]) || bytes.get(4..19) != Some(b"Standard Jet DB") {
            return Err(Error::Unsupported);
        }
        let jet4 = match byte(bytes, 20)? {
            0 => false,
            1 => true,
            _ => return Err(Error::Unsupported),
        };
        let page_size = if jet4 { 4096 } else { 2048 };
        if bytes.len() < page_size * 3 || !bytes.len().is_multiple_of(page_size) {
            return Err(Error::Invalid("truncated pages"));
        }
        // Jet masks the header even for unencrypted files; reject a nonzero database key.
        let mut header = slice(bytes, 0x18, 128)?.to_vec();
        rc4(&mut header);
        if u32_at(&header, 0x3e - 0x18)? != 0 {
            return Err(Error::Unsupported);
        }
        let code_page = u16_at(&header, 0x3c - 0x18)?;
        let label = match code_page {
            0 => "windows-1252".to_owned(),
            932 => "shift_jis".to_owned(),
            936 => "gbk".to_owned(),
            949 => "euc-kr".to_owned(),
            950 => "big5".to_owned(),
            65001 => "utf-8".to_owned(),
            n => format!("windows-{n}"),
        };
        let encoding = if jet4 { encoding_rs::WINDOWS_1252 } else { encoding_rs::Encoding::for_label(label.as_bytes()).ok_or(Error::Unsupported)? };
        let mut db = Self { bytes, jet4, page_size, encoding, tables: Vec::new(), definitions: BTreeMap::new() };
        let catalog = db.definition(2)?;
        let name = catalog.columns.iter().position(|c| c.name == "Name").ok_or(Error::Invalid("catalog Name"))?;
        let id = catalog.columns.iter().position(|c| c.name == "Id").ok_or(Error::Invalid("catalog Id"))?;
        let kind = catalog.columns.iter().position(|c| c.name == "Type").ok_or(Error::Invalid("catalog Type"))?;
        let flags = catalog.columns.iter().position(|c| c.name == "Flags");
        let catalog_table = Table { db: &db, definition: &catalog };
        let mut entries = Vec::new();
        for row in catalog_table.rows() {
            let row = row?;
            if !matches!(row.get(kind), Some(Value::Int(1)) | Some(Value::Long(1))) {
                continue;
            }
            if let Some(Value::Long(f)) = flags.and_then(|i| row.get(i))
                && (*f as u32) & 0x8000_0002 != 0
            {
                continue;
            }
            if let (Some(Value::Text(n)), Some(Value::Long(p))) = (row.get(name), row.get(id))
                && !n.to_ascii_lowercase().starts_with("msys")
                && !n.starts_with('~')
            {
                if entries.len() >= 1024 {
                    return Err(Error::Limit("1024 tables"));
                }
                entries.push((n.clone(), (*p as u32 & 0x00ff_ffff) as usize));
            }
        }
        let mut total_work = 0usize;
        for (name, page) in entries {
            let definition = db.definition(page)?;
            let schema_size = definition.columns.iter().map(|c| c.name.len().saturating_add(std::mem::size_of::<Column>())).sum::<usize>();
            total_work = total_work
                .saturating_add(schema_size.saturating_mul(2))
                .saturating_add(definition.pages.len().saturating_mul(std::mem::size_of::<usize>()));
            if total_work > MAX_WORK {
                return Err(Error::Limit("64 MiB decoded database"));
            }
            let table = Table { db: &db, definition: &definition };
            let mut row_count = 0;
            for row in table.rows() {
                let row = row?;
                total_work = total_work.saturating_add(row.iter().map(value_size).sum::<usize>());
                if total_work > MAX_WORK {
                    return Err(Error::Limit("64 MiB decoded database"));
                }
                row_count += 1;
                if row_count > MAX_ROWS {
                    return Err(Error::Limit("row count"));
                }
            }
            db.tables.push(TableInfo { name: name.clone(), columns: definition.columns.clone(), row_count });
            db.definitions.insert(name, definition);
        }
        Ok(db)
    }
    pub fn tables(&self) -> &[TableInfo] {
        &self.tables
    }
    pub fn table(&self, name: &str) -> Result<Table<'_, 'b>> {
        let (_, definition) = self.definitions.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).ok_or_else(|| Error::TableNotFound(name.into()))?;
        Ok(Table { db: self, definition })
    }
    fn row_offset(&self) -> usize {
        if self.jet4 { 12 } else { 8 }
    }
    fn page(&self, page: usize) -> Result<&'b [u8]> {
        let start = page.checked_mul(self.page_size).ok_or(Error::Invalid("page address"))?;
        slice(self.bytes, start, self.page_size)
    }
    fn raw_row(&self, page: usize, row: usize) -> Result<(&'b [u8], u16)> {
        let p = self.page(page)?;
        if byte(p, 0)? != 1 || row >= usize::from(u16_at(p, self.row_offset())?) {
            return Err(Error::Invalid("row pointer"));
        }
        let pos = self.row_offset() + 2 + row * 2;
        let ptr = u16_at(p, pos)?;
        let start = usize::from(ptr & 0x1fff);
        let end = if row == 0 { self.page_size } else { usize::from(u16_at(p, pos - 2)? & 0x1fff) };
        if start < pos + 2 {
            return Err(Error::Invalid("row overlaps directory"));
        }
        Ok((slice(p, start, end.checked_sub(start).ok_or(Error::Invalid("row bounds"))?)?, ptr))
    }
    fn resolve_row(&self, mut page: usize, mut row: usize) -> Result<&'b [u8]> {
        let mut visited = BTreeSet::new();
        for _ in 0..64 {
            if !visited.insert((page, row)) {
                return Err(Error::Invalid("cyclic overflow row"));
            }
            let (data, flags) = self.raw_row(page, row)?;
            if flags & 0x4000 == 0 {
                return Ok(data);
            }
            let ptr = u32_at(data, 0)?;
            page = (ptr >> 8) as usize;
            row = (ptr & 255) as usize;
        }
        Err(Error::Limit("64 overflow hops"))
    }
    fn definition(&self, table_page: usize) -> Result<Definition> {
        let first = self.page(table_page)?;
        if byte(first, 0)? != 2 {
            return Err(Error::Invalid("table definition page"));
        }
        let mut data = first.to_vec();
        let mut next = u32_at(first, 4)? as usize;
        let mut visited = BTreeSet::from([table_page]);
        while next != 0 {
            if !visited.insert(next) {
                return Err(Error::Invalid("cyclic table definition"));
            }
            if data.len() > MAX_STRING {
                return Err(Error::Limit("table definition"));
            }
            let p = self.page(next)?;
            data.extend_from_slice(slice(p, 8, self.page_size - 8)?);
            next = u32_at(p, 4)? as usize;
        }
        let (cols_pos, idx_pos, start, idx_size, span, num, var, flag, off, len, scale, map_pos, var_pos) =
            if self.jet4 { (45, 51, 63, 12, 25, 5, 7, 15, 21, 23, 11, 55, 43) } else { (25, 31, 43, 8, 18, 1, 3, 13, 14, 16, 9, 35, 23) };
        let count = usize::from(u16_at(&data, cols_pos)?);
        if count == 0 || count > MAX_COLUMNS {
            return Err(Error::Limit("256 columns"));
        }
        let indexes = u32_at(&data, idx_pos)? as usize;
        if indexes > 1024 {
            return Err(Error::Limit("indexes"));
        }
        let mut pos = start + indexes * idx_size;
        let mut columns = Vec::new();
        for _ in 0..count {
            let entry = slice(&data, pos, span)?;
            let kind = match byte(entry, 0)? {
                1 => ColumnType::Bool,
                2 => ColumnType::Byte,
                3 => ColumnType::Int,
                4 => ColumnType::Long,
                5 => ColumnType::Currency,
                6 => ColumnType::Single,
                7 => ColumnType::Double,
                8 => ColumnType::DateTime,
                10 => ColumnType::Text,
                12 => ColumnType::Memo,
                16 => ColumnType::Numeric,
                9 | 11 | 15 => ColumnType::Binary,
                _ => return Err(Error::Unsupported),
            };
            columns.push(Column {
                name: String::new(),
                kind,
                number: usize::from(u16_at(entry, num)?),
                variable: usize::from(u16_at(entry, var)?),
                fixed: byte(entry, flag)? & 1 != 0,
                offset: usize::from(u16_at(entry, off)?),
                length: usize::from(u16_at(entry, len)?),
                scale: byte(entry, scale)?,
            });
            pos += span;
        }
        for column in &mut columns {
            let length = if self.jet4 { usize::from(u16_at(&data, pos)?) } else { usize::from(byte(&data, pos)?) };
            pos += if self.jet4 { 2 } else { 1 };
            if length > 512 {
                return Err(Error::Limit("column name length"));
            }
            column.name = decode_text(slice(&data, pos, length)?, self.jet4, self.encoding)?;
            pos += length;
        }
        columns.sort_by_key(|c| c.number);
        let map = u32_at(&data, map_pos)?;
        let pages = self.owned_pages(map)?;
        Ok(Definition { columns, pages, has_variable: u16_at(&data, var_pos)? != 0 })
    }
    fn owned_pages(&self, pointer: u32) -> Result<Vec<usize>> {
        if pointer == 0 {
            return Ok(Vec::new());
        }
        let (map, _) = self.raw_row((pointer >> 8) as usize, (pointer & 255) as usize)?;
        let mut pages = BTreeSet::new();
        let mut add = |bits: &[u8], base: usize| -> Result<()> {
            for (i, b) in bits.iter().enumerate() {
                for bit in 0..8 {
                    if b & (1 << bit) != 0 {
                        let page = base.checked_add(i * 8 + bit).ok_or(Error::Invalid("usage map address"))?;
                        let p = self.page(page)?;
                        if byte(p, 0)? == 1 {
                            pages.insert(page);
                        }
                    }
                }
            }
            Ok(())
        };
        match byte(map, 0)? {
            0 => add(slice(map, 5, map.len().saturating_sub(5))?, u32_at(map, 1)? as usize)?,
            1 => {
                for (i, chunk) in slice(map, 1, map.len() - 1)?.as_chunks::<4>().0.iter().enumerate() {
                    let ptr = u32_at(chunk, 0)? as usize;
                    if ptr == 0 {
                        continue;
                    }
                    let p = self.page(ptr)?;
                    if byte(p, 0)? != 5 {
                        return Err(Error::Invalid("usage bitmap page"));
                    }
                    add(slice(p, 4, self.page_size - 4)?, i * (self.page_size - 4) * 8)?;
                }
            }
            _ => return Err(Error::Invalid("usage map type")),
        }
        Ok(pages.into_iter().collect())
    }
    fn decode_row(&self, raw: &[u8], def: &Definition, work: &mut usize) -> Result<Vec<Value>> {
        let width = if self.jet4 { 2 } else { 1 };
        let count = if self.jet4 { usize::from(u16_at(raw, 0)?) } else { usize::from(byte(raw, 0)?) };
        if count > 1024 {
            return Err(Error::Limit("row columns"));
        }
        let mask_pos = raw.len().checked_sub(count.div_ceil(8)).ok_or(Error::Invalid("null mask"))?;
        let mask = slice(raw, mask_pos, count.div_ceil(8))?;
        let mut offsets = Vec::new();
        if def.has_variable {
            let vpos = mask_pos.checked_sub(width).ok_or(Error::Invalid("variable count"))?;
            let n = if self.jet4 { usize::from(u16_at(raw, vpos)?) } else { usize::from(byte(raw, vpos)?) };
            if n > 1024 {
                return Err(Error::Limit("variable columns"));
            }
            let jumps = if self.jet4 { 0 } else { raw.len().saturating_sub(1) / 256 };
            let table_end = vpos.checked_sub(jumps).ok_or(Error::Invalid("jump table"))?;
            let mut high = 0;
            for i in 0..=n {
                let p = table_end.checked_sub((i + 1) * width).ok_or(Error::Invalid("variable offsets"))?;
                let value = if self.jet4 {
                    usize::from(u16_at(raw, p)?)
                } else {
                    while high < jumps && usize::from(byte(raw, vpos - 1 - high)?) == i {
                        high += 1;
                    }
                    usize::from(byte(raw, p)?) + 256 * high
                };
                offsets.push(value);
            }
        }
        let mut values = Vec::new();
        for col in &def.columns {
            let present = mask.get(col.number / 8).is_some_and(|b| b & (1 << (col.number % 8)) != 0);
            if col.kind == ColumnType::Bool {
                values.push(Value::Bool(present));
                continue;
            }
            if !present || col.number >= count {
                values.push(Value::Null);
                continue;
            }
            let data = if col.fixed {
                slice(raw, width + col.offset, col.length)?
            } else {
                let start = *offsets.get(col.variable).ok_or(Error::Invalid("variable slot"))?;
                let end = *offsets.get(col.variable + 1).ok_or(Error::Invalid("variable end"))?;
                slice(raw, start, end.checked_sub(start).ok_or(Error::Invalid("variable order"))?)?
            };
            let value = match col.kind {
                ColumnType::Bool => Value::Bool(present),
                ColumnType::Byte => Value::Byte(byte(data, 0)?),
                ColumnType::Int => Value::Int(i16::from_le_bytes(array(data, 0)?)),
                ColumnType::Long => Value::Long(i32::from_le_bytes(array(data, 0)?)),
                ColumnType::Single => Value::Single(f32::from_le_bytes(array(data, 0)?)),
                ColumnType::Double => Value::Double(f64::from_le_bytes(array(data, 0)?)),
                ColumnType::DateTime => Value::DateTime(f64::from_le_bytes(array(data, 0)?)),
                ColumnType::Currency => {
                    let value = i64::from_le_bytes(array(data, 0)?);
                    let magnitude = value.unsigned_abs();
                    Value::Currency(format!("{}{}.{:04}", if value < 0 { "-" } else { "" }, magnitude / 10000, magnitude % 10000))
                }
                ColumnType::Numeric => {
                    if col.scale > 38 {
                        return Err(Error::Invalid("numeric scale"));
                    }
                    let mut number = 0u128;
                    for chunk in slice(data, 1, 16)?.as_chunks::<4>().0 {
                        number = (number << 32) | u128::from(u32_at(chunk, 0)?);
                    }
                    let divisor = 10u128.pow(u32::from(col.scale));
                    let sign = if byte(data, 0)? != 0 && number != 0 { "-" } else { "" };
                    Value::Numeric(if col.scale == 0 {
                        format!("{sign}{number}")
                    } else {
                        format!("{sign}{}.{:0width$}", number / divisor, number % divisor, width = usize::from(col.scale))
                    })
                }
                ColumnType::Text => Value::Text(decode_text(data, self.jet4, self.encoding)?),
                ColumnType::Memo => Value::Text(decode_text(&self.long_value(data, work)?, self.jet4, self.encoding)?),
                ColumnType::Binary => Value::Binary(if col.length == 0 { self.long_value(data, work)? } else { data.to_vec() }),
            };
            *work = work.saturating_add(value_size(&value));
            if *work > MAX_WORK {
                return Err(Error::Limit("64 MiB decoded table"));
            }
            values.push(value);
        }
        Ok(values)
    }
    fn long_value(&self, data: &[u8], work: &mut usize) -> Result<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let header = u32_at(data, 0)?;
        let length = (header & 0x3fff_ffff) as usize;
        if length > MAX_STRING || work.saturating_add(length) > MAX_WORK {
            return Err(Error::Limit("long value"));
        }
        if header & 0x8000_0000 != 0 {
            return Ok(slice(data, 12, length)?.to_vec());
        }
        let mut pointer = u32_at(data, 4)?;
        let mut result = Vec::new();
        let mut visited = BTreeSet::new();
        while pointer != 0 && result.len() < length {
            if !visited.insert(pointer) {
                return Err(Error::Invalid("cyclic long value"));
            }
            let raw = self.resolve_row((pointer >> 8) as usize, (pointer & 255) as usize)?;
            let part = if header & 0x4000_0000 != 0 {
                pointer = 0;
                raw
            } else {
                pointer = u32_at(raw, 0)?;
                slice(raw, 4, raw.len().saturating_sub(4))?
            };
            if part.is_empty() || part.len() > length.saturating_sub(result.len()) {
                return Err(Error::Invalid("long value length"));
            }
            result.extend_from_slice(part);
        }
        if result.len() != length {
            return Err(Error::Invalid("truncated long value"));
        }
        Ok(result)
    }
}
fn value_size(value: &Value) -> usize {
    match value {
        Value::Text(s) | Value::Currency(s) | Value::Numeric(s) => s.len().saturating_add(32),
        Value::Binary(b) => b.len().saturating_add(32),
        _ => 32,
    }
}
fn slice(data: &[u8], offset: usize, length: usize) -> Result<&[u8]> {
    data.get(offset..offset.checked_add(length).ok_or(Error::Invalid("offset overflow"))?).ok_or(Error::Invalid("truncated data"))
}
fn array<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N]> {
    slice(data, offset, N)?.try_into().map_err(|_| Error::Invalid("scalar size"))
}
fn byte(data: &[u8], offset: usize) -> Result<u8> {
    data.get(offset).copied().ok_or(Error::Invalid("truncated byte"))
}
fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(array(data, offset)?))
}
fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(array(data, offset)?))
}
fn decode_text(data: &[u8], jet4: bool, encoding: &'static encoding_rs::Encoding) -> Result<String> {
    if data.len() > MAX_STRING {
        return Err(Error::Limit("string length"));
    }
    if !jet4 {
        let (text, _, errors) = encoding.decode(data);
        if errors {
            return Err(Error::Invalid("Jet 3 text encoding"));
        }
        return Ok(text.into_owned());
    }
    let compressed = data.starts_with(&[255, 254]);
    let mut units = Vec::new();
    let mut pos = if compressed { 2 } else { 0 };
    let mut single = compressed;
    while pos < data.len() {
        let b = byte(data, pos)?;
        if compressed && b == 0 {
            single = !single;
            pos += 1;
        } else if single {
            units.push(u16::from(b));
            pos += 1;
        } else {
            units.push(u16_at(data, pos)?);
            pos += 2;
        }
    }
    String::from_utf16(&units).map_err(|_| Error::Invalid("UTF-16 text"))
}
fn rc4(data: &mut [u8]) {
    let key = [0xc7u8, 0xda, 0x39, 0x6b];
    let mut state = [0u8; 256];
    for (i, v) in state.iter_mut().enumerate() {
        *v = i as u8;
    }
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + usize::from(state.get(i).copied().unwrap_or(0)) + usize::from(key.get(i % 4).copied().unwrap_or(0))) % 256;
        state.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    for b in data {
        i = (i + 1) % 256;
        j = (j + usize::from(state.get(i).copied().unwrap_or(0))) % 256;
        state.swap(i, j);
        let k = (usize::from(state.get(i).copied().unwrap_or(0)) + usize::from(state.get(j).copied().unwrap_or(0))) % 256;
        *b ^= state.get(k).copied().unwrap_or(0);
    }
}
