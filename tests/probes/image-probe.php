<?php
/**
 * Probe how data-URI <img> behaves in ironpress_html_to_pdf(): EXIF
 * orientation, large photos, many images near the 10 MB cap, and page
 * splitting. Synthetic images only (ImageMagick).
 *
 *   php -d extension=... tests/probes/image-probe.php OUT_DIR
 *
 * Set IRONPRESS_EXT when the extension is not loaded from php.ini.
 * Each case runs in its own php process under /usr/bin/time -l so peak RSS
 * includes Rust allocations. Results go to OUT_DIR/probe.md, renders to
 * OUT_DIR/*.png.
 */

declare(strict_types=1);

$out = rtrim($argv[1] ?? sys_get_temp_dir() . '/ironpress-image-probe', '/');
$case = $argv[2] ?? null;
@mkdir($out, 0777, true);

function sh(string $cmd): string
{
    return (string) shell_exec($cmd . ' 2>&1');
}

function uri(string $file): string
{
    $mime = str_ends_with($file, '.png') ? 'image/png' : 'image/jpeg';
    return "data:$mime;base64," . base64_encode((string) file_get_contents($file));
}

/** Photo-like noisy JPEG, a red band marks the visual top. */
function photo(string $file, int $w, int $h, int $quality, string $orient = 'TopLeft'): void
{
    if (is_file($file)) {
        return;
    }
    // Draw upright, then store pixels rotated + EXIF tag so a viewer that
    // honours EXIF shows it upright again.
    $rotate = ['TopLeft' => 0, 'BottomRight' => 180, 'RightTop' => 90, 'LeftBottom' => 270][$orient];
    sh(sprintf(
        'magick -size %dx%d plasma:fractal -seed 7 +noise Gaussian -attenuate 0.6 '
        . '-fill red -draw "rectangle 0,0 %d,%d" -fill white -font %s -pointsize %d -gravity north -annotate +0+%d TOP '
        . '-rotate %d -quality %d -orient %s %s',
        $w, $h, $w, intdiv($h, 8), escapeshellarg(dirname(__DIR__) . '/fixtures/MrsSaintDelafield-Regular.ttf'), intdiv($h, 12), intdiv($h, 40),
        (360 - $rotate) % 360, $quality, $orient, escapeshellarg($file)
    ));
    // magick -orient does not reliably write the EXIF tag; set it explicitly.
    $tag = ['TopLeft' => 1, 'BottomRight' => 3, 'RightTop' => 6, 'LeftBottom' => 8][$orient];
    sh(sprintf('exiftool -q -overwrite_original -n -Orientation=%d %s', $tag, escapeshellarg($file)));
}

$html = static fn (string $body, string $css = ''): string =>
    "<html><head><style>body{font-family:Helvetica;font-size:11pt} $css</style></head><body>$body</body></html>";

$cases = [
    'baseline-text' => fn () => $html('<p>No images.</p>'),
    'exif-1' => fn () => $html('<p>EXIF 1 (upright)</p><img src="' . uri("$out/o1.jpg") . '" style="width:300pt">'),
    'exif-3' => fn () => $html('<p>EXIF 3 (180)</p><img src="' . uri("$out/o3.jpg") . '" style="width:300pt">'),
    'exif-6' => fn () => $html('<p>EXIF 6 (90 CW)</p><img src="' . uri("$out/o6.jpg") . '" style="width:300pt">'),
    'exif-8' => fn () => $html('<p>EXIF 8 (90 CCW)</p><img src="' . uri("$out/o8.jpg") . '" style="width:300pt">'),
    'large-1' => fn () => $html('<p>One 4000x3000 photo</p><img src="' . uri("$out/big-1.jpg") . '" style="max-width:100%">'),
    'large-3' => fn () => $html(implode('', array_map(
        fn ($i) => '<p>Photo ' . $i . '</p><img src="' . uri("$out/big-$i.jpg") . '" style="max-width:100%">',
        [1, 2, 3]
    ))),
    'near-cap' => fn () => $html(implode('', array_map(
        fn ($i) => '<img src="' . uri("$out/mid-$i.jpg") . '" style="width:45%;margin:2pt">',
        range(1, 9)
    ))),
    'over-cap' => fn () => $html('<img src="' . uri("$out/huge.jpg") . '" style="max-width:100%">'),
    'taller-than-remaining' => fn () => $html(
        str_repeat('<p>Filler line to use up most of the first page.</p>', 30)
        . '<img src="' . uri("$out/tall.jpg") . '" style="width:200pt">'
        . '<p>MARK-after-image</p>'
    ),
    'taller-than-page' => fn () => $html('<p>Before</p><img src="' . uri("$out/very-tall.jpg") . '" style="width:300pt"><p>MARK-after-image</p>'),
    'table-cells' => fn () => $html(
        '<table style="width:100%">' . str_repeat(
            '<tr><td style="width:50%"><img src="' . uri("$out/o1.jpg") . '" style="max-width:100%"></td>'
            . '<td><img src="' . uri("$out/tall.jpg") . '" style="max-width:100%"></td></tr>',
            4
        ) . '</table><p>MARK-after-table</p>',
        'td{border:1px solid #999;vertical-align:top}'
    ),
    'table-cells-avoid' => fn () => $html(
        '<table style="width:100%">' . str_repeat(
            '<tr><td style="width:50%"><img src="' . uri("$out/o1.jpg") . '" style="max-width:100%"></td>'
            . '<td><img src="' . uri("$out/tall.jpg") . '" style="max-width:100%"></td></tr>',
            4
        ) . '</table><p>MARK-after-table</p>',
        'td{border:1px solid #999;vertical-align:top} tr,td,img{break-inside:avoid;page-break-inside:avoid}'
    ),
    'taller-than-page-capped' => fn () => $html('<p>Before</p><img src="' . uri("$out/very-tall.jpg") . '" style="max-width:100%;max-height:700pt"><p>MARK-after-image</p>'),
    'max-width-100' => fn () => $html('<div style="width:200pt;border:1px solid #000"><img src="' . uri("$out/big-1.jpg") . '" style="max-width:100%"></div>'
        . '<p>Unconstrained:</p><img src="' . uri("$out/o1.jpg") . '">'),
    'png-large' => fn () => $html('<img src="' . uri("$out/big.png") . '" style="max-width:100%">'),
];

