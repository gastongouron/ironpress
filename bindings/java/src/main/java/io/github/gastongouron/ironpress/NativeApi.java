package io.github.gastongouron.ironpress;

import com.sun.jna.Library;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;
import io.github.gastongouron.ironpress.internal.NativeBytes;
import io.github.gastongouron.ironpress.internal.SizeT;

/** Exact JNA projection of C ABI generation 1. */
interface NativeApi extends Library {
  int ironpress_abi_version();

  Pointer ironpress_version();

  int ironpress_converter_new(PointerByReference converter, PointerByReference error);

  int ironpress_converter_free(PointerByReference converter);

  int ironpress_converter_set_page_size(Pointer converter, int pageSize, PointerByReference error);

  int ironpress_converter_set_page_size_custom(
      Pointer converter, float width, float height, PointerByReference error);

  int ironpress_converter_set_margin(Pointer converter, float points, PointerByReference error);

  int ironpress_converter_set_margins(
      Pointer converter,
      float top,
      float right,
      float bottom,
      float left,
      PointerByReference error);

  int ironpress_converter_set_compress(Pointer converter, byte enabled, PointerByReference error);

  int ironpress_converter_set_jpeg_quality(
      Pointer converter, byte quality, PointerByReference error);

  int ironpress_converter_set_auto_resize_images(
      Pointer converter, byte enabled, PointerByReference error);

  int ironpress_converter_set_image_dpi(Pointer converter, float dpi, PointerByReference error);

  int ironpress_converter_set_filter_dpi(Pointer converter, float dpi, PointerByReference error);

  int ironpress_converter_set_mask_dpi(Pointer converter, float dpi, PointerByReference error);

  int ironpress_converter_set_background_raster_dpi(
      Pointer converter, float dpi, PointerByReference error);

  int ironpress_converter_set_occlusion_cull(
      Pointer converter, byte enabled, PointerByReference error);

  int ironpress_converter_set_sanitize(Pointer converter, byte enabled, PointerByReference error);

  int ironpress_converter_set_accessible(Pointer converter, byte enabled, PointerByReference error);

  int ironpress_converter_set_document_title(
      Pointer converter, NativeBytes title, PointerByReference error);

  int ironpress_converter_set_document_language(
      Pointer converter, NativeBytes tag, PointerByReference error);

  int ironpress_converter_set_header(
      Pointer converter, NativeBytes header, PointerByReference error);

  int ironpress_converter_set_header_html(
      Pointer converter, NativeBytes header, PointerByReference error);

  int ironpress_converter_set_footer(
      Pointer converter, NativeBytes footer, PointerByReference error);

  int ironpress_converter_set_footer_html(
      Pointer converter, NativeBytes footer, PointerByReference error);

  int ironpress_converter_add_font(
      Pointer converter, NativeBytes family, NativeBytes fontData, PointerByReference error);

  int ironpress_converter_add_font_pack(
      Pointer converter, int kind, NativeBytes fontData, PointerByReference error);

  int ironpress_converter_convert_html(
      Pointer converter, NativeBytes html, PointerByReference pdf, PointerByReference error);

  int ironpress_converter_convert_markdown(
      Pointer converter, NativeBytes markdown, PointerByReference pdf, PointerByReference error);

  Pointer ironpress_buffer_data(Pointer buffer);

  SizeT ironpress_buffer_len(Pointer buffer);

  int ironpress_buffer_free(PointerByReference buffer);

  Pointer ironpress_error_message_data(Pointer error);

  SizeT ironpress_error_message_len(Pointer error);

  int ironpress_error_free(PointerByReference error);
}
