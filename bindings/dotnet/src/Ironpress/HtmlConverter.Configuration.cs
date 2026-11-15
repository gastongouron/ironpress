using System.Diagnostics;
using Ironpress.Interop;

namespace Ironpress;

public sealed partial class HtmlConverter
{
    /// <summary>Use a named physical page size.</summary>
    public HtmlConverter SetPageSize(PageSize pageSize)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_page_size(
            converter,
            (uint)pageSize,
            out var error);
        return Complete(status, error);
    }

    /// <summary>Use validated custom physical page dimensions.</summary>
    public HtmlConverter SetCustomPageSize(PageDimensions dimensions)
    {
        ArgumentNullException.ThrowIfNull(dimensions);
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_page_size_custom(
            converter,
            dimensions.Width,
            dimensions.Height,
            out var error);
        return Complete(status, error);
    }

    /// <summary>Use one finite physical margin on every page side.</summary>
    public HtmlConverter SetMargin(float points)
    {
        var margin = PageMargins.Uniform(points);
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_margin(
            converter,
            margin.Top,
            out var error);
        return Complete(status, error);
    }

    /// <summary>Use validated physical page margins in CSS clockwise order.</summary>
    public HtmlConverter SetMargins(PageMargins margins)
    {
        ArgumentNullException.ThrowIfNull(margins);
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_margins(
            converter,
            margins.Top,
            margins.Right,
            margins.Bottom,
            margins.Left,
            out var error);
        return Complete(status, error);
    }

    /// <summary>Enable or disable FlateDecode compression.</summary>
    public HtmlConverter SetCompression(bool enabled)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_compress(
            converter,
            ToNativeBoolean(enabled),
            out var error);
        return Complete(status, error);
    }

    /// <summary>Set JPEG quality from 0 to 100; larger values are clamped.</summary>
    public HtmlConverter SetJpegQuality(byte quality)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_jpeg_quality(
            converter,
            quality,
            out var error);
        return Complete(status, error);
    }

    /// <summary>Enable or disable automatic downscaling of oversized images.</summary>
    public HtmlConverter SetAutomaticImageResize(bool enabled)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_auto_resize_images(
            converter,
            ToNativeBoolean(enabled),
            out var error);
        return Complete(status, error);
    }

    /// <summary>Set target source-image resolution in dots per inch.</summary>
    public HtmlConverter SetImageResolution(float dotsPerInch) =>
        SetResolution(ResolutionKind.Image, dotsPerInch);

    /// <summary>Set CSS filter rasterization resolution in dots per inch.</summary>
    public HtmlConverter SetFilterResolution(float dotsPerInch) =>
        SetResolution(ResolutionKind.Filter, dotsPerInch);

    /// <summary>Set CSS mask rasterization resolution in dots per inch.</summary>
    public HtmlConverter SetMaskResolution(float dotsPerInch) =>
        SetResolution(ResolutionKind.Mask, dotsPerInch);

    /// <summary>Set flattened-background resolution in dots per inch.</summary>
    public HtmlConverter SetBackgroundResolution(float dotsPerInch) =>
        SetResolution(ResolutionKind.Background, dotsPerInch);

    /// <summary>Enable or disable conservative raster occlusion culling.</summary>
    public HtmlConverter SetOcclusionCulling(bool enabled)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_occlusion_cull(
            converter,
            ToNativeBoolean(enabled),
            out var error);
        return Complete(status, error);
    }

    /// <summary>Enable or disable HTML sanitization.</summary>
    public HtmlConverter SetSanitization(bool enabled)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_sanitize(
            converter,
            ToNativeBoolean(enabled),
            out var error);
        return Complete(status, error);
    }

    /// <summary>
    /// Enable or disable accessible output: a Tagged PDF conforming to PDF/UA-1.
    /// The document needs a title and a language, from the HTML or from
    /// <see cref="SetDocumentTitle"/> and <see cref="SetDocumentLanguage"/>.
    /// </summary>
    public HtmlConverter SetAccessibility(bool enabled)
    {
        EnsureOpen();
        var status = NativeMethods.ironpress_converter_set_accessible(
            converter,
            ToNativeBoolean(enabled),
            out var error);
        return Complete(status, error);
    }

    /// <summary>Set the document title of accessible output; implies accessibility.</summary>
    public HtmlConverter SetDocumentTitle(string title) =>
        SetTextOption(TextOptionKind.DocumentTitle, title);

    /// <summary>
    /// Set the document language of accessible output as a BCP 47 tag; implies accessibility.
    /// </summary>
    public HtmlConverter SetDocumentLanguage(string tag) =>
        SetTextOption(TextOptionKind.DocumentLanguage, tag);

    /// <summary>Set plain text rendered in the top page margin.</summary>
    public HtmlConverter SetHeader(string text) => SetTextOption(TextOptionKind.Header, text);

    /// <summary>Set an HTML fragment rendered in the top margin of every page.</summary>
    public HtmlConverter SetHeaderHtml(string html) => SetTextOption(TextOptionKind.HeaderHtml, html);

    /// <summary>Set footer text, with optional {page} and {pages} placeholders.</summary>
    public HtmlConverter SetFooter(string text) => SetTextOption(TextOptionKind.Footer, text);

    /// <summary>Set an HTML fragment rendered in the bottom margin of every page.</summary>
    public HtmlConverter SetFooterHtml(string html) => SetTextOption(TextOptionKind.FooterHtml, html);

    private HtmlConverter SetResolution(ResolutionKind kind, float dotsPerInch)
    {
        EnsureOpen();
        nint error;
        var status = kind switch
        {
            ResolutionKind.Image => NativeMethods.ironpress_converter_set_image_dpi(
                converter, dotsPerInch, out error),
            ResolutionKind.Filter => NativeMethods.ironpress_converter_set_filter_dpi(
                converter, dotsPerInch, out error),
            ResolutionKind.Mask => NativeMethods.ironpress_converter_set_mask_dpi(
                converter, dotsPerInch, out error),
            ResolutionKind.Background => NativeMethods.ironpress_converter_set_background_raster_dpi(
                converter, dotsPerInch, out error),
            _ => throw new UnreachableException(),
        };
        return Complete(status, error);
    }

    private unsafe HtmlConverter SetTextOption(TextOptionKind kind, string text)
    {
        EnsureOpen();
        var encoded = Utf8Input.Encode(text, nameof(text));
        fixed (byte* data = encoded)
        {
            var input = new NativeBytes((nint)data, (nuint)encoded.Length);
            nint error;
            var status = kind switch
            {
                TextOptionKind.Header => NativeMethods.ironpress_converter_set_header(
                    converter, input, out error),
                TextOptionKind.Footer => NativeMethods.ironpress_converter_set_footer(
                    converter, input, out error),
                TextOptionKind.HeaderHtml => NativeMethods.ironpress_converter_set_header_html(
                    converter, input, out error),
                TextOptionKind.FooterHtml => NativeMethods.ironpress_converter_set_footer_html(
                    converter, input, out error),
                TextOptionKind.DocumentTitle => NativeMethods.ironpress_converter_set_document_title(
                    converter, input, out error),
                TextOptionKind.DocumentLanguage => NativeMethods.ironpress_converter_set_document_language(
                    converter, input, out error),
                _ => throw new UnreachableException(),
            };
            return Complete(status, error);
        }
    }

    private static byte ToNativeBoolean(bool value) => value ? (byte)1 : (byte)0;

    private enum ResolutionKind
    {
        Image,
        Filter,
        Mask,
        Background,
    }

    private enum TextOptionKind
    {
        Header,
        Footer,
        HeaderHtml,
        FooterHtml,
        DocumentTitle,
        DocumentLanguage,
    }
}