if ($case !== null) {
    $pdfFile = "$out/$case.pdf";
    $doc = $cases[$case]();
    $t = hrtime(true);
    try {
        $pdf = ironpress_html_to_pdf($doc);
        file_put_contents($pdfFile, $pdf);
        printf("OK %.0f %d\n", (hrtime(true) - $t) / 1e6, strlen($doc));
    } catch (Throwable $e) {
        printf("ERR %.0f %d %s: %s\n", (hrtime(true) - $t) / 1e6, strlen($doc), get_class($e), $e->getMessage());
    }
    exit(0);
}

// --- fixtures
photo("$out/o1.jpg", 400, 300, 90, 'TopLeft');
photo("$out/o3.jpg", 400, 300, 90, 'BottomRight');
photo("$out/o6.jpg", 400, 300, 90, 'RightTop');
photo("$out/o8.jpg", 400, 300, 90, 'LeftBottom');
// ~5 MB, ~6.5 MB, ~8.5 MB: base64 makes each 4/3 larger, so only the first
// fits under the 10 MiB HTML cap on its own.
foreach ([1 => 88, 2 => 90, 3 => 92] as $i => $q) {
    photo("$out/big-$i.jpg", 4000, 3000, $q);
}
foreach (range(1, 9) as $i) {
    photo("$out/mid-$i.jpg", 1600, 1200, 85);
}
photo("$out/huge.jpg", 5000, 4000, 98);
photo("$out/tall.jpg", 300, 900, 85);
photo("$out/very-tall.jpg", 300, 3000, 85);
if (!is_file("$out/big.png")) {
    sh('magick -size 2000x1500 plasma:fractal -seed 3 ' . escapeshellarg("$out/big.png"));
}

$md = "| case | input | result | time | peak RSS | PDF size | pages | embedded images (pdfimages -list) |\n|---|---|---|---|---|---|---|---|\n";
foreach (array_keys($cases) as $name) {
    // Set IRONPRESS_EXT=/path/to/lib.dylib when the extension is not in php.ini.
    $ext = getenv('IRONPRESS_EXT');
    $res = sh('/usr/bin/time -l ' . escapeshellarg(PHP_BINARY)
        . ($ext ? ' -d extension=' . escapeshellarg($ext) : '')
        . ' ' . escapeshellarg(__FILE__) . ' ' . escapeshellarg($out) . ' ' . escapeshellarg($name));
    preg_match('/^(OK|ERR) (\d+) (\d+)(.*)$/m', $res, $m);
    preg_match('/(\d+)\s+maximum resident set size/', $res, $rss);
    $pdf = "$out/$name.pdf";
    $pages = $size = $imgs = '';
    if (($m[1] ?? '') === 'OK') {
        preg_match('/^Pages:\s+(\d+)/m', sh('pdfinfo ' . escapeshellarg($pdf)), $p);
        $pages = $p[1] ?? '?';
        $size = sprintf('%.2f MB', filesize($pdf) / 1048576);
        $list = array_slice(explode("\n", trim(sh('pdfimages -list ' . escapeshellarg($pdf)))), 2);
        $imgs = implode('<br>', array_map(static function ($l) {
            $c = preg_split('/\s+/', trim($l));
            return "p{$c[0]} {$c[3]}x{$c[4]} {$c[5]} {$c[8]} {$c[14]}";
        }, $list));
        sh('pdftoppm -r 40 -png ' . escapeshellarg($pdf) . ' ' . escapeshellarg("$out/render-$name"));
    }
    $md .= sprintf("| %s | %s | %s | %s ms | %s MB | %s | %s | %s |\n", $name,
        isset($m[3]) ? sprintf('HTML %.2f MB', (int) $m[3] / 1048576) : '', trim(($m[1] ?? 'CRASH') . ($m[4] ?? '')), $m[2] ?? '?',
        isset($rss[1]) ? round((int) $rss[1] / 1048576) : '?', $size, $pages, $imgs);
}
file_put_contents("$out/probe.md", $md);
echo $md;
