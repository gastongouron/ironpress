/// Parity test framework: renders HTML fixtures to PDF and validates output.
///
/// Each fixture is a complete HTML document loaded via `include_str!`.
/// Tests verify that ironpress produces valid PDFs with reasonable file sizes.
/// The ignored `parity_benchmark_report` test runs all fixtures and prints a
/// markdown summary table with file sizes and render times.
use std::time::Instant;

// ---------------------------------------------------------------------------
// Layer 1: Individual feature fixtures
// ---------------------------------------------------------------------------
const TYPOGRAPHY_HTML: &str = include_str!("fixtures/features/typography.html");
const BOX_MODEL_HTML: &str = include_str!("fixtures/features/box-model.html");
const COLORS_BACKGROUNDS_HTML: &str = include_str!("fixtures/features/colors-backgrounds.html");
const FLEXBOX_HTML: &str = include_str!("fixtures/features/flexbox.html");
const GRID_HTML: &str = include_str!("fixtures/features/grid.html");
const TABLES_HTML: &str = include_str!("fixtures/features/tables.html");
const IMAGES_SVG_HTML: &str = include_str!("fixtures/features/images-svg.html");
const POSITIONING_HTML: &str = include_str!("fixtures/features/positioning.html");
const MATH_HTML: &str = include_str!("fixtures/features/math.html");
const PSEUDO_ELEMENTS_HTML: &str = include_str!("fixtures/features/pseudo-elements.html");
const TRANSFORMS_HTML: &str = include_str!("fixtures/features/transforms.html");
const BACKGROUNDS_ADV_HTML: &str = include_str!("fixtures/features/backgrounds-advanced.html");

// ---------------------------------------------------------------------------
// Layer 2: Combined case fixtures
// ---------------------------------------------------------------------------
const SIMPLE_REPORT_HTML: &str = include_str!("fixtures/combined/simple-report.html");
const INVOICE_HTML: &str = include_str!("fixtures/combined/invoice.html");
const RESUME_HTML: &str = include_str!("fixtures/combined/resume.html");
const ARTICLE_HTML: &str = include_str!("fixtures/combined/article.html");
const MATH_PAPER_HTML: &str = include_str!("fixtures/combined/math-paper.html");
const DASHBOARD_HTML: &str = include_str!("fixtures/combined/dashboard.html");

// ---------------------------------------------------------------------------
// Layer 3: Edge case fixtures
// ---------------------------------------------------------------------------
const DEEP_NESTING_HTML: &str = include_str!("fixtures/edge-cases/deep-nesting.html");
const LONG_TABLE_HTML: &str = include_str!("fixtures/edge-cases/long-table.html");
const PAGE_BREAKS_HTML: &str = include_str!("fixtures/edge-cases/page-breaks.html");
const OVERFLOW_HTML: &str = include_str!("fixtures/edge-cases/overflow.html");
const EMPTY_ELEMENTS_HTML: &str = include_str!("fixtures/edge-cases/empty-elements.html");
const UNICODE_HTML: &str = include_str!("fixtures/edge-cases/unicode.html");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ParityResult {
    fixture: String,
    pdf_size: usize,
    render_time_us: u128,
    valid: bool,
}

fn run_fixture(name: &str, html: &str) -> ParityResult {
    let start = Instant::now();
    let pdf = ironpress::html_to_pdf(html).expect(&format!("Failed to render {}", name));
    let elapsed = start.elapsed().as_micros();
    let valid = pdf_is_valid(&pdf);
    ParityResult {
        fixture: name.to_string(),
        pdf_size: pdf.len(),
        render_time_us: elapsed,
        valid,
    }
}

fn pdf_is_valid(pdf: &[u8]) -> bool {
    if pdf.len() < 64 {
        return false;
    }
    let text = String::from_utf8_lossy(pdf);
    pdf.starts_with(b"%PDF") && text.contains("%%EOF")
}

fn assert_fixture(name: &str, html: &str) {
    let result = run_fixture(name, html);
    assert!(
        result.valid,
        "Fixture '{}' produced an invalid PDF ({} bytes)",
        name, result.pdf_size
    );
    // Sanity: even the simplest fixture should produce more than 100 bytes
    assert!(
        result.pdf_size > 100,
        "Fixture '{}' produced a suspiciously small PDF ({} bytes)",
        name,
        result.pdf_size
    );
}

// ---------------------------------------------------------------------------
// Layer 1 tests: individual features
// ---------------------------------------------------------------------------

#[test]
fn parity_typography() {
    assert_fixture("typography", TYPOGRAPHY_HTML);
}

#[test]
fn parity_box_model() {
    assert_fixture("box-model", BOX_MODEL_HTML);
}

#[test]
fn parity_colors_backgrounds() {
    assert_fixture("colors-backgrounds", COLORS_BACKGROUNDS_HTML);
}

#[test]
fn parity_flexbox() {
    assert_fixture("flexbox", FLEXBOX_HTML);
}

#[test]
fn parity_grid() {
    assert_fixture("grid", GRID_HTML);
}

#[test]
fn parity_tables() {
    assert_fixture("tables", TABLES_HTML);
}

#[test]
fn parity_images_svg() {
    assert_fixture("images-svg", IMAGES_SVG_HTML);
}

#[test]
fn parity_positioning() {
    assert_fixture("positioning", POSITIONING_HTML);
}

#[test]
fn parity_math() {
    assert_fixture("math", MATH_HTML);
}

#[test]
fn parity_pseudo_elements() {
    assert_fixture("pseudo-elements", PSEUDO_ELEMENTS_HTML);
}

