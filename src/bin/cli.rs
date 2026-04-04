use std::process;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return;
    }

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("ironpress {VERSION}");
        return;
    }

    let mut page_size = ironpress::PageSize::A4;
    let mut landscape = false;
    let mut margin = ironpress::Margin::default();
    let mut header: Option<String> = None;
    let mut footer: Option<String> = None;
    let mut sanitize = true;
    let mut from_stdin = false;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--page-size" => {
                i += 1;
                page_size = match args.get(i).map(|s| s.to_ascii_lowercase()).as_deref() {
                    Some("letter") => ironpress::PageSize::LETTER,
                    Some("legal") => ironpress::PageSize::LEGAL,
                    Some("a4") => ironpress::PageSize::A4,
                    Some(other) => {
                        eprintln!("Unknown page size: {other}. Use a4, letter, or legal.");
                        process::exit(1);
                    }
                    None => {
                        eprintln!("--page-size requires a value");
                        process::exit(1);
                    }
                };
            }
            "--landscape" => landscape = true,
            "--margin" => {
                i += 1;
                let val: f32 = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--margin requires a numeric value in points");
                    process::exit(1);
                });
                margin = ironpress::Margin::uniform(val);
            }
            "--header" => {
                i += 1;
                header = Some(args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--header requires a value");
                    process::exit(1);
                }));
            }
            "--footer" => {
                i += 1;
                footer = Some(args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--footer requires a value");
                    process::exit(1);
                }));
            }
            "--sanitize" => {
                i += 1;
                sanitize = args
                    .get(i)
                    .map(|s| s != "false" && s != "0")
                    .unwrap_or(true);
            }
            "--stdin" => from_stdin = true,
            arg if arg.starts_with('-') => {
                eprintln!("Unknown option: {arg}");
                process::exit(1);
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    if landscape {
        page_size = ironpress::PageSize::new(page_size.height, page_size.width);
    }

    // Determine input and output
    let (input_html, output_path) = if from_stdin {
        let output = positional.first().unwrap_or_else(|| {
            eprintln!("Missing output file. Usage: ironpress --stdin <output.pdf>");
            process::exit(1);
        });
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).unwrap_or_else(|e| {
            eprintln!("Failed to read stdin: {e}");
            process::exit(1);
        });
        (buf, output.clone())
    } else {
        if positional.len() < 2 {
            eprintln!("Missing arguments. Usage: ironpress <input> <output.pdf>");
            process::exit(1);
        }
        let input_path = &positional[0];
        let output = positional[1].clone();
        let content = std::fs::read_to_string(input_path).unwrap_or_else(|e| {
            eprintln!("Failed to read {input_path}: {e}");
            process::exit(1);
        });

        // Convert markdown to HTML if needed
        let html = if input_path.ends_with(".md") || input_path.ends_with(".markdown") {
            ironpress::HtmlConverter::new()
                .page_size(page_size)
                .margin(margin)
                .sanitize(sanitize)
                .convert_markdown(&content)
                .unwrap_or_else(|e| {
                    eprintln!("Conversion failed: {e}");
                    process::exit(1);
                });
            // For markdown, we go through the builder directly and write the output
            let mut converter = ironpress::HtmlConverter::new()
                .page_size(page_size)
                .margin(margin)
                .sanitize(sanitize);
            if let Some(ref h) = header {
                converter = converter.header(h.as_str());
            }
            if let Some(ref f) = footer {
                converter = converter.footer(f.as_str());
            }
            let pdf = converter.convert_markdown(&content).unwrap_or_else(|e| {
                eprintln!("Conversion failed: {e}");
                process::exit(1);
            });
            std::fs::write(&output, pdf).unwrap_or_else(|e| {
                eprintln!("Failed to write {output}: {e}");
                process::exit(1);
            });
            eprintln!("{input_path} → {output}");
            return;
        } else {
            content
        };
        (html, output)
    };

    // Build converter and run
    let mut converter = ironpress::HtmlConverter::new()
        .page_size(page_size)
        .margin(margin)
        .sanitize(sanitize);
    if let Some(ref h) = header {
        converter = converter.header(h.as_str());
    }
    if let Some(ref f) = footer {
        converter = converter.footer(f.as_str());
    }

    let pdf = converter.convert(&input_html).unwrap_or_else(|e| {
        eprintln!("Conversion failed: {e}");
        process::exit(1);
    });

    std::fs::write(&output_path, pdf).unwrap_or_else(|e| {
        eprintln!("Failed to write {output_path}: {e}");
        process::exit(1);
    });

    if !from_stdin {
        eprintln!("{} → {output_path}", positional[0]);
    }
}
