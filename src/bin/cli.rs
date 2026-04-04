use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let opts = match ironpress::cli::parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    if opts.help {
        print!("{}", ironpress::cli::HELP);
        return;
    }
    if opts.version {
        println!("ironpress {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Determine input content
    let (content, is_markdown) = if opts.from_stdin {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf).unwrap_or_else(|e| {
            eprintln!("Failed to read stdin: {e}");
            process::exit(1);
        });
        (buf, false)
    } else {
        if opts.positional.len() < 2 {
            eprintln!("Missing arguments. Usage: ironpress <input> <output.pdf>");
            process::exit(1);
        }
        let input_path = &opts.positional[0];
        let content = std::fs::read_to_string(input_path).unwrap_or_else(|e| {
            eprintln!("Failed to read {input_path}: {e}");
            process::exit(1);
        });
        let md = input_path.ends_with(".md") || input_path.ends_with(".markdown");
        (content, md)
    };

    let output_path = if opts.from_stdin {
        opts.positional.first().unwrap_or_else(|| {
            eprintln!("Missing output file. Usage: ironpress --stdin <output.pdf>");
            process::exit(1);
        })
    } else {
        &opts.positional[1]
    };

    // Convert
    let pdf = if is_markdown {
        ironpress::cli::convert_markdown(&opts, &content)
    } else {
        ironpress::cli::convert(&opts, &content)
    };

    let pdf = pdf.unwrap_or_else(|e| {
        eprintln!("Conversion failed: {e}");
        process::exit(1);
    });

    std::fs::write(output_path, pdf).unwrap_or_else(|e| {
        eprintln!("Failed to write {output_path}: {e}");
        process::exit(1);
    });

    if !opts.from_stdin {
        eprintln!("{} → {output_path}", opts.positional[0]);
    }
}
