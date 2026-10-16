//! Embedded Unicode TrueType faces for text WinAnsiEncoding can't carry (Vietnamese, most scripts
//! outside Western Europe): a Type0 font over a CIDFontType2 descendant with Identity-H encoding
//! (two-byte glyph ids), the whole font file as FontFile2, and a ToUnicode map so the text stays
//! selectable and searchable.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::string::StringId;
use skrifa::{FontRef, GlyphId, MetadataProvider as _, Tag};

/// Faces a string can be set in: regular and bold. Italic text uses the upright face.
pub(crate) const SLOTS: usize = 2;

/// A regular and a bold TrueType font, embedded for text the standard fonts can't encode.
#[derive(Clone, Copy, Debug)]
pub struct UnicodeFonts {
    faces: [&'static [u8]; SLOTS],
}

impl UnicodeFonts {
    /// `None` unless both are TrueType (`glyf`) fonts with a character map: FontFile2 can only
    /// carry TrueType outlines.
    pub fn new(regular: &'static [u8], bold: &'static [u8]) -> Option<UnicodeFonts> {
        (Face::parse(regular).is_some() && Face::parse(bold).is_some()).then_some(UnicodeFonts { faces: [regular, bold] })
    }

    pub(crate) fn face(&self, slot: usize) -> Option<Face> {
        Face::parse(self.bytes(slot))
    }

    pub(crate) fn bytes(&self, slot: usize) -> &'static [u8] {
        self.faces.get(slot).copied().unwrap_or_default()
    }
}

static REGISTERED: OnceLock<UnicodeFonts> = OnceLock::new();

/// Makes `fonts` the Unicode faces of every later [`crate::PdfDoc::new`] and [`crate::text_width`].
/// Set once at startup; returns `false` (and keeps the first) if fonts were already registered.
pub fn register_unicode_fonts(fonts: UnicodeFonts) -> bool {
    REGISTERED.set(fonts).is_ok()
}

/// The registered Unicode faces, if any.
pub fn unicode_fonts() -> Option<UnicodeFonts> {
    REGISTERED.get().copied()
}

/// Whether WinAnsiEncoding would lose a character of `text`. Control characters don't count:
/// no font draws them.
pub(crate) fn needs_unicode(text: &str) -> bool {
    text.chars().any(|c| c != '?' && !c.is_control() && crate::win_ansi(c) == b'?')
}

/// One parsed face.
pub(crate) struct Face {
    font: FontRef<'static>,
    upem: f32,
}

impl Face {
    fn parse(bytes: &'static [u8]) -> Option<Face> {
        let font = FontRef::new(bytes).ok()?;
        let upem = f32::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
        (upem > 0.0 && font.charmap().has_map() && font.table_data(Tag::new(b"glyf")).is_some()).then_some(Face { font, upem })
    }

    fn metrics(&self) -> GlyphMetrics<'static> {
        self.font.glyph_metrics(Size::unscaled(), LocationRef::default())
    }

    /// The glyph id for each character; a missing glyph is 0 (.notdef). Tabs print as spaces, as
    /// they do in the standard fonts.
    pub(crate) fn glyphs<'t>(&self, text: &'t str) -> impl Iterator<Item = (char, u16)> + 't {
        let charmap = self.font.charmap();
        text.chars().map(move |c| {
            let gid = charmap.map(if c == '\t' { ' ' } else { c }).and_then(|g| u16::try_from(g.to_u32()).ok()).unwrap_or(0);
            (c, gid)
        })
    }

    /// Width of `text` in points at `size` points.
    pub(crate) fn text_width(&self, size: f32, text: &str) -> f32 {
        let metrics = self.metrics();
        let units: f64 = self.glyphs(text).map(|(_, g)| f64::from(metrics.advance_width(GlyphId::new(u32::from(g))).unwrap_or(0.0))).sum();
        (units * f64::from(size) / f64::from(self.upem)) as f32
    }

    /// A font-unit value in PDF glyph space (1/1000 em).
    fn em(&self, v: f32) -> i64 {
        let v = (v * 1000.0 / self.upem).round();
        if v.is_finite() { v.clamp(-1.0e6, 1.0e6) as i64 } else { 0 }
    }

    /// The font's PostScript name, reduced to characters a PDF name can hold unescaped.
    fn name(&self, slot: usize) -> String {
        let ps: String = self
            .font
            .localized_strings(StringId::POSTSCRIPT_NAME)
            .english_or_first()
            .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).take(63).collect())
            .unwrap_or_default();
        if ps.is_empty() { format!("GridCraftUnicode{slot}") } else { ps }
    }
}

