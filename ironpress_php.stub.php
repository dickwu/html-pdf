<?php

// Stubs for ironpress_php

namespace Ironpress {
    /**
     * Stateful HTML converter exposed as Ironpress\HtmlConverter.
     */
    class HtmlConverter {
        public function __construct() {}

        /**
         * @param string $name
         * @param string $ttf_data
         * @return void
         */
        public function addFont(string $name, string $ttf_data): void {}

        /**
         * @param string|null $path
         * @return void
         */
        public function basePath(?string $path = null): void {}

        /**
         * @return void
         */
        public function clearFonts(): void {}

        /**
         * @param string $html
         * @return string
         */
        public function convert(string $html): string {}

        /**
         * @param string $markdown
         * @return string
         */
        public function convertMarkdown(string $markdown): string {}

        /**
         * @param float $width
         * @param float $height
         * @return void
         */
        public function customPageSize(float $width, float $height): void {}

        /**
         * @param string|null $text
         * @return void
         */
        public function footer(?string $text = null): void {}

        /**
         * @param string|null $text
         * @return void
         */
        public function header(?string $text = null): void {}

        /**
         * @param bool $enabled
         * @return void
         */
        public function landscape(bool $enabled): void {}

        /**
         * @param float $points
         * @return void
         */
        public function margin(float $points): void {}

        /**
         * @param float $top
         * @param float $right
         * @param float $bottom
         * @param float $left
         * @return void
         */
        public function margins(float $top, float $right, float $bottom, float $left): void {}

        /**
         * Lay out `html` (without rendering a PDF) and return the top y-position,
         * in points from the top of the page content box, of every "sentinel"
         * element — an empty block whose fixed `height` (pt) and solid
         * `background-color` (#RRGGBB) both match the given signature.
         *
         * Interleave sentinel divs between blocks to measure them: the distance
         * between consecutive sentinel tops minus the sentinel height is the
         * block's exact flow height (content + vertical margins), using the same
         * fonts, CSS and wrapping as `convert()`. The whole document must fit one
         * page (declare e.g. `@page { size: 612pt 14000pt; }`) or this throws.
         *
         * @param string $html
         * @param float $sentinel_height
         * @param string $sentinel_color
         * @return array
         */
        public function measureSentinelTops(string $html, float $sentinel_height, string $sentinel_color): array {}

        /**
         * @param string $name
         * @return void
         */
        public function pageSize(string $name): void {}

        /**
         * @param bool $enabled
         * @return void
         */
        public function sanitize(bool $enabled): void {}
    }

    /**
     * Overlay text and tick marks onto the pages of an existing PDF (for example a
     * government form) without altering the original page content. Exposed as
     * Ironpress\PdfStamper.
     *
     * Coordinates are PDF points with y measured from the TOP edge of the page:
     * `text()` takes the baseline, `check()` the top-left corner of the box.
     * Text uses the built-in Helvetica / Helvetica-Bold or a TrueType font added
     * with `addFont()`, always `WinAnsi`-encoded; characters outside that
     * repertoire raise an exception instead of printing a `?`.
     */
    class PdfStamper {
        /**
         * @param string $pdf
         */
        public function __construct(string $pdf) {}

        /**
         * Embed a TrueType font (the bytes of a .ttf file) that `text()` can then
         * use by `name`, for example a script face for a signature line. The
         * whole file is embedded once per PDF. Text in it is still `WinAnsi`, and
         * a character the font has no glyph for throws. Added fonts stay through
         * `reset()`.
         *
         * @param string $name
         * @param string $ttf_data
         * @return void
         */
        public function addFont(string $name, string $ttf_data): void {}

        /**
         * Queue a vector tick inside a `size`-pt box whose top-left corner is
         * (`x`, `y_top`); `stroke` is the line width (default 1.5 pt).
         *
         * @param int $page
         * @param float $x
         * @param float $y_top
         * @param float $size
         * @param float|null $stroke
         * @return void
         */
        public function check(int $page, float $x, float $y_top, float $size, ?float $stroke = null): void {}

        /**
         * Number of pages in the loaded PDF.
         *
         * @return int
         */
        public function pageCount(): int {}

        /**
         * `[width, height]` of a 1-based page in points (its `MediaBox`).
         *
         * @param int $page
         * @return array
         */
        public function pageSize(int $page): array {}

        /**
         * Number of queued operations.
         *
         * @return int
         */
        public function pendingOps(): int {}

        /**
         * Drop every queued operation; the loaded PDF stays.
         *
         * @return void
         */
        public function reset(): void {}

        /**
         * Queue a single-line text at baseline (`x`, `y_top`) on a 1-based page.
         * `size` defaults to 9 pt, `font` to Helvetica (or Helvetica-Bold, or a
         * font added with `addFont()`), `align` to left (or center / right,
         * anchored on `x`), and `max_width` shrinks the font down to 5 pt so the
         * text fits, else throws.
         *
         * @param int $page
         * @param float $x
         * @param float $y_top
         * @param string $text
         * @param float|null $size
         * @param string|null $font
         * @param string|null $align
         * @param float|null $max_width
         * @return void
         */
        public function text(int $page, float $x, float $y_top, string $text, ?float $size = null, ?string $font = null, ?string $align = null, ?float $max_width = null): void {}

        /**
         * The original PDF with every queued operation drawn on top. The loaded
         * PDF and the queue are left untouched, so the same template can be
         * filled again after `reset()`.
         *
         * @return string
         */
        public function toPdf(): string {}
    }
}

namespace {
    /**
     * Convert an HTML file to a PDF file.
     *
     * @param string $input
     * @param string $output
     * @return void
     */
    function ironpress_convert_file(string $input, string $output): void {}

    /**
     * Convert a Markdown file to a PDF file.
     *
     * @param string $input
     * @param string $output
     * @return void
     */
    function ironpress_convert_markdown_file(string $input, string $output): void {}

    /**
     * Convert HTML to PDF bytes.
     *
     * @param string $html
     * @return string
     */
    function ironpress_html_to_pdf(string $html): string {}

    /**
     * Convert HTML to PDF and save it to a file.
     *
     * @param string $html
     * @param string $output
     * @return void
     */
    function ironpress_html_to_pdf_file(string $html, string $output): void {}

    /**
     * Convert Markdown to PDF bytes.
     *
     * @param string $markdown
     * @return string
     */
    function ironpress_markdown_to_pdf(string $markdown): string {}

    /**
     * Return the PHP extension version.
     *
     * @return string
     */
    function ironpress_version(): string {}
}
