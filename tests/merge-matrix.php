<?php
/**
 * PDF merge test matrix: synthetic fixtures only.
 *
 *   python3 tests/fixtures/merge/generate.py tests/fixtures/merge/build --large
 *   php -d extension=target/release/libironpress_php.dylib tests/merge-matrix.php \
 *       tests/fixtures/merge/build [--update-goldens]
 *
 * Every merged page is checked against the page it came from: page count
 * (pdfinfo), size and rotation (pdfinfo), qpdf --check, text (pdftotext per
 * page), image count (pdfimages -list per page) and a raster comparison
 * (pdftoppm, ImageMagick `compare -metric AE`) against the source page and
 * against the stored golden PNG. Prints a markdown table and exits 1 on any
 * failure.
 */

declare(strict_types=1);

use Ironpress\EncryptedPdfException;
use Ironpress\InvalidPdfException;
use Ironpress\PdfLimitException;
use Ironpress\PdfMergeException;
use Ironpress\PdfMerger;

$dir = rtrim($argv[1] ?? __DIR__ . '/fixtures/merge/build', '/');
$updateGoldens = in_array('--update-goldens', $argv, true);
$goldenDir = __DIR__ . '/fixtures/merge/golden';
$work = $dir . '/out';
@mkdir($work, 0777, true);
@mkdir($goldenDir, 0777, true);
const DPI = 30;

foreach (['pdfinfo', 'qpdf', 'pdftotext', 'pdfimages', 'pdftoppm', 'magick'] as $tool) {
    if (trim((string) shell_exec('command -v ' . $tool)) === '') {
        fwrite(STDERR, "missing tool: $tool\n");
        exit(2);
    }
}

function sh(string $cmd, ?int &$code = null): string
{
    $out = [];
    exec($cmd . ' 2>&1', $out, $code);
    return implode("\n", $out);
}

function q(string $s): string
{
    return escapeshellarg($s);
}

/** @return list<array{w: float, h: float, rot: int}> */
function pageInfo(string $file): array
{
    $n = pageCount($file);
    $info = sh('pdfinfo -f 1 -l ' . $n . ' ' . q($file));
    preg_match_all('/^Page\s+(\d+) size:\s+([\d.]+) x ([\d.]+)/m', $info, $sizes, PREG_SET_ORDER);
    preg_match_all('/^Page\s+(\d+) rot:\s+(\d+)/m', $info, $rots, PREG_SET_ORDER);
    $pages = [];
    foreach ($sizes as $i => $m) {
        $pages[] = ['w' => (float) $m[2], 'h' => (float) $m[3], 'rot' => (int) ($rots[$i][2] ?? 0)];
    }
    return $pages;
}

function pageCount(string $file): int
{
    preg_match('/^Pages:\s+(\d+)/m', sh('pdfinfo ' . q($file)), $m);
    return (int) ($m[1] ?? -1);
}

function pageText(string $file, int $page): string
{
    $text = sh("pdftotext -f $page -l $page -layout " . q($file) . ' -');
    return trim((string) preg_replace('/\s+/', ' ', $text));
}

function imageCount(string $file, int $page): int
{
    $lines = explode("\n", sh("pdfimages -f $page -l $page -list " . q($file)));
    return max(0, count($lines) - 2);
}

function render(string $file, int $page, string $prefix): string
{
    sh('pdftoppm -r ' . DPI . " -f $page -l $page -singlefile -png " . q($file) . ' ' . q($prefix));
    return $prefix . '.png';
}

function ae(string $a, string $b): int
{
    if (!is_file($a) || !is_file($b)) {
        return -1;
    }
    $out = sh('magick compare -metric AE ' . q($a) . ' ' . q($b) . ' null:');
    return is_numeric(strtok($out, ' ')) ? (int) strtok($out, ' ') : -1;
}

