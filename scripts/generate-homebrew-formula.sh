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
  depends_on "php"

  def self.installed_homebrew_php_formulae
    php = Formula["php"]
    ([php] + php.versioned_formulae)
      .select(&:any_version_installed?)
      .uniq { |formula| formula.opt_bin.to_s }
  end

  def install
    ENV["LIBCLANG_PATH"] = Formula["llvm"].opt_lib.to_s

    extension = OS.mac? ? "dylib" : "so"

    self.class.installed_homebrew_php_formulae.each do |php|
      php_version = php.version.major_minor.to_s
      target_dir = buildpath/"target/homebrew-php-#{php_version}"

      ENV["PHP"] = php.opt_bin/"php"
      ENV["PHP_CONFIG"] = php.opt_bin/"php-config"

      system "cargo", "build", "--jobs", ENV.make_jobs.to_s, "--lib", "--release", "--locked", "--target-dir", target_dir

      extension_dir = lib/"php/#{php_version}/extensions"
      extension_dir.install target_dir/"release/libironpress_php.#{extension}" => "ironpress_php.so"

      config_dir = etc/"php/#{php_version}/conf.d"
      config_dir.mkpath
      (config_dir/"ext-ironpress_php.ini").write <<~INI
        extension=#{opt_lib}/php/#{php_version}/extensions/ironpress_php.so
      INI
    end
  end

  def caveats
    config_paths = self.class.installed_homebrew_php_formulae
      .map { |php| "        #{etc}/php/#{php.version.major_minor}/conf.d/ext-ironpress_php.ini" }
      .join("\n")

    <<~EOS
      The formula writes one ini file for each installed Homebrew PHP:
#{config_paths}

      Reinstall html-pdf after installing another Homebrew PHP version.
      Restart long-running PHP processes after install or upgrade.
    EOS
  end

  test do
    self.class.installed_homebrew_php_formulae.each do |php|
      php_version = php.version.major_minor.to_s
      output = testpath/"homebrew-#{php_version}.pdf"
      script = <<~PHP
        ironpress_html_to_pdf_file("<h1>Homebrew</h1>", "#{output}");
        echo is_file("#{output}") && str_starts_with(file_get_contents("#{output}"), "%PDF") ? "OK" : "FAIL";
      PHP

      assert_equal "OK", shell_output("#{php.opt_bin/"php"} -n -d extension=#{lib}/php/#{php_version}/extensions/ironpress_php.so -r '#{script}'")
    end
  end
end
RUBY
