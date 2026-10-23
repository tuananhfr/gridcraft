//! In-memory replacements for a narrow DAO/AutoCAD automation surface.
mod query;
use super::host::arg;
use gridcraft_vbarun::{Argument, Error, Object, Result, Value};
use std::{collections::BTreeMap, sync::Arc};

pub(super) const BASE: u64 = 1 << 63;
const MEMORY: usize = 32 * 1024 * 1024;
#[derive(Clone)]
enum Handle {
    Dao,
    Workspaces,
    Database(usize),
    Tables(usize),
    Table(String),
    Recordset(usize),
    Fields(usize),
    Field(usize, usize),
    Acad,
    Documents,
    Document,
    ModelSpace,
    Layers,
    Layer(String),
    Entity(usize),
}
struct Database {
    bytes: Arc<Vec<u8>>,
    closed: bool,
}
struct Recordset {
    database: usize,
    names: Vec<String>,
    rows: Vec<Vec<Value>>,
    cursor: i64,
    closed: bool,
}
#[derive(Default)]
pub(super) struct Emulation {
    handles: Vec<Handle>,
    databases: Vec<Database>,
    picked: BTreeMap<String, Arc<Vec<u8>>>,
    recordsets: Vec<Recordset>,
    drawing: Option<gridcraft_dxf::Drawing>,
    layers: BTreeMap<String, u16>,
    active_layer: String,
    visible: bool,
    memory: usize,
}
fn invalid(message: impl Into<String>) -> Error {
    Error::runtime(1004, message)
}
fn unsupported(name: &str) -> Error {
    Error::unsupported(&format!("DAO/AutoCAD.{name}"))
}
fn text(args: &[Argument], i: usize, name: &str) -> Result<String> {
    arg(args, i, name).ok_or_else(|| invalid(format!("Missing {name}")))?.text()
}
fn number(args: &[Argument], i: usize, name: &str) -> Result<f64> {
    let n = arg(args, i, name).ok_or_else(|| invalid(format!("Missing {name}")))?.number()?;
    if n.is_finite() { Ok(n) } else { Err(invalid("Invalid coordinate")) }
}
fn point(args: &[Argument], i: usize, name: &str) -> Result<[f64; 3]> {
    let values = array(args, i, name)?;
    if values.len() != 3 {
        return Err(invalid("AutoCAD point requires three coordinates"));
    }
    Ok([
        values.first().ok_or_else(|| invalid("Missing X"))?.number()?,
        values.get(1).ok_or_else(|| invalid("Missing Y"))?.number()?,
        values.get(2).ok_or_else(|| invalid("Missing Z"))?.number()?,
    ])
}
fn array<'a>(args: &'a [Argument], i: usize, name: &str) -> Result<&'a [Value]> {
    match arg(args, i, name) {
        Some(Value::Array(a)) if a.bounds.len() == 1 => Ok(&a.values),
        _ => Err(invalid("AutoCAD requires a one-dimensional coordinate array")),
    }
}
fn dxf(error: gridcraft_dxf::Error) -> Error {
    invalid(error.to_string())
}
impl Emulation {
    fn object(&mut self, handle: Handle) -> Result<Value> {
        if self.handles.len() >= 100_000 {
            return Err(Error { fatal: true, ..invalid("VBA sandbox limit: emulation handles") });
        }
        let name_bytes = match &handle {
            Handle::Table(n) | Handle::Layer(n) => n.len(),
            _ => 0,
        };
        self.charge(std::mem::size_of::<Handle>().saturating_add(name_bytes))?;
        let id = BASE + self.handles.len() as u64;
        self.handles.push(handle);
        Ok(Value::Object(Some(id)))
    }
    fn resolve(&self, id: Object) -> Result<Handle> {
        id.checked_sub(BASE)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| self.handles.get(i))
            .cloned()
            .ok_or_else(|| invalid("Invalid emulated object"))
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.memory = self.memory.saturating_add(bytes);
        if self.memory > MEMORY {
            return Err(Error { fatal: true, ..invalid("VBA sandbox limit: emulation memory") });
        }
        Ok(())
    }
    pub fn create(&mut self, existing: bool, args: &[Argument]) -> Result<Value> {
        let id = text(args, usize::from(existing), "Class")?.to_ascii_lowercase();
        if id == "autocad.application" {
            if existing && arg(args, 0, "PathName").is_some_and(|v| v.text().is_ok_and(|s| !s.is_empty())) {
                return Err(unsupported("GetObject(path)"));
            }
            if self.drawing.is_none() {
                self.drawing = Some(gridcraft_dxf::Drawing::new());
                self.layers.insert("0".into(), 7);
                self.active_layer = "0".into();
            }
            return self.object(Handle::Acad);
        }
        if !existing
            && (id == "dao.dbengine" || id.strip_prefix("dao.dbengine.").is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())))
        {
            return self.object(Handle::Dao);
        }
        Err(unsupported(&id))
    }
    pub fn dao(&mut self) -> Result<Value> {
        self.object(Handle::Dao)
    }
    pub fn opens_database(&self, id: Object, name: &str) -> bool {
        name.eq_ignore_ascii_case("opendatabase") && matches!(self.resolve(id), Ok(Handle::Dao))
    }
    pub fn cached(&self, path: &str) -> Option<Arc<Vec<u8>>> {
        self.picked.get(path).cloned()
    }
    pub fn open(&mut self, path: String, bytes: Arc<Vec<u8>>) -> Result<Value> {
        let bytes = self.picked.values().find(|v| v.as_slice() == bytes.as_slice()).cloned().unwrap_or(bytes);
        gridcraft_mdb::Database::open(&bytes).map_err(|e| invalid(e.to_string()))?;
        if !self.picked.values().any(|v| Arc::ptr_eq(v, &bytes)) {
            self.charge(bytes.len())?;
        }
        self.charge(if self.picked.contains_key(&path) { 64 } else { path.len().saturating_add(64) })?;
        self.picked.insert(path, bytes.clone());
        let i = self.databases.len();
        self.databases.push(Database { bytes, closed: false });
        self.object(Handle::Database(i))
    }
    pub fn drawing_bytes(&self) -> Result<Option<Vec<u8>>> {
        self.drawing.as_ref().filter(|d| !d.entities().is_empty()).map(|d| d.to_bytes().map_err(dxf)).transpose()
    }
    fn database(&self, i: usize) -> Result<&Database> {
        self.databases.get(i).filter(|d| !d.closed).ok_or_else(|| invalid("DAO database is closed"))
    }
    fn recordset(&self, i: usize) -> Result<&Recordset> {
        let rs = self.recordsets.get(i).filter(|r| !r.closed).ok_or_else(|| invalid("DAO recordset is closed"))?;
        self.database(rs.database)?;
        Ok(rs)
    }
    fn field(&mut self, i: usize, args: &[Argument]) -> Result<Value> {
        let rs = self.recordset(i)?;
        let key = arg(args, 0, "Index").ok_or_else(|| invalid("DAO field requires a name or index"))?;
        let col = match key {
            Value::String(s) => rs.names.iter().position(|n| n.eq_ignore_ascii_case(s)),
            _ => usize::try_from(key.integer()?).ok().filter(|i| *i < rs.names.len()),
        }
        .ok_or_else(|| invalid("DAO field not found"))?;
        self.object(Handle::Field(i, col))
    }
    pub fn scalar(&self, id: Object) -> Result<Value> {
        if let Handle::Field(i, col) = self.resolve(id)? {
            let rs = self.recordset(i)?;
            return usize::try_from(rs.cursor)
                .ok()
                .and_then(|r| rs.rows.get(r))
                .and_then(|r| r.get(col))
                .cloned()
                .ok_or_else(|| invalid("DAO: no current record"));
        }
        Ok(Value::Object(Some(id)))
    }
    pub fn get(&mut self, id: Object, name: &str, args: &[Argument]) -> Result<Value> {
        let name = name.to_ascii_lowercase();
        let handle = self.resolve(id)?;
        if matches!(handle, Handle::Dao | Handle::Database(_) | Handle::Recordset(_) | Handle::Fields(_) | Handle::Field(..))
            && matches!(name.as_str(), "edit" | "addnew" | "update" | "execute" | "delete")
        {
            return Err(invalid("DAO write operations are not supported"));
        }
        match handle {
            Handle::Dao => match name.as_str() {
                "workspaces" => {
                    if args.is_empty() {
                        self.object(Handle::Workspaces)
                    } else {
                        self.workspace(args)
                    }
                }
                _ => Err(unsupported(&name)),
            },
            Handle::Workspaces if matches!(name.as_str(), "" | "item") => self.workspace(args),
            Handle::Database(i) => {
                self.database(i)?;
                match name.as_str() {
                    "close" => {
                        self.databases.get_mut(i).ok_or_else(|| invalid("Invalid database"))?.closed = true;
                        Ok(Value::Empty)
                    }
                    "tabledefs" => {
                        let value = self.object(Handle::Tables(i))?;
                        if args.is_empty() {
                            Ok(value)
                        } else if let Value::Object(Some(id)) = value {
                            self.get(id, "item", args)
                        } else {
                            Err(invalid("Invalid tables"))
                        }
                    }
                    "openrecordset" => {
                        let sql = text(args, 0, "Name")?;
                        let query = query::Query::parse(&sql)?;
                        let bytes = self.database(i)?.bytes.clone();
                        let db = gridcraft_mdb::Database::open(&bytes).map_err(|e| invalid(e.to_string()))?;
                        let table = db.table(&query.table).map_err(|e| invalid(e.to_string()))?;
                        let names: Vec<_> = table.columns().iter().map(|c| c.name.clone()).collect();
                        let mut rows = Vec::new();
                        for row in table.rows() {
                            let row = row.map_err(|e| invalid(e.to_string()))?.into_iter().map(from_mdb).collect::<Result<Vec<_>>>()?;
                            self.charge(row.iter().fold(0usize, |n, v| n.saturating_add(v.memory())))?;
                            if rows.len() >= 100_000 {
                                return Err(invalid("DAO record limit"));
                            }
                            rows.push(row);
                        }
                        let (names, rows) = query.apply(&names, rows)?;
                        self.charge(rows.iter().flatten().fold(0usize, |n, v| n.saturating_add(v.memory())))?;
                        self.charge(names.iter().fold(0usize, |n, name| n.saturating_add(name.len()).saturating_add(32)))?;
                        let r = self.recordsets.len();
                        self.recordsets.push(Recordset { database: i, names, rows, cursor: 0, closed: false });
                        self.object(Handle::Recordset(r))
                    }
                    _ => Err(unsupported(&name)),
                }
            }
            Handle::Tables(i) => {
                let db = gridcraft_mdb::Database::open(&self.database(i)?.bytes).map_err(|e| invalid(e.to_string()))?;
                if name == "count" {
                    return Ok(Value::Long(db.tables().len() as i32));
                }
                if !matches!(name.as_str(), "" | "item") {
                    return Err(unsupported(&name));
                }
                let key = arg(args, 0, "Index").ok_or_else(|| invalid("Missing table index"))?;
                let table = match key {
                    Value::String(s) => db.tables().iter().find(|t| t.name.eq_ignore_ascii_case(s)),
                    _ => usize::try_from(key.integer()?).ok().and_then(|i| db.tables().get(i)),
                }
                .ok_or_else(|| invalid("DAO table not found"))?;
                let name = table.name.clone();
                self.object(Handle::Table(name))
            }
            Handle::Table(n) if name == "name" => Ok(Value::String(n)),
            Handle::Recordset(i) => {
                let rs = self.recordset(i)?;
                match name.as_str() {
                    "eof" => Ok(Value::Boolean(rs.rows.is_empty() || rs.cursor >= rs.rows.len() as i64)),
                    "bof" => Ok(Value::Boolean(rs.rows.is_empty() || rs.cursor < 0)),
                    "recordcount" => Ok(Value::Long(rs.rows.len() as i32)),
                    "fields" => {
                        if args.is_empty() {
                            self.object(Handle::Fields(i))
                        } else {
                            self.field(i, args)
                        }
                    }
                    "" | "item" => self.field(i, args),
                    "close" | "movefirst" | "movelast" | "movenext" | "moveprevious" => {
                        if name != "close"
                            && (rs.rows.is_empty()
                                || (name == "movenext" && rs.cursor >= rs.rows.len() as i64)
                                || (name == "moveprevious" && rs.cursor < 0))
                        {
                            return Err(Error::runtime(3021, "DAO: no current record"));
                        }
                        let rs = self.recordsets.get_mut(i).ok_or_else(|| invalid("Invalid recordset"))?;
                        match name.as_str() {
                            "close" => rs.closed = true,
                            "movefirst" => rs.cursor = 0,
                            "movelast" => rs.cursor = (rs.rows.len() as i64).saturating_sub(1),
                            "movenext" => rs.cursor = rs.cursor.saturating_add(1).min(rs.rows.len() as i64),
                            _ => rs.cursor = rs.cursor.saturating_sub(1).max(-1),
                        }
                        Ok(Value::Empty)
                    }
                    _ => Err(unsupported(&name)),
                }
            }
            Handle::Fields(i) => {
                if name == "count" {
                    Ok(Value::Long(self.recordset(i)?.names.len() as i32))
                } else if matches!(name.as_str(), "" | "item") {
                    self.field(i, args)
                } else {
                    Err(unsupported(&name))
                }
            }
            Handle::Field(i, c) => match name.as_str() {
                "name" => Ok(Value::String(self.recordset(i)?.names.get(c).cloned().ok_or_else(|| invalid("Invalid field"))?)),
                "value" | "" => self.scalar(id),
                _ => Err(unsupported(&name)),
            },
            Handle::Acad => match name.as_str() {
                "activedocument" => self.object(Handle::Document),
                "documents" => self.object(Handle::Documents),
                "visible" => Ok(Value::Boolean(self.visible)),
                "caption" => Ok(Value::String("GridCraft DXF".into())),
                "zoomextents" | "zoomall" | "update" => Ok(Value::Empty),
                _ => Err(unsupported(&name)),
            },
            Handle::Documents => match name.as_str() {
                "add" | "item" | "" => self.object(Handle::Document),
                "count" => Ok(Value::Long(1)),
                _ => Err(unsupported(&name)),
            },
            Handle::Document => match name.as_str() {
                "modelspace" => self.object(Handle::ModelSpace),
                "layers" => self.object(Handle::Layers),
                "activelayer" => self.object(Handle::Layer(self.active_layer.clone())),
                "update" | "regen" => Ok(Value::Empty),
                _ => Err(unsupported(&name)),
            },
            Handle::Layers => match name.as_str() {
                "count" => Ok(Value::Long(self.layers.len() as i32)),
                "add" => {
                    let n = text(args, 0, "Name")?;
                    if let Some(existing) = self.layers.keys().find(|s| s.eq_ignore_ascii_case(&n)).cloned() {
                        return self.object(Handle::Layer(existing));
                    }
                    self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?.add_layer(&n, 7).map_err(dxf)?;
                    self.layers.entry(n.clone()).or_insert(7);
                    self.object(Handle::Layer(n))
                }
                "item" | "" => {
                    let n = text(args, 0, "Index")?;
                    let n = self.layers.keys().find(|s| s.eq_ignore_ascii_case(&n)).cloned().ok_or_else(|| invalid("Layer not found"))?;
                    self.object(Handle::Layer(n))
                }
                _ => Err(unsupported(&name)),
            },
            Handle::Layer(n) => match name.as_str() {
                "name" => Ok(Value::String(n)),
                "color" => Ok(Value::Long(i32::from(*self.layers.get(&n).ok_or_else(|| invalid("Layer not found"))?))),
                "update" => Ok(Value::Empty),
                _ => Err(unsupported(&name)),
            },
            Handle::Entity(i) => match name.as_str() {
                "update" => Ok(Value::Empty),
                "rotate" => {
                    let base = point(args, 0, "BasePoint")?;
                    let angle = number(args, 1, "RotationAngle")?;
                    self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?.entity_mut(i).map_err(dxf)?.rotate(base, angle).map_err(dxf)?;
                    Ok(Value::Empty)
                }
                "layer" | "color" => {
                    let e = self.drawing.as_ref().and_then(|d| d.entities().get(i)).ok_or_else(|| invalid("Entity not found"))?;
                    Ok(if name == "layer" { Value::String(e.layer.clone()) } else { Value::Long(i32::from(e.color)) })
                }
                _ => Err(unsupported(&name)),
            },
            Handle::ModelSpace => self.add_entity(&name, args),
            _ => Err(unsupported(&name)),
        }
    }
    pub fn copy_rows(&mut self, id: Object, max_rows: usize, max_cols: usize) -> Result<Vec<Vec<Value>>> {
        let Handle::Recordset(i) = self.resolve(id)? else { return Err(invalid("Expected a DAO recordset")) };
        let rs = self.recordset(i)?;
        let start = usize::try_from(rs.cursor).map_err(|_| invalid("DAO: no current record"))?;
        let cols = rs.names.len().min(max_cols);
        let count = rs.rows.len().saturating_sub(start).min(max_rows);
        if count.saturating_mul(cols) > 100_000 {
            return Err(invalid("VBA sandbox limit: recordset cells"));
        }
        let rows = rs.rows.iter().skip(start).take(count).map(|row| row.iter().take(cols).cloned().collect()).collect();
        self.recordsets.get_mut(i).ok_or_else(|| invalid("Invalid recordset"))?.cursor = start.saturating_add(count) as i64;
        Ok(rows)
    }
    fn workspace(&mut self, args: &[Argument]) -> Result<Value> {
        if arg(args, 0, "Index").map(Value::integer).transpose()?.unwrap_or(0) != 0 {
            return Err(invalid("DAO workspace not found"));
        }
        self.dao()
    }
    fn add_entity(&mut self, name: &str, args: &[Argument]) -> Result<Value> {
        self.charge(args.iter().fold(256usize, |n, a| n.saturating_add(a.value.memory())))?;
        let drawing = self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?;
        let i = drawing.entities().len();
        let entity = match name {
            "addline" => drawing.add_line(point(args, 0, "StartPoint")?, point(args, 1, "EndPoint")?),
            "addpoint" => drawing.add_point(point(args, 0, "Point")?),
            "addcircle" => drawing.add_circle(point(args, 0, "Center")?, number(args, 1, "Radius")?),
            "addarc" => {
                drawing.add_arc(point(args, 0, "Center")?, number(args, 1, "Radius")?, number(args, 2, "StartAngle")?, number(args, 3, "EndAngle")?)
            }
            "addtext" => drawing.add_text(&text(args, 0, "TextString")?, point(args, 1, "InsertionPoint")?, number(args, 2, "Height")?),
            "addmtext" => drawing.add_mtext(&text(args, 2, "Text")?, point(args, 0, "InsertionPoint")?, number(args, 1, "Width")?),
            "addlightweightpolyline" | "addpolyline" => {
                let values = array(args, 0, "VerticesList")?;
                let dimensions = if name == "addpolyline" { 3 } else { 2 };
                if values.len() % dimensions != 0 || values.len() > 300_000 {
                    return Err(invalid("Invalid AutoCAD polyline vertices"));
                }
                if dimensions == 3 {
                    let points = values
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|v| {
                            Ok([
                                v.first().ok_or_else(|| invalid("Missing X"))?.number()?,
                                v.get(1).ok_or_else(|| invalid("Missing Y"))?.number()?,
                                v.get(2).ok_or_else(|| invalid("Missing Z"))?.number()?,
                            ])
                        })
                        .collect::<Result<Vec<_>>>()?;
                    drawing.add_polyline(&points)
                } else {
                    let points = values
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|v| {
                            Ok([v.first().ok_or_else(|| invalid("Missing X"))?.number()?, v.get(1).ok_or_else(|| invalid("Missing Y"))?.number()?])
                        })
                        .collect::<Result<Vec<_>>>()?;
                    drawing.add_lwpolyline(&points, false, 0.0)
                }
            }
            _ => return Err(unsupported(name)),
        }
        .map_err(dxf)?;
        entity.layer = self.active_layer.clone();
        self.object(Handle::Entity(i))
    }
    pub fn set(&mut self, id: Object, name: &str, value: Value) -> Result<()> {
        let name = name.to_ascii_lowercase();
        match (self.resolve(id)?, name.as_str()) {
            (Handle::Acad, "visible") => self.visible = value.truth()?,
            (Handle::Document, "activelayer") => {
                let Value::Object(Some(id)) = value else { return Err(invalid("Expected a layer")) };
                let Handle::Layer(n) = self.resolve(id)? else { return Err(invalid("Expected a layer")) };
                self.active_layer = n;
            }
            (Handle::Layer(n), "color") => {
                let color = u16::try_from(value.integer()?).map_err(|_| invalid("Invalid color"))?;
                self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?.add_layer(&n, color).map_err(dxf)?;
                self.layers.insert(n, color);
            }
            (Handle::Entity(i), "layer") => {
                let n = value.text()?;
                let n = self.layers.keys().find(|s| s.eq_ignore_ascii_case(&n)).cloned().ok_or_else(|| invalid("Layer not found"))?;
                self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?.entity_mut(i).map_err(dxf)?.layer = n;
            }
            (Handle::Entity(i), "color") => {
                let color = u16::try_from(value.integer()?).ok().filter(|n| *n <= 256).ok_or_else(|| invalid("Invalid color"))?;
                self.drawing.as_mut().ok_or_else(|| invalid("No drawing"))?.entity_mut(i).map_err(dxf)?.color = color;
            }
            (Handle::Field(..) | Handle::Fields(_) | Handle::Recordset(_) | Handle::Database(_), _) => {
                return Err(invalid("DAO write operations are not supported"));
            }
            _ => return Err(unsupported(&name)),
        }
        Ok(())
    }
    pub fn enumerate(&mut self, id: Object) -> Result<Vec<Value>> {
        let handles = match self.resolve(id)? {
            Handle::Tables(i) => {
                let db = gridcraft_mdb::Database::open(&self.database(i)?.bytes).map_err(|e| invalid(e.to_string()))?;
                db.tables().iter().map(|t| Handle::Table(t.name.clone())).collect::<Vec<_>>()
            }
            Handle::Fields(i) => (0..self.recordset(i)?.names.len()).map(|c| Handle::Field(i, c)).collect(),
            _ => return Err(unsupported("For Each")),
        };
        handles.into_iter().map(|h| self.object(h)).collect()
    }
}
fn from_mdb(v: gridcraft_mdb::Value) -> Result<Value> {
    use gridcraft_mdb::Value as M;
    Ok(match v {
        M::Null => Value::Null,
        M::Bool(v) => Value::Boolean(v),
        M::Byte(v) => Value::Byte(v),
        M::Int(v) => Value::Integer(v),
        M::Long(v) => Value::Long(v),
        M::Single(v) => Value::Single(v),
        M::Double(v) => Value::Double(v),
        M::DateTime(v) => Value::Date(v),
        M::Text(v) => Value::String(v),
        M::Currency(v) | M::Numeric(v) => Value::Double(v.parse().map_err(|_| invalid("Invalid DAO number"))?),
        M::Binary(_) => return Err(unsupported("binary field")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macro_emulation_metadata_handles_obey_memory_budget() {
        let mut emulation = Emulation::default();
        for _ in 0..16 {
            if let Err(error) = emulation.object(Handle::Table("T".repeat(4 * 1024 * 1024))) {
                assert!(error.fatal);
                assert!(error.description.contains("emulation memory"));
                return;
            }
        }
        panic!("metadata handles bypassed the emulation memory budget");
    }
}