/// Writes the Type0 font for `glyphs` (glyph id → the character it was drawn for) in face `slot`
/// and its parts, numbering objects from `next`. Returns the Type0 font's object number.
pub(crate) fn write_font(
    fonts: &UnicodeFonts,
    slot: usize,
    glyphs: &BTreeMap<u16, char>,
    next: &mut u32,
    objects: &mut Vec<(u32, Vec<u8>)>,
) -> Option<u32> {
    let face = fonts.face(slot)?;
    let bytes = fonts.bytes(slot);
    let [type0, cid, descriptor, file, to_unicode] = [0, 1, 2, 3, 4].map(|i| *next + i);
    *next += 5;
    let name = face.name(slot);
    objects.push((
        type0,
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{cid} 0 R] /ToUnicode {to_unicode} 0 R >>")
            .into_bytes(),
    ));

    let metrics = face.metrics();
    let mut widths = String::new();
    let mut run_end: Option<u16> = None;
    for &g in glyphs.keys() {
        let w = face.em(metrics.advance_width(GlyphId::new(u32::from(g))).unwrap_or(0.0));
        // Consecutive glyph ids share one `first [w1 w2 …]` entry.
        if run_end.is_some_and(|e| e.checked_add(1) == Some(g)) {
            let _ = write!(widths, " {w}");
        } else {
            if run_end.is_some() {
                widths.push_str("] ");
            }
            let _ = write!(widths, "{g} [{w}");
        }
        run_end = Some(g);
    }
    if run_end.is_some() {
        widths.push(']');
    }
    objects.push((
        cid,
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{widths}] >>"
        )
        .into_bytes(),
    ));

    let m = face.font.metrics(Size::unscaled(), LocationRef::default());
    let bbox = m
        .bounds
        .map_or([0, face.em(m.descent), 1000, face.em(m.ascent)], |b| [face.em(b.x_min), face.em(b.y_min), face.em(b.x_max), face.em(b.y_max)]);
    let italic = if m.italic_angle.is_finite() { m.italic_angle.round() as i64 } else { 0 };
    let stem = if slot == 1 { 140 } else { 80 };
    objects.push((
        descriptor,
        format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle {italic} /Ascent {} /Descent {} /CapHeight {} /StemV {stem} /FontFile2 {file} 0 R >>",
            bbox[0],
            bbox[1],
            bbox[2],
            bbox[3],
            face.em(m.ascent),
            face.em(m.descent),
            face.em(m.cap_height.unwrap_or(m.ascent)),
        )
        .into_bytes(),
    ));

    let mut o = format!("<< /Length {} /Length1 {} >>\nstream\n", bytes.len(), bytes.len()).into_bytes();
    o.extend_from_slice(bytes);
    o.extend_from_slice(b"\nendstream");
    objects.push((file, o));

    let cmap = to_unicode_cmap(glyphs);
    let mut o = format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes();
    o.extend_from_slice(cmap.as_bytes());
    o.extend_from_slice(b"endstream");
    objects.push((to_unicode, o));
    Some(type0)
}

/// The ToUnicode CMap: each glyph id back to its character (UTF-16BE). .notdef maps to nothing,
/// since every missing character shares it.
fn to_unicode_cmap(glyphs: &BTreeMap<u16, char>) -> String {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(u16, char)> = glyphs.iter().filter(|(g, _)| **g != 0).map(|(g, c)| (*g, *c)).collect();
    // A bfchar block holds at most 100 entries.
    for chunk in entries.chunks(100) {
        let _ = writeln!(s, "{} beginbfchar", chunk.len());
        for (g, c) in chunk {
            let _ = write!(s, "<{g:04X}> <");
            for unit in c.encode_utf16(&mut [0; 2]).iter() {
                let _ = write!(s, "{unit:04X}");
            }
            s.push_str(">\n");
        }
        s.push_str("endbfchar\n");
    }
    s.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    s
}
