//! CLI argument parsing and conversion logic.
//!
//! This module is used by the `ironpress` binary but lives in the library
//! so that all logic is covered by `cargo test`.

use crate::error::IronpressError;
use crate::{HtmlConverter, Margin, PageSize, RasterQuality, TaggedPdf};

/// Parsed CLI options.
#[derive(Debug, Clone)]
pub struct CliOptions {
    /// Page size.
    pub page_size: PageSize,
    /// Landscape orientation.
    pub landscape: bool,
    /// Uniform margin in points.
    pub margin: Margin,
    /// Header text.
    pub header: Option<String>,
    /// Header HTML fragment.
    pub header_html: Option<String>,
    /// Footer text (`{page}` and `{pages}` are substituted).
    pub footer: Option<String>,
    /// Footer HTML fragment.
    pub footer_html: Option<String>,
    /// Enable HTML sanitization.
    pub sanitize: bool,
    /// FlateDecode-compress page content streams (default true).
    pub compress: bool,
    /// JPEG quality for optimized image embedding.
    pub jpeg_quality: u8,
    /// Downscale oversized source images before embedding.
    pub auto_resize_images: bool,
    /// One physical-resolution policy for source images and renderer-owned
    /// raster fallbacks. The CLI flags update individual fields in this
    /// cohesive value.
    pub raster_quality: RasterQuality,
    /// Skip embedding raster images fully covered by a later opaque element.
    pub occlusion_cull: bool,
    /// Read from stdin instead of a file.
    pub from_stdin: bool,
    /// Positional arguments (input, output).
    pub positional: Vec<String>,
    /// Print help and exit.
    pub help: bool,
    /// Print version and exit.
    pub version: bool,
    /// Base directory for resolving relative `@import` / `@font-face` URLs.
    pub base_path: Option<std::path::PathBuf>,
    /// Accessible (Tagged PDF, PDF/UA-1) output; `--accessible`, `--lang` and
    /// `--title` populate it.
    pub tagged_pdf: Option<TaggedPdf>,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            page_size: PageSize::A4,
            landscape: false,
            margin: Margin::default(),
            header: None,
            header_html: None,
            footer: None,
            footer_html: None,
            sanitize: true,
            // The real CLI compresses by default; the in-crate test build renders
            // raw so cli tests can inspect content-stream operators (see lib.rs).
            compress: !cfg!(test),
            jpeg_quality: crate::render::pdf::DEFAULT_JPEG_QUALITY,
            auto_resize_images: true,
            raster_quality: RasterQuality::default(),
            occlusion_cull: false,
            from_stdin: false,
            positional: Vec::new(),
            help: false,
            version: false,
            base_path: None,
            tagged_pdf: None,
        }
    }
}

