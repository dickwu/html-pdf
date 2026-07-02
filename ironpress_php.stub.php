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
         * Top y-positions (points, page-content coordinates) of every sentinel
         * element — an empty block matching the given height (pt) and solid
         * background color (#RRGGBB). Distances between consecutive tops minus
         * the sentinel height are exact block flow heights. The document must
         * fit one page (declare a tall @page size).
         *
         * @param string $html
         * @param float $sentinel_height
         * @param string $sentinel_color
         * @return float[]
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
