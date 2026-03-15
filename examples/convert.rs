//! Simple CLI to convert HTML or Markdown files to PDF.
//!
//! Usage:
//!   cargo run --example convert -- input.html output.pdf
//!   cargo run --example convert -- input.md output.pdf

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: convert <input.html|input.md> <output.pdf>");
        std::process::exit(1);
    }

    let input = &args[1];
    let output = &args[2];

    let result = if input.ends_with(".md") || input.ends_with(".markdown") {
        ironpress::convert_markdown_file(input, output)
    } else {
        ironpress::convert_file(input, output)
    };

    match result {
        Ok(()) => println!("{input} → {output}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
