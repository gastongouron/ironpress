package io.github.gastongouron.ironpress;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;

/** Unique owner of one configured native converter. */
final class ConverterOwner implements AutoCloseable {
  private final NativeApi api;
  private Pointer converter;

  ConverterOwner() {
    api = NativeLibraryLoader.api();
    var owner = new PointerByReference();
    var error = new PointerByReference();
    converter =
        NativeResult.takeConverter(api, api.ironpress_converter_new(owner, error), owner, error);
  }

  void setPageSize(PageSize pageSize) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_page_size(owner, pageSize.nativeValue(), error));
  }

  void setCustomPageSize(PageDimensions dimensions) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_page_size_custom(
                owner, dimensions.width(), dimensions.height(), error));
  }

  void setMargin(PageMargins margin) {
    mutate((owner, error) -> api.ironpress_converter_set_margin(owner, margin.top(), error));
  }

  void setMargins(PageMargins margins) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_margins(
                owner, margins.top(), margins.right(), margins.bottom(), margins.left(), error));
  }

  void setCompression(boolean enabled) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_compress(owner, nativeBoolean(enabled), error));
  }

  void setJpegQuality(int quality) {
    mutate(
        (owner, error) -> api.ironpress_converter_set_jpeg_quality(owner, (byte) quality, error));
  }

  void setAutomaticImageResize(boolean enabled) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_auto_resize_images(owner, nativeBoolean(enabled), error));
  }

  void setResolution(ResolutionKind kind, float dotsPerInch) {
    mutate(
        (owner, error) ->
            switch (kind) {
              case IMAGE -> api.ironpress_converter_set_image_dpi(owner, dotsPerInch, error);
              case FILTER -> api.ironpress_converter_set_filter_dpi(owner, dotsPerInch, error);
              case MASK -> api.ironpress_converter_set_mask_dpi(owner, dotsPerInch, error);
              case BACKGROUND ->
                  api.ironpress_converter_set_background_raster_dpi(owner, dotsPerInch, error);
            });
  }

  void setOcclusionCulling(boolean enabled) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_occlusion_cull(owner, nativeBoolean(enabled), error));
  }

  void setSanitization(boolean enabled) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_sanitize(owner, nativeBoolean(enabled), error));
  }

  void setAccessibility(boolean enabled) {
    mutate(
        (owner, error) ->
            api.ironpress_converter_set_accessible(owner, nativeBoolean(enabled), error));
  }

  void setTextOption(TextOptionKind kind, String text) {
    ensureOpen();
    try (var input = NativeInput.text(text, "text")) {
      mutate(
          (owner, error) ->
              switch (kind) {
                case HEADER -> api.ironpress_converter_set_header(owner, input.bytes(), error);
                case FOOTER -> api.ironpress_converter_set_footer(owner, input.bytes(), error);
                case HEADER_HTML ->
                    api.ironpress_converter_set_header_html(owner, input.bytes(), error);
                case FOOTER_HTML ->
                    api.ironpress_converter_set_footer_html(owner, input.bytes(), error);
                case DOCUMENT_TITLE ->
                    api.ironpress_converter_set_document_title(owner, input.bytes(), error);
                case DOCUMENT_LANGUAGE ->
                    api.ironpress_converter_set_document_language(owner, input.bytes(), error);
              });
    }
  }

  void addFont(String family, byte[] fontData) {
    ensureOpen();
    try (var familyInput = NativeInput.text(family, "family");
        var fontInput = NativeInput.binary(fontData, "fontData")) {
      mutate(
          (owner, error) ->
              api.ironpress_converter_add_font(
                  owner, familyInput.bytes(), fontInput.bytes(), error));
    }
  }

  void addFontPack(FontPackKind kind, byte[] fontData) {
    ensureOpen();
    try (var input = NativeInput.binary(fontData, "fontData")) {
      mutate(
          (owner, error) ->
              api.ironpress_converter_add_font_pack(
                  owner, kind.nativeValue(), input.bytes(), error));
    }
  }

  byte[] convert(DocumentKind kind, String source) {
    ensureOpen();
    try (var input = NativeInput.text(source, "source")) {
      var pdf = new PointerByReference();
      var error = new PointerByReference();
      var status =
          switch (kind) {
            case HTML -> api.ironpress_converter_convert_html(converter, input.bytes(), pdf, error);
            case MARKDOWN ->
                api.ironpress_converter_convert_markdown(converter, input.bytes(), pdf, error);
          };
      return NativeResult.takePdf(api, status, pdf, error);
    }
  }

  @Override
  public void close() {
    if (converter != null) {
      NativeResult.closeConverter(api, converter);
      converter = null;
    }
  }

  private void mutate(NativeMutation mutation) {
    ensureOpen();
    var error = new PointerByReference();
    NativeResult.ensureSuccess(api, mutation.invoke(converter, error), error);
  }

  private void ensureOpen() {
    if (converter == null) {
      throw new IllegalStateException("This Ironpress converter is already closed.");
    }
  }

  private static byte nativeBoolean(boolean value) {
    return value ? (byte) 1 : (byte) 0;
  }

  @FunctionalInterface
  private interface NativeMutation {
    int invoke(Pointer owner, PointerByReference error);
  }

  enum ResolutionKind {
    IMAGE,
    FILTER,
    MASK,
    BACKGROUND
  }

  enum TextOptionKind {
    HEADER,
    FOOTER,
    HEADER_HTML,
    FOOTER_HTML,
    DOCUMENT_TITLE,
    DOCUMENT_LANGUAGE
  }

  enum DocumentKind {
    HTML,
    MARKDOWN
  }
}
