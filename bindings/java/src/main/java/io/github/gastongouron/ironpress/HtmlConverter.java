package io.github.gastongouron.ironpress;

import java.util.Objects;

/**
 * A reusable, configured HTML and Markdown to PDF converter.
 *
 * <p>A converter owns one native handle. It may move between threads while idle, but its methods
 * and {@link #close()} must not run concurrently.
 */
public final class HtmlConverter implements AutoCloseable {
  private final ConverterOwner owner = new ConverterOwner();

  /** Create a converter with safe defaults and no resource access. */
  public HtmlConverter() {}

  /** Use a named physical page size. */
  public HtmlConverter setPageSize(PageSize pageSize) {
    owner.setPageSize(Objects.requireNonNull(pageSize, "pageSize"));
    return this;
  }

  /** Use validated custom physical page dimensions. */
  public HtmlConverter setCustomPageSize(PageDimensions dimensions) {
    owner.setCustomPageSize(Objects.requireNonNull(dimensions, "dimensions"));
    return this;
  }

  /** Use one finite physical margin on every page side. */
  public HtmlConverter setMargin(float points) {
    owner.setMargin(PageMargins.uniform(points));
    return this;
  }

  /** Use validated physical page margins in CSS clockwise order. */
  public HtmlConverter setMargins(PageMargins margins) {
    owner.setMargins(Objects.requireNonNull(margins, "margins"));
    return this;
  }

  /** Enable or disable FlateDecode compression. */
  public HtmlConverter setCompression(boolean enabled) {
    owner.setCompression(enabled);
    return this;
  }

  /** Set JPEG quality from 0 to 255; values above 100 are clamped by the renderer. */
  public HtmlConverter setJpegQuality(int quality) {
    if (quality < 0 || quality > 255) {
      throw new IllegalArgumentException("JPEG quality must be between 0 and 255.");
    }
    owner.setJpegQuality(quality);
    return this;
  }

  /** Enable or disable automatic downscaling of oversized images. */
  public HtmlConverter setAutomaticImageResize(boolean enabled) {
    owner.setAutomaticImageResize(enabled);
    return this;
  }

  /** Set target source-image resolution in dots per inch. */
  public HtmlConverter setImageResolution(float dotsPerInch) {
    owner.setResolution(ConverterOwner.ResolutionKind.IMAGE, dotsPerInch);
    return this;
  }

  /** Set CSS filter rasterization resolution in dots per inch. */
  public HtmlConverter setFilterResolution(float dotsPerInch) {
    owner.setResolution(ConverterOwner.ResolutionKind.FILTER, dotsPerInch);
    return this;
  }

  /** Set CSS mask rasterization resolution in dots per inch. */
  public HtmlConverter setMaskResolution(float dotsPerInch) {
    owner.setResolution(ConverterOwner.ResolutionKind.MASK, dotsPerInch);
    return this;
  }

  /** Set flattened-background resolution in dots per inch. */
  public HtmlConverter setBackgroundResolution(float dotsPerInch) {
    owner.setResolution(ConverterOwner.ResolutionKind.BACKGROUND, dotsPerInch);
    return this;
  }

  /** Enable or disable conservative raster occlusion culling. */
  public HtmlConverter setOcclusionCulling(boolean enabled) {
    owner.setOcclusionCulling(enabled);
    return this;
  }

  /** Enable or disable HTML sanitization. */
  public HtmlConverter setSanitization(boolean enabled) {
    owner.setSanitization(enabled);
    return this;
  }

  /**
   * Enable or disable accessible output: a Tagged PDF conforming to PDF/UA-1. The document needs a
   * title and a language, from the HTML or from {@link #setDocumentTitle} and {@link
   * #setDocumentLanguage}.
   */
  public HtmlConverter setAccessibility(boolean enabled) {
    owner.setAccessibility(enabled);
    return this;
  }

  /** Set the document title of accessible output; implies accessibility. */
  public HtmlConverter setDocumentTitle(String title) {
    owner.setTextOption(ConverterOwner.TextOptionKind.DOCUMENT_TITLE, title);
    return this;
  }

  /** Set the document language of accessible output as a BCP 47 tag; implies accessibility. */
  public HtmlConverter setDocumentLanguage(String tag) {
    owner.setTextOption(ConverterOwner.TextOptionKind.DOCUMENT_LANGUAGE, tag);
    return this;
  }

  /** Set plain text rendered in the top page margin. */
  public HtmlConverter setHeader(String text) {
    owner.setTextOption(ConverterOwner.TextOptionKind.HEADER, text);
    return this;
  }

  /** Set an HTML fragment rendered in the top margin of every page. */
  public HtmlConverter setHeaderHtml(String html) {
    owner.setTextOption(ConverterOwner.TextOptionKind.HEADER_HTML, html);
    return this;
  }

  /** Set footer text, with optional {@code {page}} and {@code {pages}} placeholders. */
  public HtmlConverter setFooter(String text) {
    owner.setTextOption(ConverterOwner.TextOptionKind.FOOTER, text);
    return this;
  }

  /** Set an HTML fragment rendered in the bottom margin of every page. */
  public HtmlConverter setFooterHtml(String html) {
    owner.setTextOption(ConverterOwner.TextOptionKind.FOOTER_HTML, html);
    return this;
  }

  /** Add or replace one custom TrueType font family. */
  public HtmlConverter addFont(String family, byte[] fontData) {
    Objects.requireNonNull(family, "family");
    if (family.isEmpty()) {
      throw new IllegalArgumentException("Font family must not be empty.");
    }
    Objects.requireNonNull(fontData, "fontData");
    if (fontData.length == 0) {
      throw new IllegalArgumentException("Font data must not be empty.");
    }
    owner.addFont(family, fontData);
    return this;
  }

  /** Parse and install one optional CJK or emoji fallback pack. */
  public HtmlConverter addFontPack(FontPackKind kind, byte[] fontData) {
    owner.addFontPack(
        Objects.requireNonNull(kind, "kind"), Objects.requireNonNull(fontData, "fontData"));
    return this;
  }

  /** Convert Java HTML text to owned PDF bytes. */
  public byte[] convertHtml(String html) {
    return owner.convert(ConverterOwner.DocumentKind.HTML, html);
  }

  /** Convert Java Markdown text to owned PDF bytes. */
  public byte[] convertMarkdown(String markdown) {
    return owner.convert(ConverterOwner.DocumentKind.MARKDOWN, markdown);
  }

  /** Release the native converter deterministically. */
  @Override
  public void close() {
    owner.close();
  }
}
