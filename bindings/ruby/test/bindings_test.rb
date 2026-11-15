# frozen_string_literal: true

require "minitest/autorun"
require "pathname"
require "tmpdir"
require "ironpress"

class BindingContractTest < Minitest::Test
  REPOSITORY_ROOT = Pathname(__dir__).join("../../..").expand_path
  FONT_PACK = REPOSITORY_ROOT.join("tests/fonts/IronpressCjkVertical.ttf")

  PORTABLE_CONVERTER_METHODS = %i[
    page_size
    page_size_custom
    margin
    margin_sides
    compress
    jpeg_quality
    auto_resize_images
    image_dpi
    filter_dpi
    mask_dpi
    background_raster_dpi
    occlusion_cull
    sanitize
    accessible
    document_title
    document_language
    add_font
    add_font_pack
    header
    header_html
    footer
    footer_html
    convert
    convert_markdown
  ].freeze

  NATIVE_CONVERTER_METHODS = %i[
    base_path
    resource_root
    convert_to_file
    convert_markdown_to_file
  ].freeze

  def test_runtime_version_matches_distribution
    gemspec = Gem::Specification.load(
      REPOSITORY_ROOT.join("bindings/ruby/ironpress.gemspec").to_s
    )

    assert_equal gemspec.version.to_s, Ironpress::VERSION
  end

  def test_converter_exposes_the_portable_and_native_contracts
    converter = Ironpress::HtmlConverter.new

    (PORTABLE_CONVERTER_METHODS + NATIVE_CONVERTER_METHODS).each do |method|
      assert_respond_to converter, method
    end
  end

  def test_portable_options_compose_into_one_conversion
    converter = Ironpress::HtmlConverter.new
      .page_size_custom(320.0, 480.0)
      .margin_sides(12.0, 13.0, 14.0, 15.0)
      .compress(false)
      .jpeg_quality(82)
      .auto_resize_images(false)
      .image_dpi(144.0)
      .filter_dpi(96.0)
      .mask_dpi(144.0)
      .background_raster_dpi(120.0)
      .occlusion_cull(true)
      .sanitize(true)
      .header("Contract header")
      .header_html("<strong>Contract HTML header</strong>")
      .footer("Page {page} of {pages}")
      .footer_html("<em>Contract HTML footer</em>")

    pdf = converter.convert("<h1>Ruby binding</h1>")

    assert pdf.start_with?("%PDF")
    assert_includes pdf, "/MediaBox [0 0 320 480]"

    converter.accessible(true).document_title("Contract").document_language("en-GB")
    assert_includes converter.convert("<p>Accessible</p>"), "/StructTreeRoot"
    assert_raises(ArgumentError) { converter.document_language("1234") }
  end

  def test_font_pack_is_parsed_at_the_binding_boundary
    converter = Ironpress::HtmlConverter.new
      .add_font_pack("cjk-jp", FONT_PACK.binread)

    pdf = converter.convert("<p lang='ja'>\u7b2c</p>")

    assert_includes pdf, "DroidSansFallback"
  end

  def test_invalid_font_pack_reports_the_expected_kinds
    error = assert_raises(ArgumentError) do
      Ironpress::HtmlConverter.new.add_font_pack("unknown", "not a font")
    end

    assert_includes error.message, "cjk-jp, cjk-kr, cjk-sc, cjk-tc, or emoji"
  end

  def test_native_file_output_writes_the_pdf
    Dir.mktmpdir do |directory|
      output = File.join(directory, "document.pdf")
      Ironpress::HtmlConverter.new.convert_to_file("<p>File output</p>", output)
      assert File.binread(output).start_with?("%PDF")
    end
  end
end