/// Parse CLI arguments into options.
pub fn parse_args(args: &[String]) -> Result<CliOptions, String> {
    let mut opts = CliOptions::default();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        opts.help = true;
        return Ok(opts);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        opts.version = true;
        return Ok(opts);
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--page-size" => {
                i += 1;
                let val = args.get(i).ok_or("--page-size requires a value")?;
                opts.page_size = match val.to_ascii_lowercase().as_str() {
                    "a4" => PageSize::A4,
                    "letter" => PageSize::LETTER,
                    "legal" => PageSize::LEGAL,
                    other => {
                        return Err(format!(
                            "Unknown page size: {other}. Use a4, letter, or legal."
                        ));
                    }
                };
            }
            "--landscape" => opts.landscape = true,
            "--margin" => {
                i += 1;
                let val = args.get(i).ok_or("--margin requires a value")?;
                let pt: f32 = val
                    .parse()
                    .map_err(|_| format!("--margin requires a number, got: {val}"))?;
                opts.margin = Margin::uniform(pt);
            }
            "--margin-top" => {
                i += 1;
                let val = args.get(i).ok_or("--margin-top requires a value")?;
                opts.margin.top = val
                    .parse()
                    .map_err(|_| format!("--margin-top requires a number, got: {val}"))?;
            }
            "--header" => {
                i += 1;
                opts.header = Some(args.get(i).ok_or("--header requires a value")?.clone());
                opts.header_html = None;
            }
            "--header-html" => {
                i += 1;
                opts.header_html =
                    Some(args.get(i).ok_or("--header-html requires a value")?.clone());
                opts.header = None;
            }
            "--footer" => {
                i += 1;
                opts.footer = Some(args.get(i).ok_or("--footer requires a value")?.clone());
                opts.footer_html = None;
            }
            "--footer-html" => {
                i += 1;
                opts.footer_html =
                    Some(args.get(i).ok_or("--footer-html requires a value")?.clone());
                opts.footer = None;
            }
            "--sanitize" => {
                i += 1;
                let val = args.get(i).ok_or("--sanitize requires a value")?;
                opts.sanitize = val != "false" && val != "0";
            }
            "--compress" => {
                i += 1;
                let val = args.get(i).ok_or("--compress requires a value")?;
                opts.compress = val != "false" && val != "0";
            }
            "--jpeg-quality" => {
                i += 1;
                let val = args.get(i).ok_or("--jpeg-quality requires a value")?;
                let quality: i32 = val
                    .parse()
                    .map_err(|_| format!("--jpeg-quality requires a number, got: {val}"))?;
                opts.jpeg_quality = quality.clamp(0, 100) as u8;
            }
            "--auto-resize-images" => {
                i += 1;
                let val = args.get(i).ok_or("--auto-resize-images requires a value")?;
                opts.auto_resize_images = val != "false" && val != "0";
            }
            "--image-dpi" => {
                i += 1;
                let val = args.get(i).ok_or("--image-dpi requires a value")?;
                let dpi: f32 = val
                    .parse()
                    .map_err(|_| format!("--image-dpi requires a number, got: {val}"))?;
                opts.raster_quality.source_image_dpi =
                    crate::style::raster_quality::raster_dpi_at_least(dpi, 72.0);
            }
            "--filter-dpi" => {
                i += 1;
                let val = args.get(i).ok_or("--filter-dpi requires a value")?;
                let dpi: f32 = val
                    .parse()
                    .map_err(|_| format!("--filter-dpi requires a number, got: {val}"))?;
                opts.raster_quality.filter_dpi =
                    crate::style::raster_quality::raster_dpi_at_least(dpi, 1.0);
            }
            "--mask-dpi" => {
                i += 1;
                let val = args.get(i).ok_or("--mask-dpi requires a value")?;
                let dpi: f32 = val
                    .parse()
                    .map_err(|_| format!("--mask-dpi requires a number, got: {val}"))?;
                opts.raster_quality.mask_dpi =
                    crate::style::raster_quality::raster_dpi_at_least(dpi, 72.0);
            }
            "--background-raster-dpi" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--background-raster-dpi requires a value")?;
                let dpi: f32 = val.parse().map_err(|_| {
                    format!("--background-raster-dpi requires a number, got: {val}")
                })?;
                opts.raster_quality.background_dpi =
                    crate::style::raster_quality::raster_dpi_at_least(
                        dpi,
                        crate::style::raster_quality::CSS_REFERENCE_DPI,
                    );
            }
            "--occlusion-cull" => {
                i += 1;
                let val = args.get(i).ok_or("--occlusion-cull requires a value")?;
                opts.occlusion_cull = val != "false" && val != "0";
            }
            "--base-path" => {
                i += 1;
                let val = args.get(i).ok_or("--base-path requires a value")?;
                opts.base_path = Some(std::path::PathBuf::from(val));
            }
            "--accessible" => {
                opts.tagged_pdf.get_or_insert_with(TaggedPdf::default);
            }
            "--lang" => {
                i += 1;
                let val = args.get(i).ok_or("--lang requires a value")?;
                let language = val.parse().map_err(|error| format!("--lang: {error}"))?;
                opts.tagged_pdf
                    .get_or_insert_with(TaggedPdf::default)
                    .language = Some(language);
            }
            "--title" => {
                i += 1;
                let val = args.get(i).ok_or("--title requires a value")?;
                let title = val.parse().map_err(|error| format!("--title: {error}"))?;
                opts.tagged_pdf.get_or_insert_with(TaggedPdf::default).title = Some(title);
            }
            "--stdin" => opts.from_stdin = true,
            arg if arg.starts_with('-') => {
                return Err(format!("Unknown option: {arg}"));
            }
            _ => opts.positional.push(args[i].clone()),
        }
        i += 1;
    }

    Ok(opts)
}

