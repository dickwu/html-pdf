#!/usr/bin/env bash
set -euo pipefail

: "${VERSION:?VERSION is required}"
: "${URL:?URL is required}"
: "${SHA256:?SHA256 is required}"

cat <<RUBY
class HtmlPdf < Formula
  desc "PHP extension for Ironpress HTML and Markdown PDF rendering"
  homepage "https://github.com/dickwu/html-pdf"
  url "${URL}"
  version "${VERSION}"
  sha256 "${SHA256}"
  license "MIT"

  depends_on "llvm" => :build
  depends_on "rust" => :build
  depends_on "php@8.3"

  def install
    php = Formula["php@8.3"]
    ENV["PHP"] = php.opt_bin/"php"
    ENV["PHP_CONFIG"] = php.opt_bin/"php-config"
    ENV["LIBCLANG_PATH"] = Formula["llvm"].opt_lib.to_s

    system "cargo", "build", "--jobs", ENV.make_jobs.to_s, "--lib", "--release", "--locked"

    extension = OS.mac? ? "dylib" : "so"
    (lib/"php/extensions").install "target/release/libironpress_php.#{extension}" => "ironpress_php.so"

    (etc/"php/8.3/conf.d").mkpath
    (etc/"php/8.3/conf.d/ext-ironpress_php.ini").write <<~INI
      extension=#{opt_lib}/php/extensions/ironpress_php.so
    INI
  end

  def caveats
    <<~EOS
      The formula writes:
        #{etc}/php/8.3/conf.d/ext-ironpress_php.ini

      Restart long-running PHP processes after install or upgrade.
    EOS
  end

  test do
    php = Formula["php@8.3"].opt_bin/"php"
    output = testpath/"homebrew.pdf"
    script = <<~PHP
      ironpress_html_to_pdf_file("<h1>Homebrew</h1>", "#{output}");
      echo is_file("#{output}") && str_starts_with(file_get_contents("#{output}"), "%PDF") ? "OK" : "FAIL";
    PHP

    assert_equal "OK", shell_output("#{php} -n -d extension=#{lib}/php/extensions/ironpress_php.so -r '#{script}'")
  end
end
RUBY
