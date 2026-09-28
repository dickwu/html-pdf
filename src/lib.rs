#![cfg_attr(windows, feature(abi_vectorcall))]
#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

use std::{fs, path::PathBuf};

use ext_php_rs::{binary::Binary, prelude::*};
use ironpress_core::{HtmlConverter as CoreHtmlConverter, Margin, PageSize};

mod base14;
mod stamper;

use stamper::{Align, CheckOp, Font, Stamper, TextOp};

type PdfBinary = Binary<u8>;

fn pdf_to_php_bytes(pdf: Vec<u8>) -> PdfBinary {
    Binary::new(pdf)
}

fn php_err(message: impl Into<String>) -> PhpException {
    PhpException::default(message.into())
}

#[allow(clippy::needless_pass_by_value)]
fn map_ironpress_err(err: ironpress_core::IronpressError) -> PhpException {
    php_err(err.to_string())
}

fn validate_positive_points(name: &str, value: f64) -> PhpResult<f32> {
    if !value.is_finite() || value <= 0.0 || value > 14_400.0 {
        return Err(php_err(format!(
            "{name} must be a finite positive value in points and <= 14400"
        )));
    }

    #[allow(clippy::cast_possible_truncation)]
    Ok(value as f32)
}

fn validate_margin_points(name: &str, value: f64) -> PhpResult<f32> {
    if !value.is_finite() || !(0.0..=14_400.0).contains(&value) {
        return Err(php_err(format!(
            "{name} must be a finite non-negative value in points and <= 14400"
        )));
    }

    #[allow(clippy::cast_possible_truncation)]
    Ok(value as f32)
}

fn parse_hex_color(hex: &str) -> PhpResult<(u8, u8, u8)> {
    let digits = hex.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(php_err("sentinel color must be #RRGGBB"));
    }
    let value = u32::from_str_radix(digits, 16).map_err(|err| php_err(err.to_string()))?;

    #[allow(clippy::cast_possible_truncation)]
    Ok((
        ((value >> 16) & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        (value & 0xFF) as u8,
    ))
}

fn page_size_by_name(name: &str) -> PhpResult<PageSize> {
    match name.trim().to_ascii_lowercase().as_str() {
        "a4" => Ok(PageSize::A4),
        "letter" => Ok(PageSize::LETTER),
        "legal" => Ok(PageSize::LEGAL),
        other => Err(php_err(format!(
            "Unknown page size '{other}'. Expected one of: a4, letter, legal"
        ))),
    }
}

/// Convert HTML to PDF bytes.
#[php_function]
#[php(name = "ironpress_html_to_pdf")]
pub fn html_to_pdf(html: &str) -> PhpResult<PdfBinary> {
    ironpress_core::html_to_pdf(html)
        .map(pdf_to_php_bytes)
        .map_err(map_ironpress_err)
}

/// Convert HTML to PDF and save it to a file.
#[php_function]
#[php(name = "ironpress_html_to_pdf_file")]
pub fn html_to_pdf_file(html: &str, output: &str) -> PhpResult<()> {
    let pdf = ironpress_core::html_to_pdf(html).map_err(map_ironpress_err)?;

    fs::write(output, pdf)
        .map_err(|err| php_err(format!("Failed to write PDF to '{output}': {err}")))
}

/// Convert Markdown to PDF bytes.
#[php_function]
#[php(name = "ironpress_markdown_to_pdf")]
pub fn markdown_to_pdf(markdown: &str) -> PhpResult<PdfBinary> {
    ironpress_core::markdown_to_pdf(markdown)
        .map(pdf_to_php_bytes)
        .map_err(map_ironpress_err)
}

/// Convert an HTML file to a PDF file.
#[php_function]
#[php(name = "ironpress_convert_file")]
pub fn convert_file(input: &str, output: &str) -> PhpResult<()> {
    ironpress_core::convert_file(input, output).map_err(map_ironpress_err)
}

/// Convert a Markdown file to a PDF file.
#[php_function]
#[php(name = "ironpress_convert_markdown_file")]
pub fn convert_markdown_file(input: &str, output: &str) -> PhpResult<()> {
    ironpress_core::convert_markdown_file(input, output).map_err(map_ironpress_err)
}

/// Return the PHP extension version.
#[php_function]
#[php(name = "ironpress_version")]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Stateful HTML converter exposed as Ironpress\HtmlConverter.
#[php_class]
#[php(name = "Ironpress\\HtmlConverter")]
#[derive(Debug, Clone)]
pub struct HtmlConverter {
    page_size: PageSize,
    margin: Margin,
    sanitize: bool,
    landscape: bool,
    header: Option<String>,
    footer: Option<String>,
    base_path: Option<PathBuf>,
    fonts: Vec<(String, Vec<u8>)>,
}

