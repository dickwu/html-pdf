//! Overlay text and vector marks onto the pages of an existing PDF without
//! touching the original page content (the government-form use case).

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use ttf_parser::{Face, Permissions, PlatformId};

use crate::base14::{HELVETICA, HELVETICA_BOLD};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Font {
    Helvetica,
    HelveticaBold,
    /// A TrueType font added with [`Stamper::add_font`], by the order it was
    /// added in.
    Embedded(usize),
}

impl Font {
    /// A built-in font by name. Fonts added with [`Stamper::add_font`] are
    /// looked up with [`Stamper::font`].
    pub fn parse(name: &str) -> Result<Self, String> {
        match name
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '_'], "-")
            .as_str()
        {
            "" | "helvetica" => Ok(Self::Helvetica),
            "helvetica-bold" | "bold" => Ok(Self::HelveticaBold),
            other => Err(format!(
                "unknown font '{other}': use Helvetica, Helvetica-Bold or a font added with addFont()"
            )),
        }
    }

    /// `/BaseFont` of a built-in font; `None` for an added one.
    fn base14(self) -> Option<&'static str> {
        match self {
            Self::Helvetica => Some("Helvetica"),
            Self::HelveticaBold => Some("Helvetica-Bold"),
            Self::Embedded(_) => None,
        }
    }

    /// Resource name used inside the page's /Font dictionary.
    fn resource_name(self) -> String {
        match self {
            Self::Helvetica => "IPStampH".to_string(),
            Self::HelveticaBold => "IPStampHB".to_string(),
            Self::Embedded(index) => format!("IPStampT{}", index + 1),
        }
    }
}

/// A TrueType font added with [`Stamper::add_font`]. The whole file is
/// embedded (no subsetting) as a simple font with `WinAnsiEncoding`, so text
/// set in it is encoded exactly like text in the built-in fonts.
#[derive(Debug, Clone)]
struct EmbeddedFont {
    /// The name `text()` refers to it by.
    name: String,
    /// The font's `PostScript` name, written as `/BaseFont`.
    base_font: String,
    /// Advance widths (1/1000 em) by `WinAnsiEncoding` byte, 0 where the font
    /// has no glyph.
    widths: [u16; 256],
    /// Whether the font has a glyph for each `WinAnsiEncoding` byte.
    covered: [bool; 256],
    flags: i64,
    bbox: [i64; 4],
    italic_angle: f32,
    ascent: i64,
    descent: i64,
    cap_height: i64,
    data: Vec<u8>,
}

impl EmbeddedFont {
    /// `index` is the font's place among the added fonts; it names a font
    /// that has no `PostScript` name of its own.
    fn parse(name: &str, data: Vec<u8>, index: usize) -> Result<Self, String> {
        if data.starts_with(b"ttcf") {
            return Err(format!(
                "font '{name}' is a font collection (.ttc): add a single .ttf"
            ));
        }
        let face =
            Face::parse(&data, 0).map_err(|err| format!("cannot parse font '{name}': {err}"))?;
        if face.tables().glyf.is_none() {
            return Err(format!(
                "font '{name}' has no TrueType outlines: CFF-based OpenType (.otf) fonts are not supported"
            ));
        }
        if face.permissions() == Some(Permissions::Restricted)
            || face
                .tables()
                .os2
                .is_some_and(|os2| !os2.is_outline_embedding_allowed())
        {
            return Err(format!(
                "font '{name}' does not permit embedding its outlines (OS/2 fsType)"
            ));
        }
        // Viewers find a nonsymbolic TrueType font's glyphs through its
        // Windows Unicode cmap; a font without one would print blank boxes.
        let windows_unicode = face.tables().cmap.is_some_and(|cmap| {
            cmap.subtables.into_iter().any(|subtable| {
                subtable.platform_id == PlatformId::Windows
                    && matches!(subtable.encoding_id, 1 | 10)
            })
        });
        if !windows_unicode {
            return Err(format!(
                "font '{name}' has no Windows Unicode cmap (3,1): PDF viewers could not find its glyphs"
            ));
        }

        let units = f64::from(face.units_per_em());
        let scale = |value: f64| em_thousandths(value, units);
        let mut widths = [0u16; 256];
        let mut covered = [false; 256];
        for byte in 0x20..=0xFF_u8 {
            let Some(glyph) = winansi_glyph_char(byte).and_then(|ch| face.glyph_index(ch)) else {
                continue;
            };
            let advance = f64::from(face.glyph_hor_advance(glyph).unwrap_or(0));
            widths[usize::from(byte)] = u16::try_from(scale(advance).clamp(0, 65_535)).unwrap_or(0);
            covered[usize::from(byte)] = true;
        }
        if !covered.contains(&true) {
            return Err(format!(
                "font '{name}' maps no WinAnsi characters (it has no Unicode cmap)"
            ));
        }

        let mut flags = 32; // Nonsymbolic: glyphs are found through the encoding.
        if face.is_monospaced() {
            flags |= 1;
        }
        if face.is_italic() {
            flags |= 64;
        }
        let bbox = face.global_bounding_box();
        let ascent = scale(f64::from(face.ascender()));
        Ok(Self {
            name: name.to_string(),
            base_font: postscript_name(&face)
                .unwrap_or_else(|| format!("IPStampFont{}", index + 1)),
            widths,
            covered,
            flags,
            bbox: [
                scale(f64::from(bbox.x_min)),
                scale(f64::from(bbox.y_min)),
                scale(f64::from(bbox.x_max)),
                scale(f64::from(bbox.y_max)),
            ],
            italic_angle: face.italic_angle(),
            ascent,
            descent: scale(f64::from(face.descender())),
            cap_height: face
                .capital_height()
                .map_or(ascent, |height| scale(f64::from(height))),
            data,
        })
    }

