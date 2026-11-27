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
    assert_includes formula, "supported_php_formulae.each do |php|"
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

  def test_formula_smoke_tests_every_supported_php_extension
    assert_includes formula, "supported_php_formulae.each do |php|"
    assert_includes formula, "homebrew-\#{php_version}.pdf"
    assert_includes formula, "-d extension=\#{lib}/php/\#{php_version}/extensions/ironpress_php.so"
  end

  def test_formula_skips_php_versions_the_extension_does_not_support
    assert_includes formula, "MIN_SUPPORTED_PHP = Version.new(\"8.0\")"
    assert_includes formula, "MAX_SUPPORTED_PHP = Version.new(\"8.5\")"
    assert_includes formula, "def self.supported_php_formulae"
    assert_includes formula, "self.class.supported_php_formulae.each do |php|"
    assert_includes formula, "self.class.unsupported_php_formulae.each do |php|"

    # The build, caveats and smoke test only walk the supported list.
    refute_includes formula, "self.class.installed_homebrew_php_formulae.each"
    assert_equal 1, formula.scan("installed_homebrew_php_formulae.select").length
    # install and the smoke test both iterate the supported list.
    assert_equal 2, formula.scan("self.class.supported_php_formulae.each do |php|").length
  end

  # Loads the generated class against minimal Homebrew stubs so the selection
  # logic runs for real, not only as text.
  def load_formula_class
    stubs = <<~RUBY
      class Version
        include Comparable
        attr_reader :parts

        def initialize(text)
          @parts = text.split(".").map(&:to_i)
        end

        def <=>(other)
          parts <=> other.parts
        end

        def major_minor
          Version.new(parts.first(2).join("."))
        end

        def to_s
          parts.join(".")
        end

        def freeze
          self
        end
      end

      class Formula
        def self.test(*) = nil
        def self.method_missing(*) = nil
        def self.respond_to_missing?(*) = true
      end
    RUBY
    scope = Module.new
    scope.module_eval(stubs)
    scope.module_eval(formula)
    scope.const_get(:HtmlPdf)
  end

  FakePhp = Struct.new(:full_name, :version)

  def test_supported_selection_excludes_php_8_6_and_keeps_8_5_and_older
    klass = load_formula_class
    fake = lambda do |name, version|
      FakePhp.new(name, klass::MIN_SUPPORTED_PHP.class.new(version))
    end
    installed = [fake.call("php", "8.6.0"), fake.call("php@8.5", "8.5.13"),
                 fake.call("php@8.3", "8.3.37"), fake.call("php@7.4", "7.4.33"),
                 fake.call("php@8.10", "8.10.1")]
    klass.define_singleton_method(:installed_homebrew_php_formulae) { installed }

    assert_equal %w[php@8.5 php@8.3], klass.supported_php_formulae.map(&:full_name)
    assert_equal %w[php php@7.4 php@8.10], klass.unsupported_php_formulae.map(&:full_name)
  end
end
