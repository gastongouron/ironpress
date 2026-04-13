/// Parity test framework: renders HTML fixtures to PDF and validates output.
///
/// Each fixture is a complete HTML document loaded via `include_str!`.
/// Tests verify that ironpress produces valid PDFs with reasonable file sizes.
/// Expectation JSON files in `tests/fixtures/expectations/` define semantic
/// assertions (required PDF operators, required text, size bounds).
/// A baseline JSON at `tests/fixtures/baseline.json` tracks historical metrics.
/// The ignored `parity_benchmark_report` test runs all fixtures and prints a
/// markdown summary table with file sizes, render times, and baseline diffs.
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

/// Load and verify the expectation JSON for a fixture.
/// The expectation key is the fixture name as it appears in baseline.json
/// (e.g. "features/typography").  The JSON file name uses underscores to
/// replace the slash separator (e.g. "features_typography.json").
fn verify_expectations(fixture_key: &str, pdf: &[u8]) {
    // Derive filename: "features/typography" -> "features_typography.json"
    let filename = fixture_key.replace('/', "_");
    let path = format!(
        "{}/tests/fixtures/expectations/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        filename
    );

    let json_str = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            // Missing expectation file is a soft warning, not a hard failure,
            // to avoid blocking tests when expectations are being incrementally added.
            eprintln!(
                "WARN: expectation file not found for '{}': {} ({})",
                fixture_key, e, path
            );
            return;
        }
    };

    let expectations: serde_json::Value = serde_json::from_str(&json_str).expect(&format!(
        "Failed to parse expectations JSON for '{}'",
        fixture_key
    ));

    let pdf_text = String::from_utf8_lossy(pdf);

    // --- must_contain_operators ---
    if let Some(ops) = expectations["pdf_assertions"]["must_contain_operators"].as_array() {
        for op in ops {
            let op_str = op.as_str().unwrap_or("");
            if !op_str.is_empty() {
                assert!(
                    pdf_text.contains(op_str),
                    "Fixture '{}': PDF missing required operator '{}'",
                    fixture_key,
                    op_str
                );
            }
        }
    }

    // --- must_contain_text ---
    if let Some(texts) = expectations["pdf_assertions"]["must_contain_text"].as_array() {
        for text in texts {
            let t = text.as_str().unwrap_or("");
            if !t.is_empty() {
                assert!(
                    pdf_text.contains(t),
                    "Fixture '{}': PDF missing required text '{}'",
                    fixture_key,
                    t
                );
            }
        }
    }

    // --- min_size_bytes ---
    if let Some(min) = expectations["pdf_assertions"]["min_size_bytes"].as_u64() {
        let min = min as usize;
        assert!(
            pdf.len() >= min,
            "Fixture '{}': PDF size {} bytes is below minimum {} bytes",
            fixture_key,
            pdf.len(),
            min
        );
    }

    // --- max_size_bytes ---
    if let Some(max) = expectations["pdf_assertions"]["max_size_bytes"].as_u64() {
        let max = max as usize;
        assert!(
            pdf.len() <= max,
            "Fixture '{}': PDF size {} bytes exceeds maximum {} bytes",
            fixture_key,
            pdf.len(),
            max
        );
    }

    // --- min_pages: count "%%" sequences as a proxy for page count ---
    if let Some(min_pages) = expectations["pdf_assertions"]["min_pages"].as_u64() {
        let min_pages = min_pages as usize;
        // Each page object in a PDF contains "Page" in the type dictionary.
        // Counting occurrences of "/Type /Page" (without the "s") is a reliable
        // heuristic for page count in ironpress-generated PDFs.
        let page_count = count_pdf_pages(pdf);
        assert!(
            page_count >= min_pages,
            "Fixture '{}': PDF has {} page(s), expected at least {}",
            fixture_key,
            page_count,
            min_pages
        );
    }
}

/// Count PDF pages by scanning for "/Type /Page" dictionary entries.
/// This avoids a full PDF parse while remaining accurate for ironpress output.
fn count_pdf_pages(pdf: &[u8]) -> usize {
    let text = String::from_utf8_lossy(pdf);
    // ironpress writes "/Type /Page" (without trailing 's') for individual pages.
    // We count non-overlapping occurrences.
    let needle = "/Type /Page";
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = text[start..].find(needle) {
        // Make sure it is not "/Type /Pages" (the Pages tree node).
        let abs = start + pos;
        let after = abs + needle.len();
        let next_char = text[after..].chars().next();
        if next_char != Some('s') {
            count += 1;
        }
        start = abs + 1;
    }
    // Fallback: at least 1 if the PDF is non-empty.
    count.max(if pdf.len() > 64 { 1 } else { 0 })
}

