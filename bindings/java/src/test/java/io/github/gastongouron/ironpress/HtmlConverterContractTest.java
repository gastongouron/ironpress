package io.github.gastongouron.ironpress;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

final class HtmlConverterContractTest {
  private static final Path CUSTOM_FONT = requiredFixture("ironpress.customFont");
  private static final Path FONT_PACK = requiredFixture("ironpress.fontPack");

  @Test
  void nativeContractIdentifiesThePackageAndAbi() {
    assertEquals(1, IronpressInfo.abiVersion());
    assertEquals(IronpressInfo.PACKAGE_VERSION, IronpressInfo.version());
  }

  @Test
  void portableOptionsComposeIntoOneConversion() throws IOException {
    try (var converter =
        new HtmlConverter()
            .setPageSize(PageSize.LETTER)
            .setCustomPageSize(PageDimensions.ofPoints(320, 480))
            .setMargins(PageMargins.ofPoints(12, 13, 14, 15))
            .setCompression(false)
            .setJpegQuality(82)
            .setAutomaticImageResize(false)
            .setImageResolution(144)
            .setFilterResolution(96)
            .setMaskResolution(144)
            .setBackgroundResolution(120)
            .setOcclusionCulling(true)
            .setSanitization(true)
            .setHeader("Contract header")
            .setHeaderHtml("<strong>Contract HTML header</strong>")
            .setFooter("Page {page} of {pages}")
            .setFooterHtml("<em>Contract HTML footer</em>")
            .addFont("ParitySans", Files.readAllBytes(CUSTOM_FONT))
            .addFontPack(FontPackKind.CJK_JAPANESE, Files.readAllBytes(FONT_PACK))) {
      var pdf =
          converter.convertHtml(
              "<h1 style='font-family:ParitySans'>Java binding</h1><p lang='ja'>第</p>");
      assertPdf(pdf);
      assertContains(pdf, "/MediaBox [0 0 320 480]");
      converter.setAccessibility(true).setDocumentTitle("Contract").setDocumentLanguage("en-GB");
      assertContains(converter.convertHtml("<p>Accessible</p>"), "/StructTreeRoot");
      assertPdf(converter.convertMarkdown("# Markdown binding"));
    }
  }

  @Test
  void fontPacksCrossTheManagedBoundary() throws IOException {
    try (var converter =
        new HtmlConverter().addFontPack(FontPackKind.CJK_JAPANESE, Files.readAllBytes(FONT_PACK))) {
      assertContains(converter.convertHtml("<p lang='ja'>第</p>"), "DroidSansFallback");
    }
  }

  @Test
  void nativeFailuresRetainTheirCategory() {
    try (var converter = new HtmlConverter()) {
      var fontError =
          assertThrows(
              IronpressException.class,
              () ->
                  converter.addFontPack(
                      FontPackKind.EMOJI, "not a font".getBytes(StandardCharsets.UTF_8)));
      assertEquals(IronpressErrorKind.FONT, fontError.getKind());

      assertThrows(IllegalArgumentException.class, () -> PageDimensions.ofPoints(0, 100));
      assertThrows(IllegalArgumentException.class, () -> converter.setMargin(Float.NaN));
      var languageError =
          assertThrows(IronpressException.class, () -> converter.setDocumentLanguage("1234"));
      assertEquals(IronpressErrorKind.INVALID_ARGUMENT, languageError.getKind());
      assertThrows(IllegalArgumentException.class, () -> converter.setHeader("\ud800"));

      var argumentError =
          assertThrows(IronpressException.class, () -> converter.setImageResolution(Float.NaN));
      assertEquals(IronpressErrorKind.INVALID_ARGUMENT, argumentError.getKind());
    }
  }

  @Test
  void closedConvertersRejectFurtherWork() {
    var converter = new HtmlConverter();
    converter.close();
    converter.close();
    assertThrows(IllegalStateException.class, () -> converter.convertHtml("<p>too late</p>"));
  }

  @Test
  void equivalentConvertersProduceIdenticalBytes() {
    try (var first = new HtmlConverter().setCompression(false).setMargin(24);
        var second = new HtmlConverter().setCompression(false).setMargin(24)) {
      var source = "<h1>Deterministic contract</h1>";
      assertArrayEquals(first.convertHtml(source), second.convertHtml(source));
    }
  }

  @Test
  void repeatedOwnershipCyclesRemainValid() {
    for (var iteration = 0; iteration < 25; iteration++) {
      try (var converter = new HtmlConverter()) {
        assertPdf(converter.convertHtml("<p>cycle " + iteration + "</p>"));
      }
    }
  }

  private static Path requiredFixture(String property) {
    var value = System.getProperty(property);
    if (value == null || value.isBlank()) {
      throw new IllegalStateException("Missing required test property: " + property);
    }
    return Path.of(value);
  }

  private static void assertPdf(byte[] bytes) {
    assertTrue(bytes.length >= 4);
    assertEquals("%PDF", new String(bytes, 0, 4, StandardCharsets.US_ASCII));
  }

  private static void assertContains(byte[] bytes, String expected) {
    assertTrue(new String(bytes, StandardCharsets.ISO_8859_1).contains(expected));
  }
}
