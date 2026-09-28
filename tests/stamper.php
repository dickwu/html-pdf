<?php

function require_true(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}

function require_throws(callable $fn, string $needle, string $message): void
{
    try {
        $fn();
    } catch (Throwable $e) {
        require_true(str_contains($e->getMessage(), $needle), $message . ' — got: ' . $e->getMessage());
        return;
    }
    throw new RuntimeException($message . ' — nothing was thrown');
}

$template = ironpress_html_to_pdf('<h1>Form</h1><p>Template page.</p>');

$stamper = new Ironpress\PdfStamper($template);
require_true($stamper->pageCount() === 1, 'template should have one page');
$size = $stamper->pageSize(1);
require_true(abs($size[0] - 595.28) < 0.5 && abs($size[1] - 841.89) < 0.5, 'A4 page size expected, got ' . json_encode($size));

$stamper->text(1, 40.0, 100.0, 'Surname: Doe');
$stamper->text(1, 300.0, 100.0, 'Centré é€', 10.0, 'Helvetica-Bold', 'center');
$stamper->text(1, 40.0, 120.0, 'A long value that must shrink to fit its rule', 9.0, null, null, 120.0);
$stamper->check(1, 40.0, 140.0, 10.8);
require_true($stamper->pendingOps() === 4, 'four operations queued');

$filled = $stamper->toPdf();
require_true(is_string($filled) && str_starts_with($filled, '%PDF'), 'toPdf should return PDF bytes');
require_true($filled === $stamper->toPdf(), 'toPdf must be repeatable');
// lopdf re-serializes the document (and compresses streams the template left
// plain), so the size can shrink; the overlay's font resource is the proof.
require_true(str_contains($filled, '/IPStampH') && str_contains($filled, '/IPStampHB'), 'overlay registers its fonts');
require_true(!str_contains($template, '/IPStampH'), 'template has no stamp fonts');
file_put_contents(__DIR__ . '/stamper-filled.pdf', $filled);

$reloaded = new Ironpress\PdfStamper($filled);
require_true($reloaded->pageCount() === 1, 'filled PDF is still one page');

$stamper->reset();
require_true($stamper->pendingOps() === 0, 'reset clears the queue');
require_true($stamper->toPdf() !== $filled, 'after reset the template renders without the overlay');

require_throws(fn () => new Ironpress\PdfStamper('not a pdf'), 'cannot parse PDF', 'garbage input is rejected');
require_throws(fn () => $stamper->text(2, 1.0, 1.0, 'x'), 'out of range', 'page range is validated');
require_throws(fn () => $stamper->text(0, 1.0, 1.0, 'x'), 'positive integer', 'page must be >= 1');
require_throws(fn () => $stamper->text(1, 1.0, 1.0, "\u{1403}"), 'U+1403', 'non-WinAnsi text is rejected');
require_throws(fn () => $stamper->text(1, 1.0, 1.0, 'far too long for five points', 9.0, null, null, 5.0), 'does not fit', 'max width failure is reported');
require_throws(fn () => $stamper->text(1, 1.0, 1.0, 'x', 9.0, 'Comic Sans'), 'unknown font', 'font names are validated');
require_throws(fn () => $stamper->text(1, 1.0, 1.0, 'x', 9.0, null, 'justify'), 'unknown align', 'align is validated');
require_throws(fn () => $stamper->check(1, 1.0, 1.0, 0.0), 'positive', 'check size is validated');
require_true($stamper->pendingOps() === 0, 'rejected operations are not queued');

echo "stamper smoke ok\n";