/** Synthetic PNG (gradient with a box), no GD needed. */
function png(int $w, int $h): string
{
    $raw = '';
    for ($y = 0; $y < $h; $y++) {
        $raw .= "\0";
        for ($x = 0; $x < $w; $x++) {
            $box = $x > $w / 4 && $x < $w * 3 / 4 && $y > $h / 4 && $y < $h * 3 / 4;
            $raw .= $box ? "\x20\x40\xa0" : chr(intdiv(255 * $x, $w)) . chr(intdiv(255 * $y, $h)) . "\x80";
        }
    }
    $chunk = static fn (string $t, string $d): string => pack('N', strlen($d)) . $t . $d . pack('N', crc32($t . $d));
    return "\x89PNG\r\n\x1a\n" . $chunk('IHDR', pack('NNCCCCC', $w, $h, 8, 2, 0, 0, 0))
        . $chunk('IDAT', (string) gzcompress($raw)) . $chunk('IEND', '');
}

// ---------------------------------------------------------------- inputs

$img = 'data:image/png;base64,' . base64_encode(png(240, 160));
$noteHtml = '<html><head><style>body{font-family:Helvetica;font-size:11pt}'
    . 'td{border:1px solid #999;padding:4pt}</style></head><body>'
    . '<h1>MARK-ironpress-note</h1><p>Synthetic progress note for merge tests.</p>'
    . '<table><tr><td>Visit</td><td>2026-10-26</td></tr><tr><td>Notes</td><td>Lorem ipsum dolor sit amet.</td></tr></table>'
    . '<p><img src="' . $img . '" style="width:240pt"></p>'
    . str_repeat('<p>Filler paragraph to push onto a second page. Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>', 40)
    . '</body></html>';
file_put_contents("$dir/ironpress-note.pdf", ironpress_html_to_pdf($noteHtml));
$letter = new Ironpress\HtmlConverter();
$letter->pageSize('letter');
$letter->landscape(true);
file_put_contents("$dir/ironpress-letter-landscape.pdf", $letter->convert(
    '<h2>MARK-ironpress-attachment</h2><p>Landscape letter attachment.</p><p><img src="' . $img . '"></p>'
));

$base = "$dir/ironpress-note.pdf";
$f = static fn (string $name): string => "$dir/$name";
$exists = static fn (string ...$names): bool => array_reduce($names, static fn ($ok, $n) => $ok && is_file("$dir/$n"), true);

$cases = [
    // name => [attachments, expected exception class or null, expected code]
    'zero attachments' => [[], null, 0],
    'one ironpress' => [['ironpress-letter-landscape.pdf'], null, 0],
    'raw inherited attrs' => [['raw-inherited.pdf'], null, 0],
    'raw incremental update' => [['raw-incremental.pdf'], null, 0],
    'raw image + link annot' => [['raw-image-annot.pdf'], null, 0],
    'qpdf object+xref streams' => [['qpdf-objstm.pdf'], null, 0],
    'ghostscript mixed sizes' => [['gs-mixed.pdf'], null, 0],
    'reportlab sizes+rotate' => [['reportlab-mixed.pdf'], null, 0],
    'imagemagick photo' => [['magick-photo.pdf'], null, 0],
    'macOS cupsfilter' => [['cups-text.pdf'], null, 0],
    'many (10 mixed)' => [[
        'ironpress-letter-landscape.pdf', 'raw-inherited.pdf', 'raw-incremental.pdf', 'raw-image-annot.pdf',
        'qpdf-objstm.pdf', 'gs-mixed.pdf', 'reportlab-mixed.pdf', 'magick-photo.pdf', 'cups-text.pdf',
        'ironpress-note.pdf',
    ], null, 0],
    'large 60p' => [['large-60p.pdf'], null, 0],
    'owner-only AES-256 (allowed)' => [['enc-owner-aes256.pdf'], null, 0, 'allowOwner'],
    'user pw AES-256' => [['enc-user-aes256.pdf'], EncryptedPdfException::class, PdfMergeException::PASSWORD_PROTECTED],
    'user pw RC4-40' => [['enc-user-rc4.pdf'], EncryptedPdfException::class, PdfMergeException::PASSWORD_PROTECTED],
    'owner-only AES-256' => [['enc-owner-aes256.pdf'], EncryptedPdfException::class, PdfMergeException::OWNER_RESTRICTED],
    'owner-only AES-128' => [['enc-owner-aes128.pdf'], EncryptedPdfException::class, PdfMergeException::OWNER_RESTRICTED],
    'truncated half' => [['bad-truncated-half.pdf'], InvalidPdfException::class, PdfMergeException::CORRUPT],
    'truncated tail (no xref)' => [['bad-truncated-tail.pdf'], InvalidPdfException::class, PdfMergeException::CORRUPT],
    'garbage in middle' => [['bad-garbage-middle.pdf'], InvalidPdfException::class, PdfMergeException::CORRUPT],
    'header only' => [['bad-header-only.pdf'], InvalidPdfException::class, PdfMergeException::CORRUPT],
    'no pages' => [['bad-no-pages.pdf'], InvalidPdfException::class, PdfMergeException::NO_PAGES],
    'zero bytes' => [['bad-zero-bytes.pdf'], InvalidPdfException::class, PdfMergeException::EMPTY],
    'HTML' => [['bad-html.pdf'], InvalidPdfException::class, PdfMergeException::NOT_PDF],
    'PNG' => [['bad-png.pdf'], InvalidPdfException::class, PdfMergeException::NOT_PDF],
    'ZIP' => [['bad-zip.pdf'], InvalidPdfException::class, PdfMergeException::NOT_PDF],
    'page limit' => [['large-60p.pdf'], PdfLimitException::class, PdfMergeException::TOO_MANY_PAGES, 'maxPages'],
    'size limit' => [['large-60p.pdf'], PdfLimitException::class, PdfMergeException::INPUT_TOO_LARGE, 'maxBytes'],
];