#[php_impl]
impl HtmlConverter {
    pub fn __construct() -> Self {
        Self {
            page_size: PageSize::A4,
            margin: Margin::default(),
            sanitize: true,
            landscape: false,
            header: None,
            footer: None,
            base_path: None,
            fonts: Vec::new(),
        }
    }

    pub fn page_size(&mut self, name: &str) -> PhpResult<()> {
        self.page_size = page_size_by_name(name)?;
        Ok(())
    }

    pub fn custom_page_size(&mut self, width: f64, height: f64) -> PhpResult<()> {
        let width = validate_positive_points("width", width)?;
        let height = validate_positive_points("height", height)?;
        self.page_size = PageSize::new(width, height);
        Ok(())
    }

    pub fn landscape(&mut self, enabled: bool) {
        self.landscape = enabled;
    }

    pub fn margin(&mut self, points: f64) -> PhpResult<()> {
        let points = validate_margin_points("margin", points)?;
        self.margin = Margin::uniform(points);
        Ok(())
    }

    pub fn margins(&mut self, top: f64, right: f64, bottom: f64, left: f64) -> PhpResult<()> {
        self.margin = Margin::new(
            validate_margin_points("top", top)?,
            validate_margin_points("right", right)?,
            validate_margin_points("bottom", bottom)?,
            validate_margin_points("left", left)?,
        );
        Ok(())
    }

    pub fn sanitize(&mut self, enabled: bool) {
        self.sanitize = enabled;
    }

    pub fn header(&mut self, text: Option<String>) {
        self.header = text;
    }

    pub fn footer(&mut self, text: Option<String>) {
        self.footer = text;
    }

    pub fn base_path(&mut self, path: Option<String>) {
        self.base_path = path.map(PathBuf::from);
    }

    pub fn add_font(&mut self, name: &str, ttf_data: Binary<u8>) -> PhpResult<()> {
        let bytes = Vec::from(ttf_data);
        if name.trim().is_empty() {
            return Err(php_err("font name must not be empty"));
        }
        if bytes.is_empty() {
            return Err(php_err("font data must not be empty"));
        }
        if bytes.len() > 50 * 1024 * 1024 {
            return Err(php_err("font data exceeds 50 MB limit"));
        }

        self.fonts.push((name.to_string(), bytes));
        Ok(())
    }

    pub fn clear_fonts(&mut self) {
        self.fonts.clear();
    }

    pub fn convert(&self, html: &str) -> PhpResult<PdfBinary> {
        self.build_converter()
            .convert(html)
            .map(pdf_to_php_bytes)
            .map_err(map_ironpress_err)
    }

    pub fn convert_markdown(&self, markdown: &str) -> PhpResult<PdfBinary> {
        self.build_converter()
            .convert_markdown(markdown)
            .map(pdf_to_php_bytes)
            .map_err(map_ironpress_err)
    }

    /// Lay out `html` (without rendering a PDF) and return the top y-position,
    /// in points from the top of the page content box, of every "sentinel"
    /// element — an empty block whose fixed `height` (pt) and solid
    /// `background-color` (#RRGGBB) both match the given signature.
    ///
    /// Interleave sentinel divs between blocks to measure them: the distance
    /// between consecutive sentinel tops minus the sentinel height is the
    /// block's exact flow height (content + vertical margins), using the same
    /// fonts, CSS and wrapping as convert(). The whole document must fit one
    /// page (declare e.g. `@page { size: 612pt 14000pt; }`) or this throws.
    pub fn measure_sentinel_tops(
        &self,
        html: &str,
        sentinel_height: f64,
        sentinel_color: &str,
    ) -> PhpResult<Vec<f64>> {
        let height = validate_positive_points("sentinel_height", sentinel_height)?;
        let color = parse_hex_color(sentinel_color)?;

        self.build_converter()
            .measure_sentinel_tops(html, height, color)
            .map(|tops| tops.into_iter().map(f64::from).collect())
            .map_err(map_ironpress_err)
    }
}

impl HtmlConverter {
    fn effective_page_size(&self) -> PageSize {
        if self.landscape && self.page_size.height > self.page_size.width {
            PageSize::new(self.page_size.height, self.page_size.width)
        } else {
            self.page_size
        }
    }

