//! Overlay text and vector marks onto the pages of an existing PDF without
//! touching the original page content (the government-form use case).

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use crate::base14::{HELVETICA, HELVETICA_BOLD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Helvetica,
    HelveticaBold,
}

impl Font {
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
                "unknown font '{other}': use Helvetica or Helvetica-Bold"
            )),
        }
    }

    fn base_font(self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::HelveticaBold => "Helvetica-Bold",
        }
    }

    /// Resource name used inside the page's /Font dictionary.
    fn resource_name(self) -> &'static str {
        match self {
            Self::Helvetica => "IPStampH",
            Self::HelveticaBold => "IPStampHB",
        }
    }

    fn widths(self) -> &'static [u16; 256] {
        match self {
            Self::Helvetica => &HELVETICA,
            Self::HelveticaBold => &HELVETICA_BOLD,
        }
    }
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

/// A loaded PDF plus the pending overlay operations.
#[derive(Debug, Clone)]
pub struct Stamper {
    doc: Document,
    pages: BTreeMap<u32, ObjectId>,
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
            ops: Vec::new(),
        })
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
        let mut size = op.size;
        if let Some(max_width) = op.max_width {
            let unit_width = text_width(&encoded, op.font, 1.0);
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
                        let width = text_width(encoded, op.font, *size);
                        let x = match op.align {
                            Align::Left => op.x,
                            Align::Center => op.x - width / 2.0,
                            Align::Right => op.x - width,
                        };
                        push_text(
                            &mut content,
                            op.font,
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
                ensure_font(&mut doc, page_id, font)?;
            }
            wrap_contents(&mut doc, page_id, lead_id, tail_id)?;
        }
        doc.compress();
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|err| format!("cannot write PDF: {err}"))?;
        Ok(out)
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

fn push_text(content: &mut Content, font: Font, size: f64, x: f64, y: f64, encoded: &[u8]) {
    let ops = &mut content.operations;
    ops.push(Operation::new("BT", vec![]));
    ops.push(Operation::new(
        "Tf",
        vec![
            Object::Name(font.resource_name().as_bytes().to_vec()),
            real(size),
        ],
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

/// Advance width in points of WinAnsi-encoded bytes at `size`.
pub fn text_width(encoded: &[u8], font: Font, size: f64) -> f64 {
    let widths = font.widths();
    let units: u32 = encoded
        .iter()
        .map(|&b| u32::from(widths[usize::from(b)]))
        .sum();
    f64::from(units) * size / 1000.0
}

/// Map a string to `WinAnsiEncoding` (cp1252) bytes. Anything outside that
/// repertoire, and any control character, is an error rather than a silent
/// substitution: the caller decides how to transliterate.
pub fn encode_winansi(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let byte = match ch {
            '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{FF}' => ch as u8,
            '€' => 0x80,
            '‚' => 0x82,
            'ƒ' => 0x83,
            '„' => 0x84,
            '…' => 0x85,
            '†' => 0x86,
            '‡' => 0x87,
            'ˆ' => 0x88,
            '‰' => 0x89,
            'Š' => 0x8A,
            '‹' => 0x8B,
            'Œ' => 0x8C,
            'Ž' => 0x8E,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '˜' => 0x98,
            '™' => 0x99,
            'š' => 0x9A,
            '›' => 0x9B,
            'œ' => 0x9C,
            'ž' => 0x9E,
            'Ÿ' => 0x9F,
            '\t' => 0x20,
            other => {
                return Err(format!(
                    "character '{other}' (U+{:04X}) cannot be written with the built-in Helvetica font",
                    u32::from(other)
                ));
            }
        };
        out.push(byte);
    }
    Ok(out)
}

/// Make sure the page's /Resources /Font dictionary maps the stamp font's
/// resource name to a base-14 font object. Handles a direct resources dict, a
/// referenced one, and resources inherited from the Pages tree (copied down to
/// the page so sibling pages are untouched).
fn ensure_font(doc: &mut Document, page_id: ObjectId, font: Font) -> Result<(), String> {
    let mut font_dict = Dictionary::new();
    font_dict.set("Type", Object::Name(b"Font".to_vec()));
    font_dict.set("Subtype", Object::Name(b"Type1".to_vec()));
    font_dict.set(
        "BaseFont",
        Object::Name(font.base_font().as_bytes().to_vec()),
    );
    font_dict.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    let font_id = doc.add_object(font_dict);

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
    fonts.set(font.resource_name(), Object::Reference(font_id));
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
        let width = text_width(&encoded, Font::Helvetica, 10.0);
        assert!((width - 20.01).abs() < 1e-6, "{width}");
        let bold = text_width(&encoded, Font::HelveticaBold, 10.0);
        assert!((bold - 21.66).abs() < 1e-6, "{bold}");
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
