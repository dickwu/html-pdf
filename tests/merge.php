<?php
/**
 * Fast PdfMerger tests: no external tools. Inputs are ironpress output plus
 * small hand-built PDFs.
 *
 *   php -d extension=target/release/libironpress_php.dylib tests/merge.php
 */

declare(strict_types=1);

use Ironpress\EncryptedPdfException;
use Ironpress\InvalidPdfException;
use Ironpress\PdfLimitException;
use Ironpress\PdfMergeException;
use Ironpress\PdfMerger;

$failed = 0;
function check(string $name, bool $ok): void
{
    global $failed;
    echo ($ok ? 'ok   ' : 'FAIL ') . $name . "\n";
    $failed += $ok ? 0 : 1;
}

function pages(string $pdf): int
{
    return (new PdfMerger($pdf))->pageCount();
}

function expectThrow(string $name, callable $fn, string $class, int $code): void
{
    try {
        $fn();
        check("$name throws $class", false);
    } catch (PdfMergeException $e) {
        check("$name throws " . (new ReflectionClass($class))->getShortName() . " #$code",
            $e instanceof $class && $e->getCode() === $code && $e instanceof Exception);
    }
}

$a = ironpress_html_to_pdf('<h1>MARK-A</h1><p>first</p>');
$b = ironpress_html_to_pdf('<h1>MARK-B</h1>' . str_repeat('<p>Lorem ipsum dolor sit amet.</p>', 120));
$bPages = pages($b);
check('fixture B spans several pages', $bPages > 1);

$m = new PdfMerger($a);
check('base page count', $m->pageCount() === 1);
check('append returns pages added', $m->append($b) === $bPages);
check('append self again', $m->append($a) === 1);
$merged = $m->toPdf();
check('merged starts with %PDF-', str_starts_with($merged, '%PDF-'));
check('merged page count', pages($merged) === 2 + $bPages);
check('merged output re-merges', (new PdfMerger($merged))->append($merged) === 2 + $bPages);
check('toPdf is repeatable', $m->toPdf() === $merged);

check('function: one PDF', pages(ironpress_pdf_merge([$a])) === 1);
check('function: many PDFs', pages(ironpress_pdf_merge([$a, $b, $a, $b])) === 2 + 2 * $bPages);
expectThrow('function: empty list', fn () => ironpress_pdf_merge([]), InvalidPdfException::class, PdfMergeException::EMPTY);
try {
    ironpress_pdf_merge([$a, $b, 'nope']);
} catch (InvalidPdfException $e) {
    check('function: error names the index', str_contains($e->getMessage(), 'PDF #2'));
}

// Rejected inputs leave the merger untouched.
$bad = [
    'zero bytes' => ['', InvalidPdfException::class, PdfMergeException::EMPTY],
    'HTML' => ['<html><body>hi</body></html>', InvalidPdfException::class, PdfMergeException::NOT_PDF],
    'PNG' => ["\x89PNG\r\n\x1a\n" . str_repeat("\0", 64), InvalidPdfException::class, PdfMergeException::NOT_PDF],
    'truncated' => [substr($b, 0, intdiv(strlen($b), 2)), InvalidPdfException::class, PdfMergeException::CORRUPT],
    'header only' => ["%PDF-1.7\n", InvalidPdfException::class, PdfMergeException::CORRUPT],
];
$encDir = __DIR__ . '/fixtures/merge';
foreach (['enc-user-aes256.pdf' => PdfMergeException::PASSWORD_PROTECTED,
          'enc-owner-aes256.pdf' => PdfMergeException::OWNER_RESTRICTED] as $file => $code) {
    if (is_file("$encDir/$file")) {
        $bad[$file] = [(string) file_get_contents("$encDir/$file"), EncryptedPdfException::class, $code];
    } else {
        echo "skip $file (committed fixture missing; regenerate with tests/fixtures/merge/generate.py)\n";
    }
}
$m = new PdfMerger($a);
$m->append($b);
$before = $m->toPdf();
foreach ($bad as $name => [$pdf, $class, $code]) {
    expectThrow("append $name", fn () => $m->append($pdf), $class, $code);
    expectThrow("construct $name", fn () => new PdfMerger($pdf), $class, $code);
}
check('state unchanged after rejected appends', $m->pageCount() === 1 + $bPages && $m->toPdf() === $before);

if (isset($bad['enc-owner-aes256.pdf'])) {
    $m->allowOwnerRestricted(true);
    check('owner-restricted accepted when allowed', $m->append($bad['enc-owner-aes256.pdf'][0]) >= 1);
}

// Limits.
$m = new PdfMerger($a);
$m->maxPages(2);
expectThrow('maxPages', fn () => $m->append($b), PdfLimitException::class, PdfMergeException::TOO_MANY_PAGES);
check('maxPages leaves state unchanged', $m->pageCount() === 1);
check('maxPages allows up to the limit', $m->append($a) === 1);
$m->maxInputBytes(strlen($a) - 1);
expectThrow('maxInputBytes', fn () => $m->append($a), PdfLimitException::class, PdfMergeException::INPUT_TOO_LARGE);
foreach ([0, -1] as $v) {
    try {
        $m->maxPages($v);
        check("maxPages($v) rejected", false);
    } catch (Exception $e) {
        check("maxPages($v) rejected", !$e instanceof PdfMergeException);
    }
}

echo $failed === 0 ? "\nall merge tests passed\n" : "\n$failed merge tests FAILED\n";
exit($failed === 0 ? 0 : 1);
