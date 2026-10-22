//! Sandboxed ASCII R2000 DXF output. Unicode input is escaped for pre-2007 readers.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::fmt::Write;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, thiserror::Error)]
#[error("Invalid DXF drawing: {0}")]
pub struct Error(&'static str);
const MAX_ENTITIES: usize = 100_000;
const MAX_VERTICES: usize = 1_000_000;
const MAX_TEXT: usize = 16_384;
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub enum Units {
    #[default]
    Unitless = 0,
    Inches = 1,
    Feet = 2,
    Millimeters = 4,
    Centimeters = 5,
    Meters = 6,
}
#[derive(Debug, Clone)]
pub struct Entity {
    pub layer: String,
    pub color: u16,
    geometry: Geometry,
}
#[derive(Debug, Clone)]
enum Geometry {
    Text { text: String, point: [f64; 3], height: f64, rotation: f64 },
    Point([f64; 3]),
    Polyline { points: Vec<[f64; 2]>, closed: bool, width: f64 },
    Line([f64; 3], [f64; 3]),
}
impl Entity {
    /// Like AutoCAD Rotate: the angle is in radians about the supplied base point.
    pub fn rotate(&mut self, base: [f64; 3], angle: f64) -> Result<()> {
        valid_point(&base)?;
        valid_number(angle)?;
        let rotate = |p: &mut [f64; 3]| {
            let (x, y) = (p[0] - base[0], p[1] - base[1]);
            p[0] = base[0] + x * angle.cos() - y * angle.sin();
            p[1] = base[1] + x * angle.sin() + y * angle.cos();
        };
        match &mut self.geometry {
            Geometry::Text { point, rotation, .. } => {
                rotate(point);
                *rotation += angle.to_degrees();
            }
            Geometry::Point(point) => rotate(point),
            Geometry::Line(a, b) => {
                rotate(a);
                rotate(b);
            }
            Geometry::Polyline { points, .. } => {
                for [x, y] in points {
                    let mut p = [*x, *y, 0.0];
                    rotate(&mut p);
                    *x = p[0];
                    *y = p[1];
                }
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
pub struct Drawing {
    pub units: Units,
    layers: BTreeMap<String, u16>,
    entities: Vec<Entity>,
    vertices: usize,
    text_bytes: usize,
}
impl Default for Drawing {
    fn default() -> Self {
        Self { units: Units::default(), layers: BTreeMap::from([("0".into(), 7)]), entities: Vec::new(), vertices: 0, text_bytes: 0 }
    }
}
impl Drawing {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }
    /// Insertion-order indices let bindings retain object handles across additions.
    pub fn entity_mut(&mut self, index: usize) -> Result<&mut Entity> {
        self.entities.get_mut(index).ok_or(Error("entity handle out of range"))
    }
    pub fn add_layer(&mut self, name: &str, color: u16) -> Result<()> {
        valid_name(name)?;
        if !(1..=255).contains(&color) {
            return Err(Error("layer color must be ACI 1..255"));
        }
        if self.layers.len() >= 1024 && !self.layers.contains_key(name) {
            return Err(Error("too many layers"));
        }
        if let Some(existing) = self.layers.keys().find(|n| n.eq_ignore_ascii_case(name)).cloned() {
            self.layers.insert(existing, color);
        } else {
            self.layers.insert(name.into(), color);
        }
        Ok(())
    }
    fn push(&mut self, geometry: Geometry) -> Result<&mut Entity> {
        if self.entities.len() >= MAX_ENTITIES {
            return Err(Error("too many entities"));
        }
        self.entities.push(Entity { layer: "0".into(), color: 256, geometry });
        self.entities.last_mut().ok_or(Error("missing entity"))
    }
    pub fn add_text(&mut self, text: &str, point: [f64; 3], height: f64) -> Result<&mut Entity> {
        valid_point(&point)?;
        valid_number(height)?;
        if height <= 0.0 || text.len() > MAX_TEXT || text.chars().any(char::is_control) {
            return Err(Error("invalid text or height"));
        }
        if self.text_bytes.saturating_add(text.len()) > MAX_OUTPUT || self.entities.len() >= MAX_ENTITIES {
            return Err(Error("drawing text/entity limit"));
        }
        self.text_bytes += text.len();
        self.push(Geometry::Text { text: text.into(), point, height, rotation: 0.0 })
    }
    pub fn add_point(&mut self, point: [f64; 3]) -> Result<&mut Entity> {
        valid_point(&point)?;
        self.push(Geometry::Point(point))
    }
    pub fn add_line(&mut self, start: [f64; 3], end: [f64; 3]) -> Result<&mut Entity> {
        valid_point(&start)?;
        valid_point(&end)?;
        self.push(Geometry::Line(start, end))
    }
    pub fn add_lwpolyline(&mut self, points: &[[f64; 2]], closed: bool, width: f64) -> Result<&mut Entity> {
        if points.len() < 2 || self.vertices.saturating_add(points.len()) > MAX_VERTICES {
            return Err(Error("invalid vertex count"));
        }
        valid_number(width)?;
        if width < 0.0 {
            return Err(Error("negative polyline width"));
        }
        for point in points {
            valid_point(point)?;
        }
        if self.entities.len() >= MAX_ENTITIES {
            return Err(Error("too many entities"));
        }
        self.vertices += points.len();
        self.push(Geometry::Polyline { points: points.to_vec(), closed, width })
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Output(String::new(), false);
        out.pair(0, "SECTION");
        out.pair(2, "HEADER");
        out.pair(9, "$ACADVER");
        out.pair(1, "AC1015");
        out.pair(9, "$DWGCODEPAGE");
        out.pair(3, "ANSI_1252");
        out.pair(9, "$INSUNITS");
        out.pair(70, self.units as u16);
        out.pair(9, "$HANDSEED");
        out.pair(5, format!("{:X}", 0x1000 + self.entities.len()));
        out.pair(0, "ENDSEC");
        out.pair(0, "SECTION");
        out.pair(2, "CLASSES");
        out.pair(0, "CLASS");
        out.pair(1, "ACDBPLACEHOLDER");
        out.pair(2, "AcDbPlaceHolder");
        out.pair(3, "GridCraft");
        out.pair(90, 0);
        out.pair(91, 1);
        out.pair(280, 0);
        out.pair(281, 0);
        out.pair(0, "ENDSEC");
        out.pair(0, "SECTION");
        out.pair(2, "TABLES");
        out.table("LTYPE", "1", 1);
        out.symbol("LTYPE", "2", "1", "AcDbLinetypeTableRecord");
        out.pair(2, "CONTINUOUS");
        out.pair(70, 0);
        out.pair(3, "Solid line");
        out.pair(72, 65);
        out.pair(73, 0);
        out.pair(40, 0);
        out.pair(0, "ENDTAB");
        out.table("LAYER", "3", self.layers.len());
        for (i, (name, color)) in self.layers.iter().enumerate() {
            valid_name(name)?;
            out.symbol("LAYER", &format!("{:X}", 0x100 + i), "3", "AcDbLayerTableRecord");
            out.pair(2, escape(name));
            out.pair(70, 0);
            out.pair(62, color);
            out.pair(6, "CONTINUOUS");
            out.pair(370, -3);
            out.pair(390, "F");
        }
        out.pair(0, "ENDTAB");
        out.table("STYLE", "4", 1);
        out.symbol("STYLE", "5", "4", "AcDbTextStyleTableRecord");
        out.pair(2, "STANDARD");
        out.pair(70, 0);
        out.pair(40, 0);
        out.pair(41, 1);
        out.pair(50, 0);
        out.pair(71, 0);
        out.pair(42, 2.5);
        out.pair(3, "DejaVuSans.ttf");
        out.pair(4, "");
        out.pair(0, "ENDTAB");
        out.table("BLOCK_RECORD", "6", 2);
        for (handle, name) in [("7", "*Model_Space"), ("8", "*Paper_Space")] {
            out.symbol("BLOCK_RECORD", handle, "6", "AcDbBlockTableRecord");
            out.pair(2, name);
            out.pair(70, 0);
        }
        out.pair(0, "ENDTAB");
        out.pair(0, "ENDSEC");
        out.pair(0, "SECTION");
        out.pair(2, "BLOCKS");
        for (owner, begin, end, name) in [("7", "9", "A", "*Model_Space"), ("8", "B", "C", "*Paper_Space")] {
            out.entity("BLOCK", begin, owner, "0", 256);
            out.pair(100, "AcDbBlockBegin");
            out.pair(2, name);
            out.pair(70, 0);
            out.point([0.0; 3], 10);
            out.pair(3, name);
            out.pair(1, "");
            out.entity("ENDBLK", end, owner, "0", 256);
            out.pair(100, "AcDbBlockEnd");
        }
        out.pair(0, "ENDSEC");
        out.pair(0, "SECTION");
        out.pair(2, "ENTITIES");
        for (i, entity) in self.entities.iter().enumerate() {
            valid_name(&entity.layer)?;
            if entity.color > 256 {
                return Err(Error("entity color must be ACI 0..256"));
            }
            let layer = self.layers.keys().find(|n| n.eq_ignore_ascii_case(&entity.layer)).ok_or(Error("entity layer does not exist"))?;
            let handle = format!("{:X}", 0x1000 + i);
            match &entity.geometry {
                Geometry::Text { text, point, height, rotation } => {
                    valid_point(point)?;
                    valid_number(*rotation)?;
                    out.entity("TEXT", &handle, "7", layer, entity.color);
                    out.pair(100, "AcDbText");
                    out.point(*point, 10);
                    out.pair(40, height);
                    out.pair(1, escape(text));
                    out.pair(50, rotation);
                    out.pair(41, 1);
                    out.pair(7, "STANDARD");
                    out.pair(72, 0);
                    out.pair(100, "AcDbText");
                    out.pair(73, 0);
                }
                Geometry::Point(point) => {
                    valid_point(point)?;
                    out.entity("POINT", &handle, "7", layer, entity.color);
                    out.pair(100, "AcDbPoint");
                    out.point(*point, 10);
                }
                Geometry::Line(a, b) => {
                    valid_point(a)?;
                    valid_point(b)?;
                    out.entity("LINE", &handle, "7", layer, entity.color);
                    out.pair(100, "AcDbLine");
                    out.point(*a, 10);
                    out.point(*b, 11);
                }
                Geometry::Polyline { points, closed, width } => {
                    out.entity("LWPOLYLINE", &handle, "7", layer, entity.color);
                    out.pair(100, "AcDbPolyline");
                    out.pair(90, points.len());
                    out.pair(70, u8::from(*closed));
                    out.pair(43, width);
                    for [x, y] in points {
                        valid_point(&[*x, *y])?;
                        out.pair(10, x);
                        out.pair(20, y);
                    }
                }
            }
            if out.0.len() > MAX_OUTPUT {
                return Err(Error("64 MiB output limit"));
            }
        }
        out.pair(0, "ENDSEC");
        out.pair(0, "SECTION");
        out.pair(2, "OBJECTS");
        out.pair(0, "DICTIONARY");
        out.pair(5, "D");
        out.pair(330, "0");
        out.pair(100, "AcDbDictionary");
        out.pair(281, 1);
        out.pair(3, "ACAD_PLOTSTYLENAME");
        out.pair(350, "E");
        out.pair(0, "DICTIONARY");
        out.pair(5, "E");
        out.pair(330, "D");
        out.pair(100, "AcDbDictionary");
        out.pair(281, 1);
        out.pair(3, "Normal");
        out.pair(350, "F");
        out.pair(0, "ACDBPLACEHOLDER");
        out.pair(5, "F");
        out.pair(330, "E");
        out.pair(100, "AcDbPlaceHolder");
        out.pair(0, "ENDSEC");
        out.pair(0, "EOF");
        if out.1 || out.0.len() > MAX_OUTPUT {
            return Err(Error("64 MiB output limit"));
        }
        Ok(out.0.into_bytes())
    }
}
struct Output(String, bool);
impl Output {
    fn pair(&mut self, code: u16, value: impl std::fmt::Display) {
        if self.0.len() > MAX_OUTPUT {
            self.1 = true;
            return;
        }
        let _ = writeln!(self.0, "{code}\r\n{value}\r");
    }
    fn table(&mut self, name: &str, handle: &str, count: usize) {
        self.pair(0, "TABLE");
        self.pair(2, name);
        self.pair(5, handle);
        self.pair(330, "0");
        self.pair(100, "AcDbSymbolTable");
        self.pair(70, count);
    }
    fn symbol(&mut self, kind: &str, handle: &str, owner: &str, subclass: &str) {
        self.pair(0, kind);
        self.pair(5, handle);
        self.pair(330, owner);
        self.pair(100, "AcDbSymbolTableRecord");
        self.pair(100, subclass);
    }
    fn entity(&mut self, kind: &str, handle: &str, owner: &str, layer: &str, color: u16) {
        self.pair(0, kind);
        self.pair(5, handle);
        self.pair(330, owner);
        self.pair(100, "AcDbEntity");
        self.pair(8, escape(layer));
        self.pair(62, color);
    }
    fn point(&mut self, [x, y, z]: [f64; 3], base: u16) {
        self.pair(base, x);
        self.pair(base + 10, y);
        self.pair(base + 20, z);
    }
}
fn valid_number(n: f64) -> Result<()> {
    if n.is_finite() && n.abs() <= 1e100 { Ok(()) } else { Err(Error("non-finite or excessive coordinate")) }
}
fn valid_point(point: &[f64]) -> Result<()> {
    for n in point {
        valid_number(*n)?;
    }
    Ok(())
}
fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 255 || name.chars().any(|c| c.is_control() || "<>/\\\":;?*|,=`".contains(c)) {
        Err(Error("invalid layer name"))
    } else {
        Ok(())
    }
}
fn escape(text: &str) -> String {
    let mut result = String::new();
    for unit in text.encode_utf16() {
        if (32..127).contains(&unit) && unit != u16::from(b'\\') {
            result.push(char::from_u32(u32::from(unit)).unwrap_or(' '));
        } else {
            let _ = write!(result, "\\U+{unit:04X}");
        }
    }
    result
}
