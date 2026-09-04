# frozen_string_literal: true

require "minitest/autorun"
require "open3"

class HomebrewFormulaTest < Minitest::Test
  def formula
    @formula ||= begin
      env = {
        "VERSION" => "9.9.9",
        "URL" => "https://example.com/html-pdf-9.9.9.tar.gz",
        "SHA256" => "0" * 64,
      }
      output, status = Open3.capture2e(env, "bash", "scripts/generate-homebrew-formula.sh")

      assert status.success?, output
      output
    end
  end

  def test_formula_discovers_installed_homebrew_php_versions
    assert_includes formula, "depends_on \"php\""
    refute_includes formula, "depends_on \"php@8.3\""

    assert_includes formula, "def self.installed_homebrew_php_formulae"
    assert_includes formula, "php.versioned_formulae"
    assert_includes formula, "any_version_installed?"
  end

  def test_formula_builds_and_installs_extension_per_php_minor
    assert_includes formula, "installed_homebrew_php_formulae.each do |php|"
    assert_includes formula, "\"--target-dir\", target_dir"
    assert_includes formula, "lib/\"php/\#{php_version}/extensions\""
    assert_includes formula, "etc/\"php/\#{php_version}/conf.d\""
    assert_includes formula, "extension=\#{opt_lib}/php/\#{php_version}/extensions/ironpress_php.so"

    refute_includes formula, "lib/\"php/extensions\""
    refute_includes formula, "etc/\"php/8.3/conf.d\""
  end

  def test_formula_clears_stale_ini_files_before_writing_config
    assert_includes formula, "rm Dir[\"\#{config_dir}/*ironpress_php*.ini\"]"

    removal = formula.index("rm Dir[")
    write = formula.index("config_file.write")

    refute_nil removal, "formula must remove stale ini files"
    refute_nil write, "formula must write the conf.d ini"
    assert_operator removal, :<, write,
                    "Homebrew's Pathname#write raises on an existing file, so stale " \
                    "ini files must be removed before the write"
  end

  def test_formula_smoke_tests_every_installed_php_extension
    assert_includes formula, "installed_homebrew_php_formulae.each do |php|"
    assert_includes formula, "homebrew-\#{php_version}.pdf"
    assert_includes formula, "-d extension=\#{lib}/php/\#{php_version}/extensions/ironpress_php.so"
  end
end