/// Run the conversion with parsed options and input content.
pub fn convert(opts: &CliOptions, html: &str) -> Result<Vec<u8>, IronpressError> {
    let mut page_size = opts.page_size;
    if opts.landscape {
        page_size = PageSize::new(page_size.height, page_size.width);
    }

    let mut converter = HtmlConverter::new()
        .page_size(page_size)
        .margin(opts.margin)
        .sanitize(opts.sanitize)
        .compress(opts.compress)
        .jpeg_quality(opts.jpeg_quality)
        .auto_resize_images(opts.auto_resize_images)
        .raster_quality(opts.raster_quality)
        .occlusion_cull(opts.occlusion_cull);

    if let Some(ref h) = opts.header {
        converter = converter.header(h.as_str());
    }
    if let Some(ref h) = opts.header_html {
        converter = converter.header_html(h.as_str());
    }
    if let Some(ref f) = opts.footer {
        converter = converter.footer(f.as_str());
    }
    if let Some(ref f) = opts.footer_html {
        converter = converter.footer_html(f.as_str());
    }
    if let Some(ref bp) = opts.base_path {
        converter = converter.base_path(bp);
    }
    if let Some(ref tagged) = opts.tagged_pdf {
        converter = converter.tagged_pdf(tagged.clone());
    }

    converter.convert(html)
}

/// Run the markdown conversion with parsed options.
pub fn convert_markdown(opts: &CliOptions, md: &str) -> Result<Vec<u8>, IronpressError> {
    let mut page_size = opts.page_size;
    if opts.landscape {
        page_size = PageSize::new(page_size.height, page_size.width);
    }

    let mut converter = HtmlConverter::new()
        .page_size(page_size)
        .margin(opts.margin)
        .sanitize(opts.sanitize)
        .compress(opts.compress)
        .jpeg_quality(opts.jpeg_quality)
        .auto_resize_images(opts.auto_resize_images)
        .raster_quality(opts.raster_quality)
        .occlusion_cull(opts.occlusion_cull);

    if let Some(ref h) = opts.header {
        converter = converter.header(h.as_str());
    }
    if let Some(ref h) = opts.header_html {
        converter = converter.header_html(h.as_str());
    }
    if let Some(ref f) = opts.footer {
        converter = converter.footer(f.as_str());
    }
    if let Some(ref f) = opts.footer_html {
        converter = converter.footer_html(f.as_str());
    }
    if let Some(ref bp) = opts.base_path {
        converter = converter.base_path(bp);
    }
    if let Some(ref tagged) = opts.tagged_pdf {
        converter = converter.tagged_pdf(tagged.clone());
    }

    converter.convert_markdown(md)
}

/// Help text.
pub const HELP: &str = "\
ironpress — HTML/CSS/Markdown to PDF converter

USAGE:
    ironpress [OPTIONS] <input> <output>
    ironpress [OPTIONS] --stdin <output>

ARGS:
    <input>     Input file (.html or .md)
    <output>    Output PDF file

