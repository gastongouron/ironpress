using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace Ironpress.Interop;

internal static partial class NativeMethods
{
    private const string LibraryName = "ironpress_ffi";

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial uint ironpress_abi_version();

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial nint ironpress_version();

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_new(
        out nint converter,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_free(ref nint converter);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_page_size(
        ConverterHandle converter,
        uint pageSize,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_page_size_custom(
        ConverterHandle converter,
        float width,
        float height,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_margin(
        ConverterHandle converter,
        float points,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_margins(
        ConverterHandle converter,
        float top,
        float right,
        float bottom,
        float left,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_compress(
        ConverterHandle converter,
        byte enabled,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_jpeg_quality(
        ConverterHandle converter,
        byte quality,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_auto_resize_images(
        ConverterHandle converter,
        byte enabled,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_accessible(
        ConverterHandle converter,
        byte enabled,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_document_title(
        ConverterHandle converter,
        NativeBytes title,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_document_language(
        ConverterHandle converter,
        NativeBytes tag,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_image_dpi(
        ConverterHandle converter,
        float dpi,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_filter_dpi(
        ConverterHandle converter,
        float dpi,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_mask_dpi(
        ConverterHandle converter,
        float dpi,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_background_raster_dpi(
        ConverterHandle converter,
        float dpi,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_occlusion_cull(
        ConverterHandle converter,
        byte enabled,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_sanitize(
        ConverterHandle converter,
        byte enabled,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_header(
        ConverterHandle converter,
        NativeBytes header,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_header_html(
        ConverterHandle converter,
        NativeBytes header,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_footer(
        ConverterHandle converter,
        NativeBytes footer,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_set_footer_html(
        ConverterHandle converter,
        NativeBytes footer,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_add_font(
        ConverterHandle converter,
        NativeBytes family,
        NativeBytes fontData,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_add_font_pack(
        ConverterHandle converter,
        uint kind,
        NativeBytes fontData,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_convert_html(
        ConverterHandle converter,
        NativeBytes html,
        out nint pdf,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_converter_convert_markdown(
        ConverterHandle converter,
        NativeBytes markdown,
        out nint pdf,
        out nint error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial nint ironpress_buffer_data(BufferHandle buffer);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial nuint ironpress_buffer_len(BufferHandle buffer);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_buffer_free(ref nint buffer);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial nint ironpress_error_message_data(ErrorHandle error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial nuint ironpress_error_message_len(ErrorHandle error);

    [LibraryImport(LibraryName)]
    [UnmanagedCallConv(CallConvs = [typeof(CallConvCdecl)])]
    internal static partial NativeStatus ironpress_error_free(ref nint error);
}
