#include "ironpress.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require(int condition, const char *message) {
    if (!condition) {
        fprintf(stderr, "%s\n", message);
        exit(EXIT_FAILURE);
    }
}

static IronpressBytes text(const char *value) {
    IronpressBytes bytes = {
        .data = (const uint8_t *)value,
        .len = strlen(value),
    };
    return bytes;
}

static IronpressBytes read_file(const char *path) {
    FILE *file = fopen(path, "rb");
    require(file != NULL, "font fixture could not be opened");
    require(fseek(file, 0, SEEK_END) == 0, "font fixture size could not be read");
    long size = ftell(file);
    require(size > 0, "font fixture is empty");
    require(fseek(file, 0, SEEK_SET) == 0, "font fixture could not be rewound");
    uint8_t *data = malloc((size_t)size);
    require(data != NULL, "font fixture allocation failed");
    require(fread(data, 1, (size_t)size, file) == (size_t)size,
            "font fixture could not be read");
    require(fclose(file) == 0, "font fixture could not be closed");
    IronpressBytes bytes = {.data = data, .len = (size_t)size};
    return bytes;
}

int main(int argc, char **argv) {
    IronpressConverter *converter = NULL;
    IronpressBuffer *pdf = NULL;
    IronpressError *error = NULL;

    require(argc == 3, "expected custom-font and font-pack fixture paths");
    require(ironpress_abi_version() == IRONPRESS_ABI_VERSION,
            "ABI version query disagrees with the header");
    require(strlen(ironpress_version()) > 0, "package version is empty");

    require(ironpress_converter_new(&converter, &error) == IRONPRESS_STATUS_OK,
            "converter allocation failed");
    require(converter != NULL && error == NULL, "converter ownership is invalid");

    require(ironpress_converter_set_page_size(
                converter, IRONPRESS_PAGE_SIZE_LETTER, &error) == IRONPRESS_STATUS_OK,
            "named page size failed");
    require(ironpress_converter_set_page_size_custom(converter, 420.0f, 595.0f,
                                                     &error) == IRONPRESS_STATUS_OK,
            "custom page size failed");
    require(ironpress_converter_set_margin(converter, 24.0f, &error) ==
                IRONPRESS_STATUS_OK,
            "uniform page margin failed");
    require(ironpress_converter_set_margins(converter, 36.0f, 36.0f, 36.0f,
                                            36.0f, &error) == IRONPRESS_STATUS_OK,
            "page margins failed");
    require(ironpress_converter_set_header(converter, text("C ABI"), &error) ==
                IRONPRESS_STATUS_OK,
            "header configuration failed");
    require(ironpress_converter_set_header_html(
                converter, text("<strong>C ABI HTML</strong>"), &error) ==
                IRONPRESS_STATUS_OK,
            "HTML header configuration failed");
    require(ironpress_converter_set_footer(
                converter, text("Page {page} / {pages}"), &error) ==
                IRONPRESS_STATUS_OK,
            "footer configuration failed");
    require(ironpress_converter_set_footer_html(
                converter, text("<em>C ABI HTML footer</em>"), &error) ==
                IRONPRESS_STATUS_OK,
            "HTML footer configuration failed");
    require(ironpress_converter_set_compress(converter, IRONPRESS_TRUE, &error) ==
                IRONPRESS_STATUS_OK,
            "compression configuration failed");
    require(ironpress_converter_set_jpeg_quality(converter, 82, &error) ==
                IRONPRESS_STATUS_OK,
            "JPEG quality configuration failed");
    require(ironpress_converter_set_auto_resize_images(
                converter, IRONPRESS_TRUE, &error) == IRONPRESS_STATUS_OK,
            "image resize configuration failed");
    require(ironpress_converter_set_image_dpi(converter, 240.0f, &error) ==
                IRONPRESS_STATUS_OK,
            "image DPI configuration failed");
    require(ironpress_converter_set_filter_dpi(converter, 192.0f, &error) ==
                IRONPRESS_STATUS_OK,
            "filter DPI configuration failed");
    require(ironpress_converter_set_mask_dpi(converter, 240.0f, &error) ==
                IRONPRESS_STATUS_OK,
            "mask DPI configuration failed");
    require(ironpress_converter_set_background_raster_dpi(
                converter, 160.0f, &error) == IRONPRESS_STATUS_OK,
            "background DPI configuration failed");
    require(ironpress_converter_set_occlusion_cull(
                converter, IRONPRESS_FALSE, &error) == IRONPRESS_STATUS_OK,
            "occlusion configuration failed");
    require(ironpress_converter_set_sanitize(converter, IRONPRESS_TRUE, &error) ==
                IRONPRESS_STATUS_OK,
            "sanitization configuration failed");

    require(ironpress_converter_set_accessible(converter, IRONPRESS_FALSE, &error) ==
                IRONPRESS_STATUS_OK,
            "accessibility configuration failed");
    require(ironpress_converter_set_document_language(converter, text("1234"),
                                                     &error) ==
                IRONPRESS_STATUS_INVALID_ARGUMENT,
            "an invalid language tag was accepted");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "language error release failed");
    require(ironpress_converter_set_document_title(converter, text("  "), &error) ==
                IRONPRESS_STATUS_INVALID_ARGUMENT,
            "a blank document title was accepted");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "title error release failed");

    IronpressBytes custom_font = read_file(argv[1]);
    require(ironpress_converter_add_font(converter, text("ParitySans"), custom_font,
                                        &error) == IRONPRESS_STATUS_OK,
            "custom font configuration failed");
    free((void *)custom_font.data);

    IronpressBytes font_pack = read_file(argv[2]);
    require(ironpress_converter_add_font_pack(
                converter, IRONPRESS_FONT_PACK_EMOJI, font_pack, &error) ==
                IRONPRESS_STATUS_OK,
            "font-pack configuration failed");
    free((void *)font_pack.data);

    require(ironpress_converter_convert_html(
                converter,
                text("<p style=\"font-family:ParitySans\">Hello from C 😀</p>"),
                &pdf, &error) ==
                IRONPRESS_STATUS_OK,
            "HTML conversion failed");
    require(pdf != NULL && error == NULL, "conversion ownership is invalid");
    require(ironpress_buffer_len(pdf) > 4, "PDF output is too short");
    require(memcmp(ironpress_buffer_data(pdf), "%PDF", 4) == 0,
            "conversion did not return a PDF");
    require(ironpress_converter_convert_markdown(converter, text("# Already owned"),
                                                 &pdf, &error) ==
                IRONPRESS_STATUS_OUTPUT_NOT_EMPTY,
            "non-empty PDF output slot was accepted");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "output-slot error release failed");

    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "PDF release failed");
    require(pdf == NULL, "PDF release did not clear the handle");
    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "repeated PDF release was not safe");

    require(ironpress_converter_set_accessible(converter, IRONPRESS_TRUE, &error) ==
                IRONPRESS_STATUS_OK,
            "accessibility enablement failed");
    require(ironpress_converter_set_document_title(converter, text("C contract"),
                                                  &error) == IRONPRESS_STATUS_OK,
            "document title configuration failed");
    require(ironpress_converter_set_document_language(converter, text("en-GB"),
                                                     &error) == IRONPRESS_STATUS_OK,
            "document language configuration failed");
    require(ironpress_converter_convert_html(converter, text("<p>Accessible</p>"),
                                            &pdf, &error) == IRONPRESS_STATUS_OK,
            "accessible conversion failed");
    require(memcmp(ironpress_buffer_data(pdf), "%PDF-1.7", 8) == 0,
            "accessible conversion did not produce a PDF 1.7 document");
    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "accessible PDF release failed");

    const uint8_t invalid_utf8[] = {0xff};
    IronpressBytes invalid = {.data = invalid_utf8, .len = sizeof(invalid_utf8)};
    require(ironpress_converter_convert_html(converter, invalid, &pdf, &error) ==
                IRONPRESS_STATUS_INVALID_UTF8,
            "invalid UTF-8 returned the wrong status");
    require(pdf == NULL && error != NULL, "invalid UTF-8 did not return an error");
    require(ironpress_error_status(error) == IRONPRESS_STATUS_INVALID_UTF8,
            "error handle returned the wrong status");
    require(ironpress_error_message_len(error) > 0, "error message is empty");
    require(ironpress_error_message_data(error) != NULL,
            "error message has no readable bytes");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "error release failed");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "repeated error release was not safe");

    require(ironpress_converter_set_page_size(converter, UINT32_MAX, &error) ==
                IRONPRESS_STATUS_INVALID_ENUM,
            "invalid page-size discriminant returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "enum error release failed");
    require(ironpress_converter_set_sanitize(converter, 2, &error) ==
                IRONPRESS_STATUS_INVALID_ENUM,
            "invalid boolean returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "boolean error release failed");
    require(ironpress_converter_add_font_pack(
                converter, UINT32_MAX, text("not a font"), &error) ==
                IRONPRESS_STATUS_INVALID_ENUM,
            "invalid font-pack discriminant returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "font-pack enum error release failed");
    require(ironpress_converter_add_font_pack(
                converter, IRONPRESS_FONT_PACK_EMOJI, text("not a font"), &error) ==
                IRONPRESS_STATUS_FONT,
            "invalid font-pack bytes returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "font-pack parse error release failed");

    IronpressBytes missing = {.data = NULL, .len = 1};
    require(ironpress_converter_convert_markdown(converter, missing, &pdf, &error) ==
                IRONPRESS_STATUS_INVALID_ARGUMENT,
            "null input range returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "input-range error release failed");
    require(ironpress_html_to_pdf(text("<p>missing output</p>"), NULL, &error) ==
                IRONPRESS_STATUS_INVALID_ARGUMENT,
            "null PDF output slot returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "output-slot error release failed");

    require(ironpress_converter_convert_markdown(converter, text("# Markdown"), &pdf,
                                                 &error) == IRONPRESS_STATUS_OK,
            "Markdown conversion failed");
    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "Markdown PDF release failed");

    require(ironpress_converter_free(&converter) == IRONPRESS_STATUS_OK,
            "converter release failed");
    require(converter == NULL, "converter release did not clear the handle");
    require(ironpress_converter_free(&converter) == IRONPRESS_STATUS_OK,
            "repeated converter release was not safe");
    require(ironpress_converter_convert_html(converter, text("<p>closed</p>"), &pdf,
                                             &error) == IRONPRESS_STATUS_INVALID_HANDLE,
            "null converter returned the wrong status");
    require(ironpress_error_free(&error) == IRONPRESS_STATUS_OK,
            "handle error release failed");

    require(ironpress_html_to_pdf(text("<p>one shot</p>"), &pdf, &error) ==
                IRONPRESS_STATUS_OK,
            "one-shot HTML conversion failed");
    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "one-shot HTML release failed");
    require(ironpress_markdown_to_pdf(text("# one shot"), &pdf, &error) ==
                IRONPRESS_STATUS_OK,
            "one-shot Markdown conversion failed");
    require(ironpress_buffer_free(&pdf) == IRONPRESS_STATUS_OK,
            "one-shot Markdown release failed");

    return EXIT_SUCCESS;
}