    /// Write the font program, its descriptor and the font dictionary; the
    /// font dictionary's id is returned.
    fn write(&self, doc: &mut Document) -> ObjectId {
        let name = || Object::Name(self.base_font.as_bytes().to_vec());
        let mut file = Stream::new(Dictionary::new(), self.data.clone());
        file.dict.set(
            "Length1",
            Object::Integer(i64::try_from(self.data.len()).unwrap_or(i64::MAX)),
        );
        let file_id = doc.add_object(file);

        let mut descriptor = Dictionary::new();
        descriptor.set("Type", Object::Name(b"FontDescriptor".to_vec()));
        descriptor.set("FontName", name());
        descriptor.set("Flags", Object::Integer(self.flags));
        descriptor.set(
            "FontBBox",
            Object::Array(
                self.bbox
                    .iter()
                    .map(|&value| Object::Integer(value))
                    .collect(),
            ),
        );
        descriptor.set("ItalicAngle", Object::Real(self.italic_angle));
        descriptor.set("Ascent", Object::Integer(self.ascent));
        descriptor.set("Descent", Object::Integer(self.descent));
        descriptor.set("CapHeight", Object::Integer(self.cap_height));
        descriptor.set("StemV", Object::Integer(80));
        descriptor.set("FontFile2", Object::Reference(file_id));
        let descriptor_id = doc.add_object(descriptor);

        let mut font = Dictionary::new();
        font.set("Type", Object::Name(b"Font".to_vec()));
        font.set("Subtype", Object::Name(b"TrueType".to_vec()));
        font.set("BaseFont", name());
        font.set("FirstChar", Object::Integer(32));
        font.set("LastChar", Object::Integer(255));
        font.set(
            "Widths",
            Object::Array(
                self.widths[32..]
                    .iter()
                    .map(|&width| Object::Integer(i64::from(width)))
                    .collect(),
            ),
        );
        font.set("FontDescriptor", Object::Reference(descriptor_id));
        font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        doc.add_object(font)
    }
}

/// A font-unit value in thousandths of an em, the unit of PDF font metrics.
#[allow(clippy::cast_possible_truncation)]
fn em_thousandths(value: f64, units_per_em: f64) -> i64 {
    (value * 1000.0 / units_per_em).round() as i64
}