$rows = [];
$failures = 0;
foreach ($cases as $name => $case) {
    [$inputs, $expectClass, $expectCode] = $case;
    $option = $case[3] ?? null;
    if (!$exists(...$inputs)) {
        $rows[] = [$name, 'skipped (fixture not generated)', '', '', '', '', '', ''];
        continue;
    }
    $slug = trim((string) preg_replace('/[^a-z0-9]+/', '-', strtolower($name)), '-');
    $merger = new PdfMerger((string) file_get_contents($base));
    if ($option === 'allowOwner') {
        $merger->allowOwnerRestricted(true);
    } elseif ($option === 'maxPages') {
        $merger->maxPages(20);
    } elseif ($option === 'maxBytes') {
        $merger->maxInputBytes(1024 * 1024);
    }

    if ($expectClass !== null) {
        $before = $merger->toPdf();
        $pagesBefore = $merger->pageCount();
        $t = hrtime(true);
        try {
            $merger->append((string) file_get_contents($f($inputs[0])));
            $result = 'NOT THROWN';
        } catch (PdfMergeException $e) {
            $result = (get_class($e) === $expectClass && $e->getCode() === $expectCode)
                ? 'OK ' . (new ReflectionClass($e))->getShortName() . ' #' . $e->getCode()
                : 'WRONG ' . get_class($e) . ' #' . $e->getCode();
            $message = $e->getMessage();
        }
        $ms = (hrtime(true) - $t) / 1e6;
        $unchanged = $merger->pageCount() === $pagesBefore && $merger->toPdf() === $before;
        $ok = str_starts_with($result, 'OK') && $unchanged;
        $failures += $ok ? 0 : 1;
        $rows[] = [$name, $result . ' — "' . ($message ?? '') . '"', $unchanged ? 'state unchanged' : 'STATE CHANGED',
            '', '', '', sprintf('%.1f ms', $ms), $ok ? 'PASS' : 'FAIL'];
        continue;
    }

    $sources = [[$base, 1, pageCount($base)]];
    foreach ($inputs as $input) {
        $sources[] = [$f($input), 1, pageCount($f($input))];
    }
    $inBytes = array_sum(array_map(static fn ($s) => filesize($s[0]), $sources));
    $t = hrtime(true);
    foreach ($inputs as $input) {
        $merger->append((string) file_get_contents($f($input)));
    }
    $pdf = $merger->toPdf();
    $ms = (hrtime(true) - $t) / 1e6;
    $out = "$work/$slug.pdf";
    file_put_contents($out, $pdf);

    $expected = array_sum(array_column($sources, 2));
    $pages = pageCount($out);
    sh('qpdf --check ' . q($out), $qcode);
    $qpdf = match ($qcode) { 0 => 'ok', 3 => 'warnings', default => "ERROR($qcode)" };

    $outInfo = pageInfo($out);
    $page = 0;
    $textBad = $imgBad = $geomBad = 0;
    $maxAe = 0;
    $goldenBad = 0;
    $imagesTotal = 0;
    foreach ($sources as [$src]) {
        $srcInfo = pageInfo($src);
        foreach ($srcInfo as $i => $geom) {
            $page++;
            $o = $outInfo[$page - 1] ?? null;
            if ($o === null || abs($o['w'] - $geom['w']) > 0.5 || abs($o['h'] - $geom['h']) > 0.5 || $o['rot'] !== $geom['rot']) {
                $geomBad++;
            }
            if (pageText($out, $page) !== pageText($src, $i + 1)) {
                $textBad++;
            }
            $n = imageCount($src, $i + 1);
            $imagesTotal += $n;
            if (imageCount($out, $page) !== $n) {
                $imgBad++;
            }
            // Raster: every page of small cases, a sample of the large one.
            if (count($srcInfo) <= 12 || $i % 10 === 0) {
                $a = render($src, $i + 1, "$work/$slug-src-$page");
                $b = render($out, $page, "$work/$slug-out-$page");
                $diff = ae($a, $b);
                $maxAe = $diff < 0 ? -1 : max($maxAe, $diff);
                $golden = "$goldenDir/$slug-p$page.png";
                if ($updateGoldens || !is_file($golden)) {
                    copy($b, $golden);
                } elseif (ae($golden, $b) !== 0) {
                    $goldenBad++;
                }
                @unlink($a);
                @unlink($b);
            }
        }
    }
    $ok = $pages === $expected && $qcode === 0 && $textBad === 0 && $imgBad === 0 && $geomBad === 0 && $maxAe === 0 && $goldenBad === 0;
    $failures += $ok ? 0 : 1;
    $rows[] = [
        $name,
        "$pages/$expected pages" . ($geomBad ? ", $geomBad size/rot mismatch" : ', size+rot ok'),
        "qpdf $qpdf",
        ($textBad ? "text FAIL x$textBad" : 'text ok') . ', ' . ($imgBad ? "images FAIL x$imgBad" : "images ok ($imagesTotal)"),
        'AE ' . $maxAe . ($goldenBad ? ", golden FAIL x$goldenBad" : ', golden ok'),
        sprintf('%.0f ms', $ms),
        sprintf('%s in → %s out', human($inBytes), human(strlen($pdf))),
        $ok ? 'PASS' : 'FAIL',
    ];
}