    fn build_converter(&self) -> CoreHtmlConverter {
        let mut converter = CoreHtmlConverter::new()
            .page_size(self.effective_page_size())
            .margin(self.margin)
            .sanitize(self.sanitize);

        if let Some(header) = &self.header {
            converter = converter.header(header.clone());
        }
        if let Some(footer) = &self.footer {
            converter = converter.footer(footer.clone());
        }
        if let Some(base_path) = &self.base_path {
            converter = converter.base_path(base_path.as_path());
        }
        for (name, bytes) in &self.fonts {
            converter = converter.add_font(name, bytes.clone());
        }

        converter
    }
}

/// Overlay text and tick marks onto the pages of an existing PDF (for example a
/// government form) without altering the original page content. Exposed as
/// Ironpress\PdfStamper.
///
/// Coordinates are PDF points with y measured from the TOP edge of the page:
/// `text()` takes the baseline, `check()` the top-left corner of the box.
/// Text uses the built-in Helvetica / Helvetica-Bold (`WinAnsi`); characters
/// outside that repertoire raise an exception instead of printing a `?`.
#[php_class]
#[php(name = "Ironpress\\PdfStamper")]
#[derive(Debug, Clone)]
pub struct PdfStamper {
    inner: Stamper,
}

#[php_impl]
impl PdfStamper {
    pub fn __construct(pdf: Binary<u8>) -> PhpResult<Self> {
        let bytes = Vec::from(pdf);
        if bytes.len() > 100 * 1024 * 1024 {
            return Err(php_err("PDF data exceeds 100 MB limit"));
        }
        Stamper::load(&bytes)
            .map(|inner| Self { inner })
            .map_err(php_err)
    }

    /// Number of pages in the loaded PDF.
    pub fn page_count(&self) -> i64 {
        i64::try_from(self.inner.page_count()).unwrap_or(i64::MAX)
    }

    /// `[width, height]` of a 1-based page in points (its `MediaBox`).
    pub fn page_size(&self, page: i64) -> PhpResult<Vec<f64>> {
        let (width, height) = self.inner.page_size(page_number(page)?).map_err(php_err)?;
        Ok(vec![width, height])
    }

    /// Queue a single-line text at baseline (`x`, `y_top`) on a 1-based page.
    /// `size` defaults to 9 pt, `font` to Helvetica (or Helvetica-Bold),
    /// `align` to left (or center / right, anchored on `x`), and `max_width`
    /// shrinks the font down to 5 pt so the text fits, else throws.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        page: i64,
        x: f64,
        y_top: f64,
        text: &str,
        size: Option<f64>,
        font: Option<&str>,
        align: Option<&str>,
        max_width: Option<f64>,
    ) -> PhpResult<()> {
        let op = TextOp {
            page: page_number(page)?,
            x,
            y_top,
            text: text.to_string(),
            size: size.unwrap_or(9.0),
            font: Font::parse(font.unwrap_or("")).map_err(php_err)?,
            align: Align::parse(align.unwrap_or("")).map_err(php_err)?,
            max_width,
        };
        self.inner.text(op).map_err(php_err)
    }

    /// Queue a vector tick inside a `size`-pt box whose top-left corner is
    /// (`x`, `y_top`); `stroke` is the line width (default 1.5 pt).
    pub fn check(
        &mut self,
        page: i64,
        x: f64,
        y_top: f64,
        size: f64,
        stroke: Option<f64>,
    ) -> PhpResult<()> {
        let op = CheckOp {
            page: page_number(page)?,
            x,
            y_top,
            size,
            stroke: stroke.unwrap_or(1.5),
        };
        self.inner.check(op).map_err(php_err)
    }

    /// Drop every queued operation; the loaded PDF stays.
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Number of queued operations.
    pub fn pending_ops(&self) -> i64 {
        i64::try_from(self.inner.pending_ops()).unwrap_or(i64::MAX)
    }

    /// The original PDF with every queued operation drawn on top. The loaded
    /// PDF and the queue are left untouched, so the same template can be
    /// filled again after `reset()`.
    pub fn to_pdf(&self) -> PhpResult<PdfBinary> {
        self.inner.to_pdf().map(pdf_to_php_bytes).map_err(php_err)
    }
}

fn page_number(page: i64) -> PhpResult<u32> {
    u32::try_from(page)
        .ok()
        .filter(|&page| page >= 1)
        .ok_or_else(|| php_err(format!("page must be a positive integer, got {page}")))
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
        .name("ironpress_php")
        .version(env!("CARGO_PKG_VERSION"))
        .function(wrap_function!(html_to_pdf))
        .function(wrap_function!(html_to_pdf_file))
        .function(wrap_function!(markdown_to_pdf))
        .function(wrap_function!(convert_file))
        .function(wrap_function!(convert_markdown_file))
        .function(wrap_function!(version))
        .class::<HtmlConverter>()
        .class::<PdfStamper>()
}