/// The font's `PostScript` name (name ID 6), reduced to characters that are
/// safe in a PDF name.
fn postscript_name(face: &Face) -> Option<String> {
    face.names()
        .into_iter()
        .filter(|record| record.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .find_map(|record| record.to_string())
        .map(|name| {
            name.chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '+' | '.'))
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "" | "left" => Ok(Self::Left),
            "center" | "centre" => Ok(Self::Center),
            "right" => Ok(Self::Right),
            other => Err(format!(
                "unknown align '{other}': use left, center or right"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextOp {
    pub page: u32,
    pub x: f64,
    /// Baseline, measured from the top edge of the page.
    pub y_top: f64,
    pub text: String,
    pub size: f64,
    pub font: Font,
    pub align: Align,
    pub max_width: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CheckOp {
    pub page: u32,
    /// Top-left corner of the box, y measured from the top edge of the page.
    pub x: f64,
    pub y_top: f64,
    pub size: f64,
    pub stroke: f64,
}

#[derive(Debug, Clone)]
enum Op {
    Text {
        op: TextOp,
        encoded: Vec<u8>,
        size: f64,
    },
    Check(CheckOp),
}

impl Op {
    fn page(&self) -> u32 {
        match self {
            Self::Text { op, .. } => op.page,
            Self::Check(op) => op.page,
        }
    }
}

pub const MIN_FONT_SIZE: f64 = 5.0;
const MAX_COORD: f64 = 14_400.0;

/// A loaded PDF plus the fonts added to it and the pending overlay operations.
#[derive(Debug, Clone)]
pub struct Stamper {
    doc: Document,
    pages: BTreeMap<u32, ObjectId>,
    fonts: Vec<EmbeddedFont>,
    ops: Vec<Op>,
}

impl Stamper {
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() {
            return Err("PDF data must not be empty".to_string());
        }
        let doc = Document::load_mem(bytes).map_err(|err| format!("cannot parse PDF: {err}"))?;
        if doc.is_encrypted() {
            return Err("encrypted PDFs are not supported".to_string());
        }
        let pages = doc.get_pages();
        if pages.is_empty() {
            return Err("PDF has no pages".to_string());
        }
        Ok(Self {
            doc,
            pages,
            fonts: Vec::new(),
            ops: Vec::new(),
        })
    }

    /// Add a TrueType font (the bytes of a .ttf file) that `text()` can then
    /// use by `name`. Added fonts stay through [`Self::reset`].
    pub fn add_font(&mut self, name: &str, data: Vec<u8>) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("font name must not be empty".to_string());
        }
        if data.is_empty() {
            return Err("font data must not be empty".to_string());
        }
        if Font::parse(name).is_ok() {
            return Err(format!("'{name}' is the name of a built-in font"));
        }
        if self
            .fonts
            .iter()
            .any(|font| font.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("font '{name}' was already added"));
        }
        let font = EmbeddedFont::parse(name, data, self.fonts.len())?;
        self.fonts.push(font);
        Ok(())
    }

    /// The font `text()` uses for `name`: a font added with
    /// [`Self::add_font`] (names compare case-insensitively), else a
    /// built-in one.
    pub fn font(&self, name: &str) -> Result<Font, String> {
        let name = name.trim();
        match self
            .fonts
            .iter()
            .position(|font| font.name.eq_ignore_ascii_case(name))
        {
            Some(index) => Ok(Font::Embedded(index)),
            None => Font::parse(name),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// `MediaBox` width and height of a 1-based page.
    pub fn page_size(&self, page: u32) -> Result<(f64, f64), String> {
        let (x0, y0, x1, y1) = self.media_box(self.page_id(page)?)?;
        Ok((x1 - x0, y1 - y0))
    }

    pub fn text(&mut self, op: TextOp) -> Result<(), String> {
        self.page_id(op.page)?;
        check_coord("x", op.x)?;
        check_coord("y", op.y_top)?;
        if !op.size.is_finite() || op.size <= 0.0 || op.size > 500.0 {
            return Err("font size must be a positive number of points (<= 500)".to_string());
        }
        if op
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
        {
            return Err("max_width must be a positive number of points".to_string());
        }
        let encoded = encode_winansi(&op.text)?;
        if let Font::Embedded(index) = op.font {
            let font = self
                .fonts
                .get(index)
                .ok_or_else(|| format!("font #{index} was never added"))?;
            // encode_winansi writes one byte per character.
            if let Some(ch) = op
                .text
                .chars()
                .zip(&encoded)
                .find_map(|(ch, &byte)| (!font.covered[usize::from(byte)]).then_some(ch))
            {
                return Err(format!(
                    "character '{ch}' (U+{:04X}) has no glyph in font '{}'",
                    u32::from(ch),
                    font.name
                ));
            }
        }
        let mut size = op.size;
        if let Some(max_width) = op.max_width {
            let unit_width = text_width(&encoded, self.widths(op.font)?, 1.0);
            if unit_width * size > max_width {
                size = (max_width / unit_width).max(0.0);
                if size < MIN_FONT_SIZE {
                    return Err(format!(
                        "text '{}' does not fit in {max_width:.1}pt even at {MIN_FONT_SIZE}pt",
                        op.text
                    ));
                }
            }
        }
        self.ops.push(Op::Text { op, encoded, size });
        Ok(())
    }

    pub fn check(&mut self, op: CheckOp) -> Result<(), String> {
        self.page_id(op.page)?;
        check_coord("x", op.x)?;
        check_coord("y", op.y_top)?;
        if !op.size.is_finite() || op.size <= 0.0 || op.size > 500.0 {
            return Err("check size must be a positive number of points (<= 500)".to_string());
        }
        if !op.stroke.is_finite() || op.stroke <= 0.0 || op.stroke > 50.0 {
            return Err("stroke width must be a positive number of points (<= 50)".to_string());
        }
        self.ops.push(Op::Check(op));
        Ok(())
    }

    pub fn reset(&mut self) {
        self.ops.clear();
    }

    pub fn pending_ops(&self) -> usize {
        self.ops.len()
    }

    /// Apply the pending operations to a copy of the document and serialize it.
    pub fn to_pdf(&self) -> Result<Vec<u8>, String> {
        let mut doc = self.doc.clone();
        let mut by_page: BTreeMap<u32, Vec<&Op>> = BTreeMap::new();
        for op in &self.ops {
            by_page.entry(op.page()).or_default().push(op);
        }
        // One font object per font, shared by every page that uses it, so an
        // added font's program is embedded once.
        let mut font_ids: BTreeMap<Font, ObjectId> = BTreeMap::new();
        for (page, ops) in by_page {
            let page_id = self.page_id(page)?;
            let (x0, y0, _x1, y1) = self.media_box(page_id)?;
            let height = y1 - y0;
            let mut content = Content {
                operations: Vec::new(),
            };
            let mut fonts_used: Vec<Font> = Vec::new();
            // Close the wrapper opened by the leading "q" stream, then draw.
            content.operations.push(Operation::new("Q", vec![]));
            for op in ops {
                match op {
                    Op::Text { op, encoded, size } => {
                        if !fonts_used.contains(&op.font) {
                            fonts_used.push(op.font);
                        }
                        let width = text_width(encoded, self.widths(op.font)?, *size);
                        let x = match op.align {
                            Align::Left => op.x,
                            Align::Center => op.x - width / 2.0,
                            Align::Right => op.x - width,
                        };
                        push_text(
                            &mut content,
                            &op.font.resource_name(),
                            *size,
                            x0 + x,
                            y0 + height - op.y_top,
                            encoded,
                        );
                    }
                    Op::Check(op) => push_check(
                        &mut content,
                        x0 + op.x,
                        y0 + height - op.y_top,
                        op.size,
                        op.stroke,
                    ),
                }
            }
            let encoded = content
                .encode()
                .map_err(|err| format!("cannot encode overlay: {err}"))?;
            let lead_id = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
            let tail_id = doc.add_object(Stream::new(Dictionary::new(), encoded));
            for font in fonts_used {
                let font_id = if let Some(id) = font_ids.get(&font) {
                    *id
                } else {
                    let id = self.write_font(&mut doc, font)?;
                    font_ids.insert(font, id);
                    id
                };
                ensure_font(&mut doc, page_id, &font.resource_name(), font_id)?;
            }
            wrap_contents(&mut doc, page_id, lead_id, tail_id)?;
        }
        doc.compress();
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|err| format!("cannot write PDF: {err}"))?;
        Ok(out)
    }

    /// Advance widths (1/1000 em) by `WinAnsiEncoding` byte.
    fn widths(&self, font: Font) -> Result<&[u16; 256], String> {
        match font {
            Font::Helvetica => Ok(&HELVETICA),
            Font::HelveticaBold => Ok(&HELVETICA_BOLD),
            Font::Embedded(index) => self
                .fonts
                .get(index)
                .map(|font| &font.widths)
                .ok_or_else(|| format!("font #{index} was never added")),
        }
    }

    /// Add the font's objects to `doc`; returns the font dictionary's id.
    fn write_font(&self, doc: &mut Document, font: Font) -> Result<ObjectId, String> {
        if let Font::Embedded(index) = font {
            return self
                .fonts
                .get(index)
                .map(|embedded| embedded.write(doc))
                .ok_or_else(|| format!("font #{index} was never added"));
        }
        let mut dict = Dictionary::new();
        dict.set("Type", Object::Name(b"Font".to_vec()));
        dict.set("Subtype", Object::Name(b"Type1".to_vec()));
        dict.set(
            "BaseFont",
            Object::Name(font.base14().unwrap_or("Helvetica").as_bytes().to_vec()),
        );
        dict.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        Ok(doc.add_object(dict))
    }

    fn page_id(&self, page: u32) -> Result<ObjectId, String> {
        self.pages
            .get(&page)
            .copied()
            .ok_or_else(|| format!("page {page} is out of range (1..={})", self.pages.len()))
    }

    /// `MediaBox` of a page, walking up the Pages tree when inherited.
    fn media_box(&self, page_id: ObjectId) -> Result<(f64, f64, f64, f64), String> {
        let mut node = page_id;
        for _ in 0..64 {
            let dict = self
                .doc
                .get_dictionary(node)
                .map_err(|err| format!("page object: {err}"))?;
            if let Ok(object) = dict.get(b"MediaBox") {
                let object = match object {
                    Object::Reference(id) => {
                        self.doc.get_object(*id).map_err(|err| err.to_string())?
                    }
                    other => other,
                };
                let values = object
                    .as_array()
                    .map_err(|_| "MediaBox is not an array".to_string())?;
                if values.len() != 4 {
                    return Err("MediaBox must have 4 numbers".to_string());
                }
                let mut numbers = [0.0f64; 4];
                for (slot, value) in numbers.iter_mut().zip(values) {
                    *slot = f64::from(
                        value
                            .as_float()
                            .map_err(|_| "MediaBox entry is not a number".to_string())?,
                    );
                }
                let (x0, y0, x1, y1) = (
                    numbers[0].min(numbers[2]),
                    numbers[1].min(numbers[3]),
                    numbers[0].max(numbers[2]),
                    numbers[1].max(numbers[3]),
                );
                return Ok((x0, y0, x1, y1));
            }
            match dict.get(b"Parent").and_then(Object::as_reference) {
                Ok(parent) => node = parent,
                Err(_) => break,
            }
        }
        // PDF default when nothing declares a MediaBox: US Letter.
        Ok((0.0, 0.0, 612.0, 792.0))
    }
}

fn check_coord(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() || value.abs() > MAX_COORD {
        return Err(format!(
            "{name} must be a finite coordinate in points (|{name}| <= {MAX_COORD})"
        ));
    }
    Ok(())
}

fn push_text(content: &mut Content, resource: &str, size: f64, x: f64, y: f64, encoded: &[u8]) {
    let ops = &mut content.operations;
    ops.push(Operation::new("BT", vec![]));
    ops.push(Operation::new(
        "Tf",
        vec![Object::Name(resource.as_bytes().to_vec()), real(size)],
    ));
    ops.push(Operation::new("g", vec![real(0.0)]));
    ops.push(Operation::new(
        "Tm",
        vec![real(1.0), real(0.0), real(0.0), real(1.0), real(x), real(y)],
    ));
    ops.push(Operation::new(
        "Tj",
        vec![Object::String(encoded.to_vec(), StringFormat::Hexadecimal)],
    ));
    ops.push(Operation::new("ET", vec![]));
}

/// Vector tick inside a box whose top-left corner is (x, `y_top_pdf`) — y here is
/// already in PDF coordinates (the box's top edge).
fn push_check(content: &mut Content, x: f64, y_top_pdf: f64, size: f64, stroke: f64) {
    let ops = &mut content.operations;
    let y0 = y_top_pdf - size; // bottom edge in PDF coordinates
    ops.push(Operation::new("q", vec![]));
    ops.push(Operation::new("G", vec![real(0.0)]));
    ops.push(Operation::new("w", vec![real(stroke)]));
    ops.push(Operation::new("J", vec![Object::Integer(1)]));
    ops.push(Operation::new("j", vec![Object::Integer(1)]));
    ops.push(Operation::new(
        "m",
        vec![real(x + 0.18 * size), real(y0 + 0.52 * size)],
    ));
    ops.push(Operation::new(
        "l",
        vec![real(x + 0.42 * size), real(y0 + 0.22 * size)],
    ));
    ops.push(Operation::new(
        "l",
        vec![real(x + 0.86 * size), real(y0 + 0.86 * size)],
    ));
    ops.push(Operation::new("S", vec![]));
    ops.push(Operation::new("Q", vec![]));
}

#[allow(clippy::cast_possible_truncation)]
fn real(value: f64) -> Object {
    // Keep the content stream tidy: 3 decimals is well below 1/100 of a point.
    Object::Real(((value * 1000.0).round() / 1000.0) as f32)
}

/// Advance width in points of WinAnsi-encoded bytes at `size`, from a font's
/// width table (1/1000 em by byte).
pub fn text_width(encoded: &[u8], widths: &[u16; 256], size: f64) -> f64 {
    let units: f64 = encoded
        .iter()
        .map(|&b| f64::from(widths[usize::from(b)]))
        .sum();
    units * size / 1000.0
}

/// `WinAnsiEncoding` (cp1252) bytes 0x80-0x9F and the characters they encode.
const CP1252_HIGH: [(u8, char); 27] = [
    (0x80, '€'),
    (0x82, '‚'),
    (0x83, 'ƒ'),
    (0x84, '„'),
    (0x85, '…'),
    (0x86, '†'),
    (0x87, '‡'),
    (0x88, 'ˆ'),
    (0x89, '‰'),
    (0x8A, 'Š'),
    (0x8B, '‹'),
    (0x8C, 'Œ'),
    (0x8E, 'Ž'),
    (0x91, '‘'),
    (0x92, '’'),
    (0x93, '“'),
    (0x94, '”'),
    (0x95, '•'),
    (0x96, '–'),
    (0x97, '—'),
    (0x98, '˜'),
    (0x99, '™'),
    (0x9A, 'š'),
    (0x9B, '›'),
    (0x9C, 'œ'),
    (0x9E, 'ž'),
    (0x9F, 'Ÿ'),
];

/// Map a string to `WinAnsiEncoding` (cp1252) bytes. Anything outside that
/// repertoire, and any control character, is an error rather than a silent
/// substitution: the caller decides how to transliterate.
pub fn encode_winansi(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let byte = match ch {
            '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{FF}' => ch as u8,
            '\t' => 0x20,
            other => CP1252_HIGH
                .iter()
                .find_map(|&(byte, high)| (high == other).then_some(byte))
                .ok_or_else(|| {
                    format!(
                        "character '{other}' (U+{:04X}) cannot be written with a WinAnsi font",
                        u32::from(other)
                    )
                })?,
        };
        out.push(byte);
    }
    Ok(out)
}

/// The character whose glyph a `WinAnsiEncoding` byte draws. Bytes 0xA0 and
/// 0xAD are the encoding's second space and hyphen.
fn winansi_glyph_char(byte: u8) -> Option<char> {
    match byte {
        0xA0 => Some(' '),
        0xAD => Some('-'),
        0x20..=0x7E | 0xA1..=0xFF => Some(char::from(byte)),
        _ => CP1252_HIGH
            .iter()
            .find_map(|&(high, ch)| (high == byte).then_some(ch)),
    }
}

/// Make sure the page's /Resources /Font dictionary maps `resource` to the
/// font object `font_id`. Handles a direct resources dict, a referenced one,
/// and resources inherited from the Pages tree (copied down to the page so
/// sibling pages are untouched).
fn ensure_font(
    doc: &mut Document,
    page_id: ObjectId,
    resource: &str,
    font_id: ObjectId,
) -> Result<(), String> {
    // Locate (or create) the page-level resources dictionary.
    let resources_ref: Option<ObjectId> = {
        let page = doc
            .get_dictionary(page_id)
            .map_err(|err| format!("page object: {err}"))?;
        match page.get(b"Resources") {
            Ok(Object::Reference(id)) => Some(*id),
            Ok(Object::Dictionary(_)) => None,
            _ => {
                // Inherited: copy the nearest ancestor's resources onto the page.
                let inherited = inherited_resources(doc, page_id)?;
                let page = doc
                    .get_object_mut(page_id)
                    .and_then(Object::as_dict_mut)
                    .map_err(|err| format!("page object: {err}"))?;
                page.set("Resources", Object::Dictionary(inherited));
                None
            }
        }
    };

    let resources: &mut Dictionary = match resources_ref {
        Some(id) => doc
            .get_object_mut(id)
            .and_then(Object::as_dict_mut)
            .map_err(|err| format!("resources object: {err}"))?,
        None => doc
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|err| format!("page object: {err}"))?
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .map_err(|err| format!("resources dictionary: {err}"))?,
    };

    // /Font may itself be a reference to a shared dictionary.
    let fonts_ref = match resources.get(b"Font") {
        Ok(Object::Reference(id)) => Some(*id),
        Ok(Object::Dictionary(_)) => None,
        _ => {
            resources.set("Font", Object::Dictionary(Dictionary::new()));
            None
        }
    };
    let fonts = if let Some(id) = fonts_ref {
        doc.get_object_mut(id)
            .and_then(Object::as_dict_mut)
            .map_err(|err| format!("font dictionary: {err}"))?
    } else {
        resources
            .get_mut(b"Font")
            .and_then(Object::as_dict_mut)
            .map_err(|err| format!("font dictionary: {err}"))?
    };
    fonts.set(resource, Object::Reference(font_id));
    Ok(())
}

