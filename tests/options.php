<?php

function require_true(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}

function expectException(callable $callback, string $messagePart): void
{
    try {
        $callback();
        throw new RuntimeException('Expected exception');
    } catch (Throwable $e) {
        require_true(str_contains($e->getMessage(), $messagePart), $e->getMessage());
    }
}

$converter = new Ironpress\HtmlConverter();
$converter->pageSize('letter');
$converter->landscape(true);
$converter->margin(36.0);
$converter->margins(12.0, 18.0, 24.0, 30.0);
$converter->sanitize(true);
$converter->header('Header');
$converter->footer('Page {page} of {pages}');
$converter->basePath(__DIR__);

$pdf = $converter->convert('<h1>Options</h1><p>Testing options.</p>');
require_true(is_string($pdf), 'HTML conversion should return a string');
require_true(str_starts_with($pdf, '%PDF'), 'HTML conversion should return PDF bytes');

$markdownPdf = $converter->convertMarkdown("# Options\n\nTesting **Markdown** options.");
require_true(is_string($markdownPdf), 'Markdown conversion should return a string');
require_true(str_starts_with($markdownPdf, '%PDF'), 'Markdown conversion should return PDF bytes');

$converter->customPageSize(320.0, 240.0);
$converter->header(null);
$converter->footer(null);
$converter->basePath(null);
$pdf = $converter->convert('<h1>Custom size</h1>');
require_true(str_starts_with($pdf, '%PDF'), 'Custom page size conversion should return PDF bytes');

expectException(fn () => $converter->pageSize('unknown'), 'Unknown page size');
expectException(fn () => $converter->margin(-1.0), 'margin must be a finite non-negative value');
expectException(fn () => $converter->margins(1.0, -1.0, 1.0, 1.0), 'right must be a finite non-negative value');
expectException(fn () => $converter->customPageSize(0.0, 10.0), 'width must be a finite positive value');
expectException(fn () => $converter->addFont('', 'font-data'), 'font name must not be empty');
expectException(fn () => $converter->addFont('Empty', ''), 'font data must not be empty');

$converter->clearFonts();

// Table-cell metrics: 0.2.x-compatible by default, Chromium-like on request.
$sentinel = '<div style="height:2.5pt;background-color:#010203"></div>';
$cellHtml = '<html><head><style>@page { size: 612pt 14000pt; margin: 0; } body { margin: 0; font-size: 10pt; line-height: 12pt; }</style></head><body>'
    . $sentinel
    . '<table style="border-collapse:collapse;width:100%"><tr><td style="padding:0;border:6pt solid #000"><p style="margin:10pt 0 0 0">x</p><p style="margin:0 0 10pt 0">y</p></td></tr></table>'
    . $sentinel . '</body></html>';
$band = static function (Ironpress\HtmlConverter $c) use ($cellHtml): float {
    $tops = $c->measureSentinelTops($cellHtml, 2.5, '#010203');
    require_true(count($tops) === 2, 'Expected two sentinels');
    return $tops[1] - $tops[0] - 2.5;
};
$metrics = new Ironpress\HtmlConverter();
$legacyBand = $band($metrics);
require_true(abs($legacyBand - 44.0) < 0.1, "0.2.x cell metrics: expected 44pt, got $legacyBand");
$metrics->legacyTableCellMetrics(false);
$chromiumBand = $band($metrics);
require_true(abs($chromiumBand - 56.0) < 0.1, "Chromium cell metrics: expected 56pt, got $chromiumBand");

// A no-break space extracts as a plain space.
$nbspPdf = (new Ironpress\HtmlConverter())->convert("<p>500\u{00A0}mL</p>");
require_true(str_contains($nbspPdf, '<0020>') && !str_contains($nbspPdf, '<00A0>'), 'No-break space should map to U+0020 in ToUnicode');

echo "OK\n";
