<?php

function require_true(bool $condition, string $message): void
{
    if (!$condition) {
        throw new RuntimeException($message);
    }
}

$htmlPdf = ironpress_html_to_pdf('<h1>Hello</h1><p>World</p>');
require_true(is_string($htmlPdf), 'HTML conversion should return a string');
require_true(str_starts_with($htmlPdf, '%PDF'), 'HTML conversion should return PDF bytes');
file_put_contents(__DIR__ . '/smoke-html.pdf', $htmlPdf);

$directOutput = __DIR__ . '/smoke-direct-html.pdf';
ironpress_html_to_pdf_file('<h1>Direct HTML</h1><p>Saved directly.</p>', $directOutput);
require_true(is_file($directOutput), 'direct HTML conversion should create output file');
require_true(str_starts_with(file_get_contents($directOutput), '%PDF'), 'direct HTML output should be PDF bytes');

$markdownPdf = ironpress_markdown_to_pdf("# Hello\n\nWorld");
require_true(is_string($markdownPdf), 'Markdown conversion should return a string');
require_true(str_starts_with($markdownPdf, '%PDF'), 'Markdown conversion should return PDF bytes');
file_put_contents(__DIR__ . '/smoke-md.pdf', $markdownPdf);

$inputHtml = __DIR__ . '/smoke-input.html';
$outputHtml = __DIR__ . '/smoke-file-html.pdf';
file_put_contents($inputHtml, '<h1>File HTML</h1><p>Generated from file.</p>');
ironpress_convert_file($inputHtml, $outputHtml);
require_true(is_file($outputHtml), 'HTML file conversion should create output file');
require_true(str_starts_with(file_get_contents($outputHtml), '%PDF'), 'HTML file output should be PDF bytes');

$inputMarkdown = __DIR__ . '/smoke-input.md';
$outputMarkdown = __DIR__ . '/smoke-file-md.pdf';
file_put_contents($inputMarkdown, "# File Markdown\n\nGenerated from file.");
ironpress_convert_markdown_file($inputMarkdown, $outputMarkdown);
require_true(is_file($outputMarkdown), 'Markdown file conversion should create output file');
require_true(str_starts_with(file_get_contents($outputMarkdown), '%PDF'), 'Markdown file output should be PDF bytes');

require_true(is_string(ironpress_version()), 'version should return a string');
require_true(ironpress_version() !== '', 'version should not be empty');

echo "OK\n";