fn inherited_resources(doc: &Document, page_id: ObjectId) -> Result<Dictionary, String> {
    let mut node = page_id;
    for _ in 0..64 {
        let dict = doc
            .get_dictionary(node)
            .map_err(|err| format!("pages tree: {err}"))?;
        if node != page_id {
            match dict.get(b"Resources") {
                Ok(Object::Reference(id)) => {
                    return doc
                        .get_dictionary(*id)
                        .cloned()
                        .map_err(|err| format!("resources: {err}"));
                }
                Ok(Object::Dictionary(inner)) => return Ok(inner.clone()),
                _ => {}
            }
        }
        match dict.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => node = parent,
            Err(_) => break,
        }
    }
    Ok(Dictionary::new())
}

/// Contents := [lead, ...existing..., tail].
fn wrap_contents(
    doc: &mut Document,
    page_id: ObjectId,
    lead_id: ObjectId,
    tail_id: ObjectId,
) -> Result<(), String> {
    let existing: Vec<Object> = {
        let page = doc
            .get_dictionary(page_id)
            .map_err(|err| format!("page object: {err}"))?;
        match page.get(b"Contents") {
            Ok(Object::Reference(id)) => vec![Object::Reference(*id)],
            Ok(Object::Array(items)) => items.clone(),
            Ok(other) => {
                return Err(format!(
                    "unsupported page Contents entry: {}",
                    other.enum_variant()
                ));
            }
            Err(_) => Vec::new(),
        }
    };
    let mut contents = Vec::with_capacity(existing.len() + 2);
    contents.push(Object::Reference(lead_id));
    contents.extend(existing);
    contents.push(Object::Reference(tail_id));
    let page = doc
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|err| format!("page object: {err}"))?;
    page.set("Contents", Object::Array(contents));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        ironpress_core::html_to_pdf("<h1>Fixture</h1><p>One page.</p>").expect("fixture PDF")
    }

    fn text_op(page: u32, text: &str) -> TextOp {
        TextOp {
            page,
            x: 40.0,
            y_top: 100.0,
            text: text.to_string(),
            size: 9.0,
            font: Font::Helvetica,
            align: Align::Left,
            max_width: None,
        }
    }

    #[test]
    fn loads_pages_and_media_box() {
        let stamper = Stamper::load(&fixture()).expect("load");
        assert_eq!(stamper.page_count(), 1);
        let (width, height) = stamper.page_size(1).expect("size");
        assert!((width - 595.28).abs() < 0.5, "A4 width, got {width}");
        assert!((height - 841.89).abs() < 0.5, "A4 height, got {height}");
        assert!(stamper.page_size(2).is_err());
    }

    #[test]
    fn rejects_garbage_and_empty_input() {
        assert!(Stamper::load(b"").is_err());
        assert!(Stamper::load(b"not a pdf at all").is_err());
    }

    #[test]
    fn winansi_encoding_covers_latin1_and_cp1252_specials() {
        assert_eq!(encode_winansi("Ab 1").unwrap(), b"Ab 1".to_vec());
        assert_eq!(
            encode_winansi("é€–’").unwrap(),
            vec![0xE9, 0x80, 0x96, 0x92]
        );
        assert_eq!(encode_winansi("a\tb").unwrap(), b"a b".to_vec());
        let err = encode_winansi("ᐃᓄᒃ").unwrap_err();
        assert!(err.contains("U+1403"), "{err}");
        assert!(encode_winansi("line\nbreak").is_err());
    }

    #[test]
    fn widths_follow_the_afm_tables() {
        let encoded = encode_winansi("AAA").unwrap();
        let width = text_width(&encoded, &HELVETICA, 10.0);
        assert!((width - 20.01).abs() < 1e-6, "{width}");
        let bold = text_width(&encoded, &HELVETICA_BOLD, 10.0);
        assert!((bold - 21.66).abs() < 1e-6, "{bold}");
    }

    /// Mrs Saint Delafield (SIL Open Font License, tests/fixtures).
    const SCRIPT_TTF: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/MrsSaintDelafield-Regular.ttf"
    ));

    #[test]
    fn added_fonts_are_embedded_whole_as_winansi_truetype() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        stamper
            .add_font("Signature", SCRIPT_TTF.to_vec())
            .expect("add font");
        let font = stamper
            .font(" signature ")
            .expect("names compare case-insensitively");
        assert_eq!(font, Font::Embedded(0));
        let mut signed = text_op(1, "Jane Doe");
        signed.font = font;
        signed.size = 18.0;
        stamper
            .text(signed.clone())
            .expect("text in the added font");
        signed.y_top = 140.0;
        stamper.text(signed).expect("the font again");
        stamper
            .text(text_op(1, "plain"))
            .expect("a built-in font alongside");

        let pdf = stamper.to_pdf().expect("render");
        let doc = Document::load_mem(&pdf).expect("reparse");
        let page_id = doc.get_pages()[&1];
        let text = String::from_utf8_lossy(&doc.get_page_content(page_id)).to_string();
        assert!(text.contains("/IPStampT1 18 Tf"), "{text}");
        assert!(text.contains("/IPStampH 9 Tf"), "{text}");

        let fonts = doc.get_page_fonts(page_id).expect("fonts");
        let embedded = fonts
            .get(b"IPStampT1".as_slice())
            .expect("added font registered");
        let name = |key: &[u8]| embedded.get(key).unwrap().as_name().unwrap().to_vec();
        assert_eq!(name(b"Subtype"), b"TrueType");
        assert_eq!(name(b"Encoding"), b"WinAnsiEncoding");
        assert_eq!(name(b"BaseFont"), b"MrsSaintDelafield-Regular");
        assert_eq!(embedded.get(b"FirstChar").unwrap().as_i64().unwrap(), 32);
        assert_eq!(embedded.get(b"LastChar").unwrap().as_i64().unwrap(), 255);
        let widths = embedded.get(b"Widths").unwrap().as_array().unwrap();
        assert_eq!(widths.len(), 224);
        let face = Face::parse(SCRIPT_TTF, 0).unwrap();
        let advance = face
            .glyph_hor_advance(face.glyph_index('J').unwrap())
            .unwrap();
        let expected = em_thousandths(f64::from(advance), f64::from(face.units_per_em()));
        assert_eq!(widths[usize::from(b'J' - 32)].as_i64().unwrap(), expected);

        let descriptor = doc
            .get_dictionary(
                embedded
                    .get(b"FontDescriptor")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .expect("descriptor");
        assert_eq!(
            descriptor.get(b"Flags").unwrap().as_i64().unwrap() & 32,
            32,
            "nonsymbolic, so viewers find glyphs through WinAnsiEncoding"
        );
        let program = doc
            .get_object(
                descriptor
                    .get(b"FontFile2")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .and_then(Object::as_stream)
            .expect("font program");
        assert_eq!(
            program.dict.get(b"Length1").unwrap().as_i64().unwrap(),
            i64::try_from(SCRIPT_TTF.len()).unwrap()
        );
        assert_eq!(
            program.decompressed_content().unwrap(),
            SCRIPT_TTF.to_vec(),
            "the whole font program, byte for byte"
        );
        // The fixture embeds its own fonts; the stamp adds exactly one program.
        let programs = |doc: &Document| {
            doc.objects
                .values()
                .filter(|object| {
                    object
                        .as_stream()
                        .is_ok_and(|stream| stream.dict.has(b"Length1"))
                })
                .count()
        };
        let template = Document::load_mem(&fixture()).expect("template");
        assert_eq!(
            programs(&doc),
            programs(&template) + 1,
            "one embedded copy however often it is used"
        );
    }

    #[test]
    fn added_fonts_are_validated() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        let err = |result: Result<(), String>| result.unwrap_err();
        assert!(err(stamper.add_font(" ", SCRIPT_TTF.to_vec())).contains("must not be empty"));
        assert!(err(stamper.add_font("Script", Vec::new())).contains("must not be empty"));
        assert!(
            err(stamper.add_font("Script", b"not a font".to_vec())).contains("cannot parse font")
        );
        let mut collection = b"ttcf".to_vec();
        collection.extend_from_slice(SCRIPT_TTF);
        assert!(err(stamper.add_font("Script", collection)).contains(".ttc"));
        assert!(err(stamper.add_font("Helvetica-Bold", SCRIPT_TTF.to_vec())).contains("built-in"));
        stamper
            .add_font("Script", SCRIPT_TTF.to_vec())
            .expect("add");
        assert!(err(stamper.add_font("SCRIPT", SCRIPT_TTF.to_vec())).contains("already added"));
        assert!(
            stamper
                .font("Comic Sans")
                .unwrap_err()
                .contains("unknown font")
        );
        assert_eq!(stamper.font("bold").unwrap(), Font::HelveticaBold);

        let mut stray = text_op(1, "x");
        stray.font = Font::Embedded(7);
        assert!(stamper.text(stray).unwrap_err().contains("never added"));
    }

    /// The fixture with one of its tables edited in place (table checksums
    /// are not verified by the parser, so the edit is all that changes).
    fn patched_fixture(tag: [u8; 4], edit: impl Fn(&mut [u8])) -> Vec<u8> {
        let mut font = SCRIPT_TTF.to_vec();
        let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let record = (0..tables)
            .map(|i| 12 + 16 * i)
            .find(|&at| font[at..at + 4] == tag)
            .expect("table present");
        let offset = u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
        let length =
            u32::from_be_bytes(font[record + 12..record + 16].try_into().unwrap()) as usize;
        edit(&mut font[offset..offset + length]);
        font
    }

    #[test]
    fn fonts_whose_licence_forbids_embedding_are_refused() {
        // OS/2 fsType sits at byte 8: 0x0002 restricted, 0x0200 bitmap only.
        for fs_type in [0x0002_u16, 0x0200] {
            let font = patched_fixture(*b"OS/2", |os2| {
                os2[8..10].copy_from_slice(&fs_type.to_be_bytes());
            });
            let mut stamper = Stamper::load(&fixture()).expect("load");
            let err = stamper.add_font("Script", font).unwrap_err();
            assert!(
                err.contains("does not permit embedding"),
                "{fs_type:#06x}: {err}"
            );
        }
    }

    #[test]
    fn fonts_without_a_windows_unicode_cmap_are_refused() {
        // Relabel every (3,1) encoding record as (0,3): the glyphs stay
        // reachable through Unicode-platform records, which PDF viewers ignore.
        let font = patched_fixture(*b"cmap", |cmap| {
            let records = usize::from(u16::from_be_bytes([cmap[2], cmap[3]]));
            for at in (0..records).map(|i| 4 + 8 * i) {
                if cmap[at..at + 4] == [0, 3, 0, 1] {
                    cmap[at..at + 4].copy_from_slice(&[0, 0, 0, 3]);
                }
            }
        });
        assert!(
            Face::parse(&font, 0).unwrap().glyph_index('a').is_some(),
            "still mapped for the parser"
        );
        let mut stamper = Stamper::load(&fixture()).expect("load");
        let err = stamper.add_font("Script", font).unwrap_err();
        assert!(err.contains("no Windows Unicode cmap"), "{err}");
    }

    #[test]
    fn characters_without_a_glyph_in_an_added_font_are_rejected() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        stamper
            .add_font("Script", SCRIPT_TTF.to_vec())
            .expect("add");
        let font = &stamper.fonts[0];
        assert!(font.covered[usize::from(b'a')] && font.covered[usize::from(b' ')]);
        let Some(byte) = (0x20..=0xFF_u8)
            .find(|&byte| winansi_glyph_char(byte).is_some() && !font.covered[usize::from(byte)])
        else {
            return; // the fixture covers every WinAnsi character
        };
        let ch = winansi_glyph_char(byte).unwrap();
        let mut op = text_op(1, &format!("ab{ch}"));
        op.font = stamper.font("Script").unwrap();
        let message = stamper.text(op).unwrap_err();
        assert!(
            message.contains("has no glyph in font 'Script'"),
            "{message}"
        );
        assert_eq!(stamper.pending_ops(), 0);
    }

    #[test]
    fn added_fonts_shrink_to_fit_with_their_own_widths() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        stamper
            .add_font("Script", SCRIPT_TTF.to_vec())
            .expect("add");
        let mut op = text_op(1, "Jane Doe");
        op.font = stamper.font("Script").unwrap();
        op.size = 30.0;
        let natural = text_width(
            &encode_winansi("Jane Doe").unwrap(),
            &stamper.fonts[0].widths,
            30.0,
        );
        op.max_width = Some(natural / 2.0);
        stamper.text(op).expect("shrinks");
        match &stamper.ops[0] {
            Op::Text { size, .. } => assert!((*size - 15.0).abs() < 1e-6, "{size}"),
            Op::Check(_) => panic!("expected a text op"),
        }
    }

    #[test]
    fn winansi_bytes_map_back_to_their_characters() {
        for (byte, ch) in CP1252_HIGH {
            assert_eq!(winansi_glyph_char(byte), Some(ch));
            assert_eq!(encode_winansi(&ch.to_string()).unwrap(), vec![byte]);
        }
        assert_eq!(winansi_glyph_char(b'A'), Some('A'));
        assert_eq!(winansi_glyph_char(0xE9), Some('é'));
        assert_eq!(winansi_glyph_char(0xA0), Some(' '));
        assert_eq!(winansi_glyph_char(0xAD), Some('-'));
        assert_eq!(winansi_glyph_char(0x81), None);
        assert_eq!(winansi_glyph_char(0x7F), None);
    }

    #[test]
    fn validates_pages_sizes_and_fit() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        assert!(stamper.text(text_op(2, "x")).is_err(), "page out of range");
        assert!(stamper.text(text_op(0, "x")).is_err());
        let mut huge = text_op(1, "x");
        huge.size = 0.0;
        assert!(stamper.text(huge).is_err());
        let mut nan = text_op(1, "x");
        nan.x = f64::NAN;
        assert!(stamper.text(nan).is_err());
        let mut narrow = text_op(1, "This will never fit in ten points");
        narrow.max_width = Some(10.0);
        let err = stamper.text(narrow).unwrap_err();
        assert!(err.contains("does not fit"), "{err}");
        let mut shrink = text_op(1, "Shrink me");
        shrink.max_width = Some(30.0);
        stamper.text(shrink).expect("shrinks instead of failing");
        match &stamper.ops[0] {
            Op::Text { size, .. } => {
                assert!(*size >= MIN_FONT_SIZE && *size < 9.0, "shrunk size {size}");
            }
            Op::Check(_) => panic!("expected a text op"),
        }
        assert!(
            stamper
                .check(CheckOp {
                    page: 1,
                    x: 10.0,
                    y_top: 10.0,
                    size: 0.0,
                    stroke: 1.0
                })
                .is_err()
        );
        assert!(
            stamper
                .check(CheckOp {
                    page: 3,
                    x: 10.0,
                    y_top: 10.0,
                    size: 5.0,
                    stroke: 1.0
                })
                .is_err()
        );
        assert_eq!(stamper.pending_ops(), 1);
        stamper.reset();
        assert_eq!(stamper.pending_ops(), 0);
    }

    #[test]
    fn overlay_wraps_original_content_and_registers_fonts() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        stamper.text(text_op(1, "Hello é")).expect("text");
        let mut bold = text_op(1, "Right");
        bold.font = Font::HelveticaBold;
        bold.align = Align::Right;
        stamper.text(bold).expect("bold text");
        stamper
            .check(CheckOp {
                page: 1,
                x: 50.0,
                y_top: 60.0,
                size: 10.0,
                stroke: 1.5,
            })
            .expect("check");

        let first = stamper.to_pdf().expect("render");
        let second = stamper.to_pdf().expect("render again");
        assert!(first.starts_with(b"%PDF"));
        assert_eq!(
            first, second,
            "rendering must not consume the queue or mutate the template"
        );

        let doc = Document::load_mem(&first).expect("reparse");
        let page_id = doc.get_pages()[&1];
        let page = doc.get_dictionary(page_id).expect("page");
        let contents = page
            .get(b"Contents")
            .and_then(Object::as_array)
            .expect("contents array");
        assert_eq!(contents.len(), 3, "lead + original + tail");
        let lead = doc
            .get_object(contents[0].as_reference().unwrap())
            .and_then(Object::as_stream)
            .expect("lead stream");
        assert_eq!(lead.decompressed_content().unwrap(), b"q\n".to_vec());
        let tail = doc
            .get_object(contents[2].as_reference().unwrap())
            .and_then(Object::as_stream)
            .expect("tail stream");
        let tail_text = String::from_utf8_lossy(&tail.decompressed_content().unwrap()).to_string();
        assert!(
            tail_text.starts_with('Q'),
            "tail closes the wrapper: {tail_text}"
        );
        assert!(tail_text.contains("/IPStampH 9 Tf"), "{tail_text}");
        assert!(tail_text.contains("/IPStampHB 9 Tf"), "{tail_text}");
        assert!(
            tail_text.contains("<48656C6C6F20E9> Tj"),
            "hex-encoded WinAnsi text: {tail_text}"
        );
        assert!(tail_text.contains("l\nS\nQ"), "tick stroke: {tail_text}");

        let fonts = doc.get_page_fonts(page_id).expect("fonts");
        let helvetica = fonts
            .get(b"IPStampH".as_slice())
            .expect("Helvetica registered");
        assert_eq!(
            helvetica.get(b"BaseFont").unwrap().as_name().unwrap(),
            b"Helvetica"
        );
        assert_eq!(
            helvetica.get(b"Encoding").unwrap().as_name().unwrap(),
            b"WinAnsiEncoding"
        );
        let bold = fonts
            .get(b"IPStampHB".as_slice())
            .expect("Helvetica-Bold registered");
        assert_eq!(
            bold.get(b"BaseFont").unwrap().as_name().unwrap(),
            b"Helvetica-Bold"
        );
    }

    #[test]
    fn right_and_center_alignment_anchor_on_x() {
        let mut stamper = Stamper::load(&fixture()).expect("load");
        let mut centred = text_op(1, "AA");
        centred.align = Align::Center;
        centred.x = 100.0;
        centred.size = 10.0;
        stamper.text(centred).expect("centred");
        let pdf = stamper.to_pdf().expect("render");
        let doc = Document::load_mem(&pdf).expect("reparse");
        let page_id = doc.get_pages()[&1];
        let text = String::from_utf8_lossy(&doc.get_page_content(page_id)).to_string();
        // "AA" at 10 pt is 13.34 pt wide, so the centred start is 100 - 6.67.
        assert!(text.contains("1 0 0 1 93.33 741.89 Tm"), "{text}");
    }

    #[test]
    fn pages_without_their_own_resources_get_a_copy() {
        // Build a document whose page inherits Resources from the Pages node.
        let mut doc = Document::with_version("1.5");
        let tree_id = doc.new_object_id();
        let mut inherited = Dictionary::new();
        inherited.set(
            "ProcSet",
            Object::Array(vec![Object::Name(b"PDF".to_vec())]),
        );
        let content_id =
            doc.add_object(Stream::new(Dictionary::new(), b"0 0 m 10 10 l S".to_vec()));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(tree_id));
        page.set("Contents", Object::Reference(content_id));
        let page_id = doc.add_object(page);
        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        pages.set("Count", Object::Integer(1));
        pages.set(
            "MediaBox",
            Object::Array(vec![0.into(), 0.into(), 200.into(), 100.into()]),
        );
        pages.set("Resources", Object::Dictionary(inherited));
        doc.objects.insert(tree_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(tree_id));
        let catalog_id = doc.add_object(catalog);
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save fixture");

        let mut stamper = Stamper::load(&bytes).expect("load");
        assert_eq!(stamper.page_size(1).unwrap(), (200.0, 100.0));
        stamper.text(text_op(1, "inherit")).expect("text");
        let out = stamper.to_pdf().expect("render");
        let doc = Document::load_mem(&out).expect("reparse");
        let page_id = doc.get_pages()[&1];
        let page = doc.get_dictionary(page_id).expect("page");
        let resources = page
            .get(b"Resources")
            .and_then(Object::as_dict)
            .expect("page-level resources copy");
        assert!(resources.has(b"ProcSet"), "inherited entries kept");
        let fonts = doc.get_page_fonts(page_id).expect("fonts");
        assert!(fonts.contains_key(b"IPStampH".as_slice()));
        // The Pages node itself is untouched.
        let pages = doc
            .get_dictionary(
                doc.get_dictionary(page_id)
                    .unwrap()
                    .get(b"Parent")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap();
        let shared = pages.get(b"Resources").and_then(Object::as_dict).unwrap();
        assert!(!shared.has(b"Font"));
    }
}