function human(int $bytes): string
{
    return $bytes >= 1048576 ? sprintf('%.1f MB', $bytes / 1048576) : sprintf('%.0f KB', $bytes / 1024);
}

// ironpress_pdf_merge() with 0, 1 and many inputs.
$fn = [];
try {
    ironpress_pdf_merge([]);
    $fn[] = '[] did not throw';
} catch (InvalidPdfException $e) {
    $fn[] = '[] → InvalidPdfException #' . $e->getCode();
}
$one = ironpress_pdf_merge([(string) file_get_contents($base)]);
file_put_contents("$work/fn-one.pdf", $one);
$fn[] = '[base] → ' . pageCount("$work/fn-one.pdf") . ' pages';
try {
    ironpress_pdf_merge([(string) file_get_contents($base), (string) file_get_contents($f('raw-inherited.pdf')), 'junk']);
    $fn[] = 'junk did not throw';
} catch (InvalidPdfException $e) {
    $fn[] = 'junk at #2 → "' . $e->getMessage() . '"';
}

echo "| case | pages | qpdf --check | text / images | raster (max AE vs source; golden) | time | size | result |\n";
echo "|---|---|---|---|---|---|---|---|\n";
foreach ($rows as $row) {
    echo '| ' . implode(' | ', $row) . " |\n";
}
echo "\nironpress_pdf_merge(): " . implode('; ', $fn) . "\n";
echo $failures === 0 ? "\nALL PASS\n" : "\n$failures FAILED\n";
exit($failures === 0 ? 0 : 1);
