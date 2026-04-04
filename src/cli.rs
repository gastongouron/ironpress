//! CLI argument parsing and conversion logic.
//!
//! This module is used by the `ironpress` binary but lives in the library
//! so that all logic is covered by `cargo test`.

use crate::error::IronpressError;
use crate::{HtmlConverter, Margin, PageSize};

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
    /// Footer text (`{page}` and `{pages}` are substituted).
    pub footer: Option<String>,
    /// Enable HTML sanitization.
    pub sanitize: bool,
    /// Read from stdin instead of a file.
    pub from_stdin: bool,
    /// Positional arguments (input, output).
    pub positional: Vec<String>,
    /// Print help and exit.
    pub help: bool,
    /// Print version and exit.
    pub version: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            page_size: PageSize::A4,
            landscape: false,
            margin: Margin::default(),
            header: None,
            footer: None,
            sanitize: true,
            from_stdin: false,
            positional: Vec::new(),
            help: false,
            version: false,
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
            "--header" => {
                i += 1;
                opts.header = Some(args.get(i).ok_or("--header requires a value")?.clone());
            }
            "--footer" => {
                i += 1;
                opts.footer = Some(args.get(i).ok_or("--footer requires a value")?.clone());
            }
            "--sanitize" => {
                i += 1;
                let val = args.get(i).ok_or("--sanitize requires a value")?;
                opts.sanitize = val != "false" && val != "0";
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
        .sanitize(opts.sanitize);

    if let Some(ref h) = opts.header {
        converter = converter.header(h.as_str());
    }
    if let Some(ref f) = opts.footer {
        converter = converter.footer(f.as_str());
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
        .sanitize(opts.sanitize);

    if let Some(ref h) = opts.header {
        converter = converter.header(h.as_str());
    }
    if let Some(ref f) = opts.footer {
        converter = converter.footer(f.as_str());
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
    --footer <TEXT>         Footer text ({page} and {pages} for numbering)
    --sanitize <BOOL>       Enable/disable HTML sanitization (default: true)
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
    fn help_text_contains_usage() {
        assert!(HELP.contains("USAGE:"));
        assert!(HELP.contains("--page-size"));
        assert!(HELP.contains("--stdin"));
    }
}