fn assert_fixture(fixture_key: &str, html: &str) {
    // The short name used for error messages is the last path component.
    let short_name = fixture_key.rsplit('/').next().unwrap_or(fixture_key);

    let start = Instant::now();
    let pdf = ironpress::html_to_pdf(html).expect(&format!("Failed to render {}", short_name));
    let _elapsed = start.elapsed().as_micros();

    assert!(
        pdf_is_valid(&pdf),
        "Fixture '{}' produced an invalid PDF ({} bytes)",
        short_name,
        pdf.len()
    );
    assert!(
        pdf.len() > 100,
        "Fixture '{}' produced a suspiciously small PDF ({} bytes)",
        short_name,
        pdf.len()
    );

    // Semantic assertions from expectation JSON.
    verify_expectations(fixture_key, &pdf);
}

// ---------------------------------------------------------------------------
// Layer 1 tests: individual features
// ---------------------------------------------------------------------------

#[test]
fn parity_typography() {
    assert_fixture("features/typography", TYPOGRAPHY_HTML);
}

#[test]
fn parity_box_model() {
    assert_fixture("features/box-model", BOX_MODEL_HTML);
}

#[test]
fn parity_colors_backgrounds() {
    assert_fixture("features/colors-backgrounds", COLORS_BACKGROUNDS_HTML);
}

#[test]
fn parity_flexbox() {
    assert_fixture("features/flexbox", FLEXBOX_HTML);
}

#[test]
fn parity_grid() {
    assert_fixture("features/grid", GRID_HTML);
}

#[test]
fn parity_tables() {
    assert_fixture("features/tables", TABLES_HTML);
}

#[test]
fn parity_images_svg() {
    assert_fixture("features/images-svg", IMAGES_SVG_HTML);
}

#[test]
fn parity_positioning() {
    assert_fixture("features/positioning", POSITIONING_HTML);
}

#[test]
fn parity_math() {
    assert_fixture("features/math", MATH_HTML);
}

#[test]
fn parity_pseudo_elements() {
    assert_fixture("features/pseudo-elements", PSEUDO_ELEMENTS_HTML);
}

#[test]
fn parity_transforms() {
    assert_fixture("features/transforms", TRANSFORMS_HTML);
}

#[test]
fn parity_backgrounds_advanced() {
    assert_fixture("features/backgrounds-advanced", BACKGROUNDS_ADV_HTML);
}

// ---------------------------------------------------------------------------
// Layer 2 tests: combined cases
// ---------------------------------------------------------------------------

#[test]
fn parity_simple_report() {
    assert_fixture("combined/simple-report", SIMPLE_REPORT_HTML);
}

#[test]
fn parity_invoice() {
    assert_fixture("combined/invoice", INVOICE_HTML);
}

#[test]
fn parity_resume() {
    assert_fixture("combined/resume", RESUME_HTML);
}

#[test]
fn parity_article() {
    assert_fixture("combined/article", ARTICLE_HTML);
}

#[test]
fn parity_math_paper() {
    assert_fixture("combined/math-paper", MATH_PAPER_HTML);
}

#[test]
fn parity_dashboard() {
    assert_fixture("combined/dashboard", DASHBOARD_HTML);
}

// ---------------------------------------------------------------------------
// Layer 3 tests: edge cases
// ---------------------------------------------------------------------------

#[test]
fn parity_deep_nesting() {
    assert_fixture("edge-cases/deep-nesting", DEEP_NESTING_HTML);
}

#[test]
fn parity_long_table() {
    assert_fixture("edge-cases/long-table", LONG_TABLE_HTML);
}

#[test]
fn parity_page_breaks() {
    assert_fixture("edge-cases/page-breaks", PAGE_BREAKS_HTML);
}

#[test]
fn parity_overflow() {
    assert_fixture("edge-cases/overflow", OVERFLOW_HTML);
}

#[test]
fn parity_empty_elements() {
    assert_fixture("edge-cases/empty-elements", EMPTY_ELEMENTS_HTML);
}

