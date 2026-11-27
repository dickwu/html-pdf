<?php
// Registered-fonts-only mode, standard-font listing, HTML header/footer, font
// faces and viewer presentation (ironpress-php 0.5). Synthetic documents; the
// fonts are the Liberation faces in tests/fixtures/fonts (SIL OFL 1.1).

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
    } catch (Throwable $e) {
        require_true(str_contains($e->getMessage(), $messagePart), $e->getMessage());
        return;
    }
    throw new RuntimeException("Expected an exception containing '$messagePart'");
}

/** @return string[] every /BaseFont name, subset prefixes removed, sorted and unique */
function baseFonts(string $pdf): array
{
    preg_match_all('#/BaseFont /(?:[A-Z]{6}\+)?([A-Za-z0-9-]+)#', $pdf, $m);
    $names = array_values(array_unique($m[1]));
    sort($names);
    return $names;
}

/** @return string[] standard Type1 fonts listed in the PDF, sorted */
function type1Fonts(string $pdf): array
{
    preg_match_all('#/Subtype /Type1 /BaseFont /([A-Za-z-]+)#', $pdf, $m);
    $names = $m[1];
    sort($names);
    return $names;
}

$regular = file_get_contents(__DIR__ . '/fixtures/fonts/LiberationSans-Regular.ttf');
$bold = file_get_contents(__DIR__ . '/fixtures/fonts/LiberationSans-Bold.ttf');
$mixed = '<p style="font-family: GfitSans">Name 中文 😀 end</p>';

// G1: registered fonts only, missing glyphs blank by default.
$c = new Ironpress\HtmlConverter();
$c->addFont('GfitSans', $regular);
$c->registeredFontsOnly(true);
$pdf = $c->convert($mixed);
require_true(baseFonts($pdf) === ['LiberationSans'], 'Only the added font may be embedded: ' . implode(', ', baseFonts($pdf)));
require_true(substr_count($pdf, '/FontFile2') === 1, 'Exactly one embedded font program');

// G1: missing glyphs can throw instead, listing every uncovered character.
$c->missingGlyphs('error');
expectException(fn () => $c->convert($mixed), "U+4E2D '中'");
expectException(fn () => $c->convert($mixed), "U+1F600 '😀'");
require_true(str_starts_with($c->convert('<p style="font-family: GfitSans">Covered text</p>'), '%PDF'), 'Covered text converts in error mode');
$c->missingGlyphs('blank');
require_true(str_starts_with($c->convert($mixed), '%PDF'), 'Blank mode converts again');
expectException(fn () => $c->missingGlyphs('tofu'), 'Unknown missing-glyph policy');

// G1: standard names map to the built-in Base-14 fonts, never host files.
$base14 = new Ironpress\HtmlConverter();
$base14->registeredFontsOnly(true);
$pdf = $base14->convert('<p style="font-family: Helvetica">Helvetica</p><p style="font-family: Times"><b>Times</b></p>');
require_true(!str_contains($pdf, '/FontFile'), 'No font program may be embedded for Base-14 names');
require_true(type1Fonts($pdf) === ['Helvetica', 'Times-Bold'], 'Base-14 fonts: ' . implode(', ', type1Fonts($pdf)));

// G1: switching the mode off restores host fallback (default behaviour).
$c->registeredFontsOnly(false);
require_true(str_starts_with($c->convert($mixed), '%PDF'), 'System fallback still converts');

// G2: only standard fonts the document uses are listed.
$plain = new Ironpress\HtmlConverter();
$plain->addFont('GfitSans', $regular);
$pdf = $plain->convert('<p style="font-family: GfitSans">Only the added font</p>');
require_true(type1Fonts($pdf) === [], 'Unused standard fonts must not be listed: ' . implode(', ', type1Fonts($pdf)));

