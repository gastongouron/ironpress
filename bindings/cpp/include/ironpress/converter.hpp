#pragma once

#include "ironpress/pdf.hpp"

#include <cstdint>
#include <string_view>
#include <utility>

namespace ironpress {

/// A reusable configured converter with unique native ownership.
class Converter final {
public:
    /// Allocate a default converter after verifying the linked ABI generation.
    Converter() {
        detail::NativeResult::require_compatible_abi();
        IronpressConverter* converter = nullptr;
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_new(&converter, &error);
        detail::ConverterOwner owner(converter);
        detail::NativeResult::require_success(status, error);
        if (!owner) {
            throw detail::NativeResult::contract_violation(
                "Native converter allocation returned no owner.");
        }
        owner_ = std::move(owner);
    }

    Converter(const Converter&) = delete;
    Converter& operator=(const Converter&) = delete;
    Converter(Converter&&) noexcept = default;
    Converter& operator=(Converter&&) noexcept = default;
    ~Converter() noexcept = default;

    /// Report whether this object still owns a native converter.
    [[nodiscard]] explicit operator bool() const noexcept {
        return static_cast<bool>(owner_);
    }

    /// Configure one named physical page size.
    Converter& set_page_size(PageSize page_size) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_set_page_size(
            owner_.get(), static_cast<std::uint32_t>(page_size), &error);
        return configured(status, error);
    }

    /// Configure one custom physical page size.
    Converter& set_page_size(PageDimensions dimensions) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_set_page_size_custom(
            owner_.get(), dimensions.width(), dimensions.height(), &error);
        return configured(status, error);
    }

    /// Configure physical page margins.
    Converter& set_margins(PageMargins margins) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_set_margins(
            owner_.get(), margins.top(), margins.right(), margins.bottom(),
            margins.left(), &error);
        return configured(status, error);
    }

    /// Enable or disable PDF compression.
    Converter& set_compression(bool enabled) {
        return configure_boolean(ironpress_converter_set_compress, enabled);
    }

    /// Set JPEG quality; the renderer clamps values above 100.
    Converter& set_jpeg_quality(std::uint8_t quality) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_set_jpeg_quality(
            owner_.get(), quality, &error);
        return configured(status, error);
    }

    /// Enable or disable automatic source-image downscaling.
    Converter& set_auto_resize_images(bool enabled) {
        return configure_boolean(ironpress_converter_set_auto_resize_images,
                                 enabled);
    }

    /// Set target source-image resolution in dots per inch.
    Converter& set_image_dpi(float dpi) {
        return configure_number(ironpress_converter_set_image_dpi, dpi);
    }

    /// Set CSS filter rasterization resolution in dots per inch.
    Converter& set_filter_dpi(float dpi) {
        return configure_number(ironpress_converter_set_filter_dpi, dpi);
    }

    /// Set CSS mask rasterization resolution in dots per inch.
    Converter& set_mask_dpi(float dpi) {
        return configure_number(ironpress_converter_set_mask_dpi, dpi);
    }

    /// Set flattened-background rasterization resolution in dots per inch.
    Converter& set_background_raster_dpi(float dpi) {
        return configure_number(ironpress_converter_set_background_raster_dpi,
                                dpi);
    }

    /// Enable or disable conservative raster occlusion culling.
    Converter& set_occlusion_culling(bool enabled) {
        return configure_boolean(ironpress_converter_set_occlusion_cull,
                                 enabled);
    }

    /// Enable or disable HTML sanitization.
    Converter& set_sanitization(bool enabled) {
        return configure_boolean(ironpress_converter_set_sanitize, enabled);
    }

    /// Enable or disable accessible output: a Tagged PDF conforming to
    /// PDF/UA-1. The document needs a title and a language, from the HTML or
    /// from the two setters below.
    Converter& set_accessibility(bool enabled) {
        return configure_boolean(ironpress_converter_set_accessible, enabled);
    }

    /// Set the document title of accessible output; implies accessibility.
    Converter& set_document_title(std::string_view title) {
        return configure_text(ironpress_converter_set_document_title, title);
    }

    /// Set the document language of accessible output as a BCP 47 tag;
    /// implies accessibility.
    Converter& set_document_language(std::string_view tag) {
        return configure_text(ironpress_converter_set_document_language, tag);
    }

    /// Configure the plain-text page header.
    Converter& set_header(std::string_view header) {
        return configure_text(ironpress_converter_set_header, header);
    }

    /// Configure an HTML fragment in the top page margin.
    Converter& set_header_html(std::string_view header) {
        return configure_text(ironpress_converter_set_header_html, header);
    }

    /// Configure the plain-text page footer.
    Converter& set_footer(std::string_view footer) {
        return configure_text(ironpress_converter_set_footer, footer);
    }

    /// Configure an HTML fragment in the bottom page margin.
    Converter& set_footer_html(std::string_view footer) {
        return configure_text(ironpress_converter_set_footer_html, footer);
    }

    /// Add or replace one custom TrueType font family.
    Converter& add_font(std::string_view family, BytesView font_data) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_add_font(
            owner_.get(), detail::text(family), font_data.native(), &error);
        return configured(status, error);
    }

    /// Add one optional CJK or emoji fallback pack.
    Converter& add_font_pack(FontPackKind kind, BytesView font_data) {
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_add_font_pack(
            owner_.get(), static_cast<std::uint32_t>(kind), font_data.native(),
            &error);
        return configured(status, error);
    }

    /// Convert UTF-8 HTML to one uniquely owned PDF buffer.
    [[nodiscard]] Pdf convert_html(std::string_view html) const {
        IronpressBuffer* pdf = nullptr;
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_convert_html(
            owner_.get(), detail::text(html), &pdf, &error);
        return Pdf::take(status, pdf, error);
    }

    /// Convert UTF-8 Markdown to one uniquely owned PDF buffer.
    [[nodiscard]] Pdf convert_markdown(std::string_view markdown) const {
        IronpressBuffer* pdf = nullptr;
        IronpressError* error = nullptr;
        const auto status = ironpress_converter_convert_markdown(
            owner_.get(), detail::text(markdown), &pdf, &error);
        return Pdf::take(status, pdf, error);
    }

private:
    using BooleanSetter = IronpressStatus (*)(IronpressConverter*, std::uint8_t,
                                               IronpressError**);
    using NumberSetter = IronpressStatus (*)(IronpressConverter*, float,
                                              IronpressError**);
    using TextSetter = IronpressStatus (*)(IronpressConverter*, IronpressBytes,
                                            IronpressError**);

    Converter& configured(IronpressStatus status, IronpressError* error) {
        detail::NativeResult::require_success(status, error);
        return *this;
    }

    Converter& configure_boolean(BooleanSetter setter, bool enabled) {
        IronpressError* error = nullptr;
        const auto status = setter(
            owner_.get(), enabled ? IRONPRESS_TRUE : IRONPRESS_FALSE, &error);
        return configured(status, error);
    }

    Converter& configure_number(NumberSetter setter, float value) {
        IronpressError* error = nullptr;
        const auto status = setter(owner_.get(), value, &error);
        return configured(status, error);
    }

    Converter& configure_text(TextSetter setter, std::string_view value) {
        IronpressError* error = nullptr;
        const auto status = setter(owner_.get(), detail::text(value), &error);
        return configured(status, error);
    }

    detail::ConverterOwner owner_;
};

}