#[test]
fn parity_unicode() {
    assert_fixture("edge-cases/unicode", UNICODE_HTML);
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

    // Load baseline for diff computation.
    let baseline = load_baseline();

    let mut results: Vec<ParityResult> = Vec::new();
    for (name, html) in &fixtures {
        results.push(run_fixture(name, html));
    }

    // Print markdown table with baseline diffs.
    println!();
    println!("## Parity Benchmark Report");
    println!();
    println!(
        "| {:<35} | {:>10} | {:>12} | {:>10} | {:>12} | {:>5} |",
        "Fixture", "Size (B)", "Δ Size", "Time (us)", "Δ Time", "Valid"
    );
    println!(
        "|{:-<37}|{:-<12}|{:-<12}|{:-<12}|{:-<14}|{:-<7}|",
        "", "", "", "", "", ""
    );

    let mut total_size: usize = 0;
    let mut total_time: u128 = 0;
    let mut all_valid = true;

    for r in &results {
        total_size += r.pdf_size;
        total_time += r.render_time_us;
        if !r.valid {
            all_valid = false;
        }

        let (size_diff, time_diff) =
            compute_baseline_diff(&baseline, &r.fixture, r.pdf_size, r.render_time_us);

        println!(
            "| {:<35} | {:>10} | {:>10} | {:>12} | {:>12} | {:>5} |",
            r.fixture,
            format_size(r.pdf_size),
            size_diff,
            format_time(r.render_time_us),
            time_diff,
            if r.valid { "ok" } else { "FAIL" }
        );
    }

    println!(
        "|{:-<37}|{:-<12}|{:-<12}|{:-<12}|{:-<14}|{:-<7}|",
        "", "", "", "", "", ""
    );
    println!(
        "| {:<35} | {:>10} | {:>10} | {:>12} | {:>12} | {:>5} |",
        "TOTAL",
        format_size(total_size),
        "",
        format_time(total_time),
        "",
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

    // Print baseline info.
    println!("Baseline: {}", baseline_summary(&baseline));
    println!(
        "(Run with --ignored to update the report; baseline values of 0 indicate not yet populated.)"
    );
    println!();

    // Assert everything passed.
    for r in &results {
        assert!(r.valid, "Fixture '{}' produced an invalid PDF", r.fixture);
    }
}

// ---------------------------------------------------------------------------
// Baseline helpers
// ---------------------------------------------------------------------------

fn load_baseline() -> serde_json::Value {
    let path = format!(
        "{}/tests/fixtures/baseline.json",
        env!("CARGO_MANIFEST_DIR")
    );
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    }
}

fn baseline_summary(baseline: &serde_json::Value) -> String {
    if baseline.is_null() {
        return "(no baseline file found)".to_string();
    }
    let version = baseline["version"].as_str().unwrap_or("unknown");
    let generated_at = baseline["generated_at"].as_str().unwrap_or("unknown");
    format!("v{} generated {}", version, generated_at)
}

/// Returns (size_diff_str, time_diff_str) comparing current run to baseline.
/// If baseline value is 0 (uninitialized), returns "n/a".
fn compute_baseline_diff(
    baseline: &serde_json::Value,
    fixture: &str,
    current_size: usize,
    current_time_us: u128,
) -> (String, String) {
    if baseline.is_null() {
        return ("n/a".to_string(), "n/a".to_string());
    }

    let fixtures = &baseline["fixtures"];
    let entry = &fixtures[fixture];

    let size_diff = if let Some(base_size) = entry["size_bytes"].as_u64() {
        if base_size == 0 {
            "n/a".to_string()
        } else {
            let pct = (current_size as f64 - base_size as f64) / base_size as f64 * 100.0;
            format_pct(pct)
        }
    } else {
        "n/a".to_string()
    };

    let time_diff = if let Some(base_time) = entry["render_time_us"].as_u64() {
        if base_time == 0 {
            "n/a".to_string()
        } else {
            let pct = (current_time_us as f64 - base_time as f64) / base_time as f64 * 100.0;
            format_pct(pct)
        }
    } else {
        "n/a".to_string()
    };

    (size_diff, time_diff)
}

fn format_pct(pct: f64) -> String {
    if pct >= 0.0 {
        format!("+{:.1}%", pct)
    } else {
        format!("{:.1}%", pct)
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

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
