# frozen_string_literal: true

require "rbconfig"
require_relative "ironpress/version"

ruby_api_version = RUBY_VERSION.split(".").first(2).join(".")
versioned_extension = File.join(__dir__, "ironpress", ruby_api_version, "ironpress_ruby")
source_extension = File.join(__dir__, "ironpress", "ironpress_ruby")
extension_suffix = RbConfig::CONFIG.fetch("DLEXT")
extension = if File.file?("#{versioned_extension}.#{extension_suffix}")
              versioned_extension
            else
              source_extension
            end
require extension

module Ironpress
  # Mutable Ruby facade over immutable native converter values.
  class HtmlConverter
    CONFIGURATION_METHODS = %i[
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
      base_path
      resource_root
      header
      header_html
      footer
      footer_html
    ].freeze

    def initialize
      @native = NativeConverter.new
    end

    CONFIGURATION_METHODS.each do |name|
      define_method(name) do |*arguments|
        @native = @native.public_send(name, *arguments)
        self
      end
    end

    def convert(html)
      @native.convert(html)
    end

    def convert_markdown(markdown)
      @native.convert_markdown(markdown)
    end

    def convert_to_file(html, path)
      @native.convert_to_file(html, path)
      self
    end

    def convert_markdown_to_file(markdown, path)
      @native.convert_markdown_to_file(markdown, path)
      self
    end
  end

  private_constant :NativeConverter
end
