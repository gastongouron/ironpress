from importlib.metadata import version
from pathlib import Path
import tempfile
import unittest

import ironpress


REPOSITORY_ROOT = Path(__file__).parents[3]
FONT_PACK = REPOSITORY_ROOT / "tests" / "fonts" / "IronpressCjkVertical.ttf"

PORTABLE_CONVERTER_METHODS = {
    "page_size",
    "page_size_custom",
    "margin",
    "margin_sides",
    "compress",
    "jpeg_quality",
    "auto_resize_images",
    "image_dpi",
    "filter_dpi",
    "mask_dpi",
    "background_raster_dpi",
    "occlusion_cull",
    "sanitize",
    "accessible",
    "document_title",
    "document_language",
    "add_font",
    "add_font_pack",
    "header",
    "header_html",
    "footer",
    "footer_html",
    "convert",
    "convert_markdown",
}

NATIVE_CONVERTER_METHODS = {
    "base_path",
    "resource_root",
    "convert_to_file",
    "convert_markdown_to_file",
}


class BindingContractTest(unittest.TestCase):
    def test_runtime_version_matches_distribution(self):
        self.assertEqual(ironpress.__version__, version("ironpress"))

    def test_converter_exposes_the_portable_and_native_contracts(self):
        converter = ironpress.HtmlConverter()

        for method in PORTABLE_CONVERTER_METHODS | NATIVE_CONVERTER_METHODS:
            with self.subTest(method=method):
                self.assertTrue(callable(getattr(converter, method, None)))

    def test_portable_options_compose_into_one_conversion(self):
        converter = ironpress.HtmlConverter()
        converter.page_size_custom(320.0, 480.0)
        converter.margin_sides(12.0, 13.0, 14.0, 15.0)
        converter.compress(False)
        converter.jpeg_quality(82)
        converter.auto_resize_images(False)
        converter.image_dpi(144.0)
        converter.filter_dpi(96.0)
        converter.mask_dpi(144.0)
        converter.background_raster_dpi(120.0)
        converter.occlusion_cull(True)
        converter.sanitize(True)
        converter.header("Contract header")
        converter.header_html("<strong>Contract HTML header</strong>")
        converter.footer("Page {page} of {pages}")
        converter.footer_html("<em>Contract HTML footer</em>")

        pdf = converter.convert("<h1>Python binding</h1>")

        self.assertTrue(pdf.startswith(b"%PDF"))
        self.assertIn(b"/MediaBox [0 0 320 480]", pdf)

        converter.accessible(True)
        converter.document_title("Contract")
        converter.document_language("en-GB")
        self.assertIn(b"/StructTreeRoot", converter.convert("<p>Accessible</p>"))
        with self.assertRaises(ValueError):
            converter.document_language("1234")

    def test_font_pack_is_parsed_at_the_binding_boundary(self):
        converter = ironpress.HtmlConverter()
        converter.add_font_pack("cjk-jp", FONT_PACK.read_bytes())

        pdf = converter.convert("<p lang='ja'>\u7b2c</p>")

        self.assertIn(b"DroidSansFallback", pdf)

    def test_invalid_font_pack_reports_the_expected_kinds(self):
        converter = ironpress.HtmlConverter()

        with self.assertRaisesRegex(ValueError, "cjk-jp, cjk-kr, cjk-sc, cjk-tc, or emoji"):
            converter.add_font_pack("unknown", b"not a font")

    def test_native_file_output_writes_the_pdf(self):
        converter = ironpress.HtmlConverter()

        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "document.pdf"
            converter.convert_to_file("<p>File output</p>", str(output))
            self.assertTrue(output.read_bytes().startswith(b"%PDF"))


if __name__ == "__main__":
    unittest.main()