// G3: HTML header and footer. Courier and Times-Bold appear only if the
// header and footer fragments are rendered.
$hf = new Ironpress\HtmlConverter();
$hf->registeredFontsOnly(true);
$hf->margins(72.0, 36.0, 72.0, 36.0);
$hf->headerHtml('<div style="font-family: Courier; font-size: 9pt">Workout plan <span style="color:#c00">header</span></div>');
$hf->footerHtml('<div style="font-family: Times; font-weight: bold; font-size: 9pt">Footer</div>');
$pdf = $hf->convert('<p style="font-family: Helvetica">Body</p><p style="font-family: Helvetica; break-before: page">Page two</p>');
require_true(type1Fonts($pdf) === ['Courier', 'Helvetica', 'Times-Bold'], 'Header/footer fonts: ' . implode(', ', type1Fonts($pdf)));
$hf->headerHtml(null);
$hf->footerHtml(null);
$pdf = $hf->convert('<p style="font-family: Helvetica">Body</p>');
require_true(type1Fonts($pdf) === ['Helvetica'], 'null removes the HTML header and footer');
$hf->header('Text header');
$hf->headerHtml('<div style="font-family: Courier">HTML header</div>');
$pdf = $hf->convert('<p style="font-family: Times">Body</p>');
require_true(type1Fonts($pdf) === ['Courier', 'Times-Roman'], 'headerHtml() replaces header(): ' . implode(', ', type1Fonts($pdf)));

// G4: bold and italic faces through addFont(name, data, weight, style).
$faces = new Ironpress\HtmlConverter();
$faces->registeredFontsOnly(true);
$faces->missingGlyphs('error');
$faces->addFont('Brand', $regular);
$faces->addFont('Brand', $bold, 700);
$pdf = $faces->convert('<p style="font-family: Brand">Regular <b>Bold</b> <span style="font-weight: 600">Semi</span></p>');
require_true(baseFonts($pdf) === ['LiberationSans', 'LiberationSans-Bold'], 'Bold face: ' . implode(', ', baseFonts($pdf)));
$italic = new Ironpress\HtmlConverter();
$italic->addFont('Brand', $regular);
$italic->addFont('Brand', $bold, 400, 'italic');
$pdf = $italic->convert('<p style="font-family: Brand"><i>Italic face</i></p>');
require_true(baseFonts($pdf) === ['LiberationSans-Bold'], 'The italic slot takes the registered face: ' . implode(', ', baseFonts($pdf)));
expectException(fn () => $faces->addFont('Brand', $bold, 1200), 'font weight must be between 1 and 1000');
expectException(fn () => $faces->addFont('Brand', $bold, 700, 'slanted'), 'Unknown font style');

// G5: viewer page layout and open zoom (TCPDF SetDisplayMode('fullpage', 'SinglePage')).
$v = new Ironpress\HtmlConverter();
$pdf = $v->convert('<p>Default</p>');
require_true(!str_contains($pdf, '/PageLayout') && !str_contains($pdf, '/OpenAction'), 'No viewer entries by default');
$v->pageLayout('single-page');
$v->openZoom('fit-page');
$pdf = $v->convert('<p>One</p>');
require_true(str_contains($pdf, '/PageLayout /SinglePage'), 'PageLayout SinglePage');
require_true((bool) preg_match('#/OpenAction \[\d+ 0 R /Fit\]#', $pdf), 'OpenAction Fit');
$v->pageLayout('TwoPageRight');
$v->openZoom('75%');
$pdf = $v->convert('<p>Zoom</p>');
require_true(str_contains($pdf, '/PageLayout /TwoPageRight'), 'PageLayout TwoPageRight');
require_true(str_contains($pdf, '/XYZ null null 0.75]'), 'OpenAction 75%');
$v->openZoom('fullwidth');
require_true(str_contains($v->convert('<p>w</p>'), ' /FitH null]'), 'OpenAction FitH');
$v->pageLayout(null);
$v->openZoom(null);
$pdf = $v->convert('<p>Reset</p>');
require_true(!str_contains($pdf, '/PageLayout') && !str_contains($pdf, '/OpenAction'), 'null clears viewer entries');
expectException(fn () => $v->pageLayout('book'), 'Unknown page layout');
expectException(fn () => $v->openZoom('huge'), 'Unknown open zoom');
expectException(fn () => $v->openZoom('0'), 'Unknown open zoom');

echo "OK\n";