#[test]
fn parity_transforms() {
    assert_fixture("transforms", TRANSFORMS_HTML);
}

#[test]
fn parity_backgrounds_advanced() {
    assert_fixture("backgrounds-advanced", BACKGROUNDS_ADV_HTML);
}

// ---------------------------------------------------------------------------
// Layer 2 tests: combined cases
// ---------------------------------------------------------------------------

#[test]
fn parity_simple_report() {
    assert_fixture("simple-report", SIMPLE_REPORT_HTML);
}

#[test]
fn parity_invoice() {
    assert_fixture("invoice", INVOICE_HTML);
}

#[test]
fn parity_resume() {
    assert_fixture("resume", RESUME_HTML);
}

#[test]
fn parity_article() {
    assert_fixture("article", ARTICLE_HTML);
}

#[test]
fn parity_math_paper() {
    assert_fixture("math-paper", MATH_PAPER_HTML);
}

#[test]
fn parity_dashboard() {
    assert_fixture("dashboard", DASHBOARD_HTML);
}

// ---------------------------------------------------------------------------
// Layer 3 tests: edge cases
// ---------------------------------------------------------------------------

#[test]
fn parity_deep_nesting() {
    assert_fixture("deep-nesting", DEEP_NESTING_HTML);
}

#[test]
fn parity_long_table() {
    assert_fixture("long-table", LONG_TABLE_HTML);
}

#[test]
fn parity_page_breaks() {
    assert_fixture("page-breaks", PAGE_BREAKS_HTML);
}

#[test]
fn parity_overflow() {
    assert_fixture("overflow", OVERFLOW_HTML);
}

#[test]
fn parity_empty_elements() {
    assert_fixture("empty-elements", EMPTY_ELEMENTS_HTML);
}

#[test]
fn parity_unicode() {
    assert_fixture("unicode", UNICODE_HTML);
}

// ---------------------------------------------------------------------------
// Benchmark report (run with: cargo test --test parity_tests -- --ignored)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn parity_benchmark_report() {
    let fixtures: Vec<(&str, &str)> = vec![
        // Layer 1
        ("features/typography", TYPOGRAPHY_HTML),
        ("features/box-model", BOX_MODEL_HTML),
        ("features/colors-backgrounds", COLORS_BACKGROUNDS_HTML),
        ("features/flexbox", FLEXBOX_HTML),
        ("features/grid", GRID_HTML),
        ("features/tables", TABLES_HTML),
        ("features/images-svg", IMAGES_SVG_HTML),
        ("features/positioning", POSITIONING_HTML),
        ("features/math", MATH_HTML),
        ("features/pseudo-elements", PSEUDO_ELEMENTS_HTML),
        ("features/transforms", TRANSFORMS_HTML),
        ("features/backgrounds-advanced", BACKGROUNDS_ADV_HTML),
        // Layer 2
        ("combined/simple-report", SIMPLE_REPORT_HTML),
        ("combined/invoice", INVOICE_HTML),
        ("combined/resume", RESUME_HTML),
        ("combined/article", ARTICLE_HTML),
        ("combined/math-paper", MATH_PAPER_HTML),
        ("combined/dashboard", DASHBOARD_HTML),
        // Layer 3
        ("edge-cases/deep-nesting", DEEP_NESTING_HTML),
        ("edge-cases/long-table", LONG_TABLE_HTML),
        ("edge-cases/page-breaks", PAGE_BREAKS_HTML),
        ("edge-cases/overflow", OVERFLOW_HTML),
        ("edge-cases/empty-elements", EMPTY_ELEMENTS_HTML),
        ("edge-cases/unicode", UNICODE_HTML),
    ];

    let mut results: Vec<ParityResult> = Vec::new();
    for (name, html) in &fixtures {
        results.push(run_fixture(name, html));
    }

    // Print markdown table
    println!();
    println!("## Parity Benchmark Report");
    println!();
    println!(
        "| {:<35} | {:>10} | {:>12} | {:>5} |",
        "Fixture", "Size (B)", "Time (us)", "Valid"
    );
    println!("|{:-<37}|{:-<12}|{:-<14}|{:-<7}|", "", "", "", "");

    let mut total_size: usize = 0;
    let mut total_time: u128 = 0;
    let mut all_valid = true;

    for r in &results {
        total_size += r.pdf_size;
        total_time += r.render_time_us;
        if !r.valid {
            all_valid = false;
        }
        println!(
            "| {:<35} | {:>10} | {:>12} | {:>5} |",
            r.fixture,
            format_size(r.pdf_size),
            format_time(r.render_time_us),
            if r.valid { "ok" } else { "FAIL" }
        );
    }

    println!("|{:-<37}|{:-<12}|{:-<14}|{:-<7}|", "", "", "", "");
    println!(
        "| {:<35} | {:>10} | {:>12} | {:>5} |",
        "TOTAL",
        format_size(total_size),
        format_time(total_time),
        if all_valid { "ok" } else { "FAIL" }
    );
    println!();
    println!("Fixtures: {}", results.len());
    println!(
        "All valid: {}",
        if all_valid {
            "yes"
        } else {
            "NO - see failures above"
        }
    );
    println!();

    // Assert everything passed
    for r in &results {
        assert!(r.valid, "Fixture '{}' produced an invalid PDF", r.fixture);
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

fn format_time(us: u128) -> String {
    if us >= 1_000_000 {
        format!("{:.1} s", us as f64 / 1_000_000.0)
    } else if us >= 1_000 {
        format!("{:.1} ms", us as f64 / 1_000.0)
    } else {
        format!("{} us", us)
    }
}