OPTIONS:
    --page-size <SIZE>      Page size: a4, letter, legal (default: a4)
    --landscape             Use landscape orientation
    --margin <PT>           Uniform margin in points (default: 72)
    --header <TEXT>         Header text on each page
    --header-html <HTML>    Header HTML fragment on each page
    --footer <TEXT>         Footer text ({page} and {pages} for numbering)
    --footer-html <HTML>    Footer HTML fragment on each page
    --sanitize <BOOL>       Enable/disable HTML sanitization (default: true)
    --compress <BOOL>       FlateDecode-compress content streams (default: true)
    --jpeg-quality <N>      JPEG quality for optimized image embedding (default: 95)
    --auto-resize-images <BOOL>
                            Downscale oversized source images (default: true)
    --image-dpi <DPI>       Target source-image resolution (default: 300)
    --filter-dpi <DPI>      Rasterization DPI for blur/filter bitmaps (default: 300)
    --mask-dpi <DPI>        Rasterization DPI for CSS masks (default: 300)
    --background-raster-dpi <DPI>
                            Rasterization DPI for flattened backgrounds (default: 192)
    --occlusion-cull <BOOL> Skip raster images fully covered by a later opaque element (default: false)
    --base-path <DIR>       Base dir for relative @import / @font-face URLs
    --accessible            Tagged PDF output conforming to PDF/UA-1
    --lang <TAG>            Document language (BCP 47); implies --accessible
    --title <TEXT>          Document title; implies --accessible
    --stdin                 Read HTML from stdin instead of a file
    --version               Print version
    --help                  Print this help
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parse_help() {
        let opts = parse_args(&args("--help")).unwrap();
        assert!(opts.help);
    }

    #[test]
    fn parse_version() {
        let opts = parse_args(&args("--version")).unwrap();
        assert!(opts.version);
    }

    #[test]
    fn parse_short_help() {
        let opts = parse_args(&args("-h")).unwrap();
        assert!(opts.help);
    }

    #[test]
    fn parse_short_version() {
        let opts = parse_args(&args("-V")).unwrap();
        assert!(opts.version);
    }

    #[test]
    fn parse_empty_args_shows_help() {
        let opts = parse_args(&[]).unwrap();
        assert!(opts.help);
    }

    #[test]
    fn parse_basic_args() {
        let opts = parse_args(&args("input.html output.pdf")).unwrap();
        assert_eq!(opts.positional, vec!["input.html", "output.pdf"]);
        assert!(!opts.help);
        assert!(!opts.version);
    }

    #[test]
    fn parse_page_size_letter() {
        let opts = parse_args(&args("--page-size letter input.html out.pdf")).unwrap();
        assert!((opts.page_size.width - 612.0).abs() < 1.0);
    }

    #[test]
    fn parse_page_size_legal() {
        let opts = parse_args(&args("--page-size legal input.html out.pdf")).unwrap();
        assert!((opts.page_size.height - 1008.0).abs() < 1.0);
    }

    #[test]
    fn parse_page_size_a4() {
        let opts = parse_args(&args("--page-size a4 input.html out.pdf")).unwrap();
        assert!((opts.page_size.width - 595.28).abs() < 1.0);
    }

    #[test]
    fn parse_page_size_invalid() {
        let result = parse_args(&args("--page-size tabloid input.html out.pdf"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("tabloid"));
    }

    #[test]
    fn parse_page_size_missing_value() {
        let result = parse_args(&args("--page-size"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_landscape() {
        let opts = parse_args(&args("--landscape input.html out.pdf")).unwrap();
        assert!(opts.landscape);
    }

    #[test]
    fn parse_margin() {
        let opts = parse_args(&args("--margin 54 input.html out.pdf")).unwrap();
        assert!((opts.margin.top - 54.0).abs() < 0.1);
        assert!((opts.margin.left - 54.0).abs() < 0.1);
    }

    #[test]
    fn parse_margin_invalid() {
        let result = parse_args(&args("--margin abc input.html out.pdf"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_margin_missing_value() {
        let result = parse_args(&args("--margin"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_header_footer() {
        let a: Vec<String> = vec![
            "--header",
            "My Doc",
            "--footer",
            "Page {page}",
            "in.html",
            "out.pdf",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let opts = parse_args(&a).unwrap();
        assert_eq!(opts.header.as_deref(), Some("My Doc"));
        assert_eq!(opts.footer.as_deref(), Some("Page {page}"));
    }

    #[test]
    fn parse_rich_header_footer_selects_html_over_earlier_plain_text() {
        let options = vec![
            "--header",
            "plain header",
            "--header-html",
            "<strong>rich header</strong>",
            "--footer",
            "plain footer",
            "--footer-html",
            "<em>rich footer</em>",
            "in.html",
            "out.pdf",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let opts = parse_args(&options).unwrap();

        assert_eq!(opts.header, None);
        assert_eq!(opts.footer, None);
        assert_eq!(
            opts.header_html.as_deref(),
            Some("<strong>rich header</strong>")
        );
        assert_eq!(opts.footer_html.as_deref(), Some("<em>rich footer</em>"));
    }

    #[test]
    fn parse_header_missing_value() {
        let result = parse_args(&args("--header"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_footer_missing_value() {
        let result = parse_args(&args("--footer"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_sanitize_false() {
        let opts = parse_args(&args("--sanitize false input.html out.pdf")).unwrap();
        assert!(!opts.sanitize);
    }

    #[test]
    fn parse_sanitize_zero() {
        let opts = parse_args(&args("--sanitize 0 input.html out.pdf")).unwrap();
        assert!(!opts.sanitize);
    }

    #[test]
    fn parse_sanitize_true() {
        let opts = parse_args(&args("--sanitize true input.html out.pdf")).unwrap();
        assert!(opts.sanitize);
    }

    #[test]
    fn parse_sanitize_missing_value() {
        let result = parse_args(&args("--sanitize"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_stdin() {
        let opts = parse_args(&args("--stdin out.pdf")).unwrap();
        assert!(opts.from_stdin);
        assert_eq!(opts.positional, vec!["out.pdf"]);
    }

    #[test]
    fn parse_unknown_option() {
        let result = parse_args(&args("--bogus input.html out.pdf"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("--bogus"));
    }

    #[test]
    fn parse_all_options() {
        let a: Vec<String> = vec![
            "--page-size",
            "letter",
            "--landscape",
            "--margin",
            "36",
            "--header",
            "Title",
            "--footer",
            "p{page}",
            "--sanitize",
            "false",
            "in.html",
            "out.pdf",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let opts = parse_args(&a).unwrap();
        assert!(opts.landscape);
        assert!(!opts.sanitize);
        assert!((opts.margin.top - 36.0).abs() < 0.1);
        assert_eq!(opts.header.as_deref(), Some("Title"));
        assert_eq!(opts.positional, vec!["in.html", "out.pdf"]);
    }

    #[test]
    fn parse_background_raster_dpi() {
        let opts = parse_args(&args("--background-raster-dpi 300 in.html out.pdf")).unwrap();
        assert_eq!(opts.raster_quality.background_dpi, 300.0);

        let clamped = parse_args(&args("--background-raster-dpi 24 in.html out.pdf")).unwrap();
        assert_eq!(
            clamped.raster_quality.background_dpi,
            crate::style::raster_quality::CSS_REFERENCE_DPI
        );
    }

    #[test]
    fn parse_mask_dpi() {
        let opts = parse_args(&args("--mask-dpi 240 in.html out.pdf")).unwrap();
        assert_eq!(opts.raster_quality.mask_dpi, 240.0);

        let clamped = parse_args(&args("--mask-dpi 24 in.html out.pdf")).unwrap();
        assert_eq!(clamped.raster_quality.mask_dpi, 72.0);
    }

    #[test]
    fn defaults_keep_automatic_jpeg_reencoding_visually_lossless() {
        assert_eq!(
            CliOptions::default().jpeg_quality,
            crate::render::pdf::DEFAULT_JPEG_QUALITY
        );
    }

    #[test]
    fn convert_html_produces_valid_pdf() {
        let opts = CliOptions::default();
        let pdf = convert(&opts, "<h1>Hello</h1><p>World</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_with_landscape() {
        let opts = CliOptions {
            landscape: true,
            ..Default::default()
        };
        let pdf = convert(&opts, "<p>Landscape</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_with_header_footer() {
        let opts = CliOptions {
            header: Some("Header".into()),
            footer: Some("Page {page}".into()),
            ..Default::default()
        };
        let pdf = convert(&opts, "<p>Content</p>").unwrap();
        let content = String::from_utf8_lossy(&pdf);
        assert!(content.contains("Header"));
    }

    #[test]
    fn convert_markdown_produces_valid_pdf() {
        let opts = CliOptions::default();
        let pdf = convert_markdown(&opts, "# Title\n\nParagraph").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_markdown_with_options() {
        let opts = CliOptions {
            page_size: PageSize::LETTER,
            landscape: true,
            header: Some("MD".into()),
            footer: Some("{page}/{pages}".into()),
            ..Default::default()
        };
        let pdf = convert_markdown(&opts, "# Test\n\nContent").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_with_sanitize_disabled() {
        let opts = CliOptions {
            sanitize: false,
            ..Default::default()
        };
        let pdf = convert(&opts, "<script>alert(1)</script><p>Hi</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn convert_with_custom_margin() {
        let opts = CliOptions {
            margin: Margin::uniform(36.0),
            ..Default::default()
        };
        let pdf = convert(&opts, "<p>Tight margins</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF"));
    }

    #[test]
    fn parse_accessible_flags() {
        let opts = parse_args(&args("--accessible in.html out.pdf")).unwrap();
        assert_eq!(opts.tagged_pdf, Some(TaggedPdf::default()));

        let opts = parse_args(&args("--lang en-GB in.html out.pdf")).unwrap();
        let tagged = opts.tagged_pdf.expect("--lang implies --accessible");
        assert_eq!(
            tagged.language.as_ref().map(ToString::to_string).as_deref(),
            Some("en-GB")
        );
        assert_eq!(tagged.title, None);

        let titled: Vec<String> = ["--title", "Quarterly report", "in.html", "out.pdf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let opts = parse_args(&titled).unwrap();
        let title = opts.tagged_pdf.unwrap().title.unwrap();
        assert_eq!(title.as_str(), "Quarterly report");
    }

    #[test]
    fn parse_accessible_flags_reject_bad_values() {
        let error = parse_args(&args("--lang en_GB in.html out.pdf")).unwrap_err();
        assert!(error.starts_with("--lang: "), "{error}");
        assert!(parse_args(&args("--lang")).is_err());

        let blank: Vec<String> = ["--title", " ", "in.html", "out.pdf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let error = parse_args(&blank).unwrap_err();
        assert!(error.starts_with("--title: "), "{error}");
        assert!(parse_args(&args("--title")).is_err());
    }

    #[test]
    fn default_options_keep_output_untagged() {
        assert_eq!(CliOptions::default().tagged_pdf, None);
        let pdf = convert(&CliOptions::default(), "<h1>Hello</h1>").unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(!String::from_utf8_lossy(&pdf).contains("/MarkInfo"));
    }

    #[test]
    fn convert_accessible_writes_the_pdf_ua_identification() {
        let opts = parse_args(&args("--lang en --title Hello in.html out.pdf")).unwrap();
        let pdf = convert(&opts, "<p>Body</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF-1.7"));
        let content = String::from_utf8_lossy(&pdf);
        assert!(
            content.contains("/MarkInfo << /Marked true >>"),
            "{content}"
        );
        assert!(content.contains("/Lang (en)"), "{content}");
        assert!(content.contains("/Title (Hello)"), "{content}");

        let markdown = convert_markdown(&opts, "# Ignored\n\nBody").unwrap();
        assert!(String::from_utf8_lossy(&markdown).contains("/Title (Hello)"));
    }

    #[test]
    fn help_text_contains_usage() {
        assert!(HELP.contains("USAGE:"));
        assert!(HELP.contains("--page-size"));
        assert!(HELP.contains("--stdin"));
        assert!(HELP.contains("--accessible"));
    }
}
