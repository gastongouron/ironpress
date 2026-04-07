//! Math layout engine following TeX typesetting conventions.
//!
//! Converts a [`MathNode`] AST into positioned [`MathGlyph`]s ready for
//! PDF rendering. Implements TeX's four style levels, Knuth's atom spacing
//! matrix, and proper fraction/script/radical layout.

use crate::parser::math::{AtomType, MathNode};

/// A positioned glyph or drawing command in the laid-out math expression.
#[derive(Debug, Clone)]
pub enum MathGlyph {
    /// A character to render, with font info.
    Char {
        ch: char,
        x: f32,
        y: f32,
        font_size: f32,
        italic: bool,
    },
    /// A text string rendered upright (for \text, operators).
    Text {
        text: String,
        x: f32,
        y: f32,
        font_size: f32,
    },
    /// A horizontal line (fraction bar, overline accent).
    Rule {
        x: f32,
        y: f32,
        width: f32,
        thickness: f32,
    },
    /// A radical (square root) sign path.
    Radical {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: f32,
    },
    /// A delimiter (parenthesis, bracket) scaled to height.
    Delimiter {
        ch: char,
        x: f32,
        y: f32,
        height: f32,
        font_size: f32,
    },
}

/// Result of laying out a math expression.
#[derive(Debug, Clone)]
pub struct MathLayout {
    /// All positioned glyphs.
    pub glyphs: Vec<MathGlyph>,
    /// Total width in points.
    pub width: f32,
    /// Height above baseline in points.
    pub ascent: f32,
    /// Depth below baseline in points (positive value).
    pub descent: f32,
}

impl MathLayout {
    pub fn height(&self) -> f32 {
        self.ascent + self.descent
    }

    /// Translate all glyphs by (dx, dy).
    fn translate(&mut self, dx: f32, dy: f32) {
        for g in &mut self.glyphs {
            match g {
                MathGlyph::Char { x, y, .. }
                | MathGlyph::Text { x, y, .. }
                | MathGlyph::Rule { x, y, .. }
                | MathGlyph::Radical { x, y, .. }
                | MathGlyph::Delimiter { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
            }
        }
    }
}

/// TeX math style levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathStyle {
    Display,      // \displaystyle (display equations)
    Text,         // \textstyle (inline math)
    Script,       // \scriptstyle (superscripts/subscripts)
    ScriptScript, // \scriptscriptstyle (scripts of scripts)
}

impl MathStyle {
    /// Font size multiplier relative to the base font size.
    pub fn size_factor(self) -> f32 {
        match self {
            MathStyle::Display | MathStyle::Text => 1.0,
            MathStyle::Script => 0.7,
            MathStyle::ScriptScript => 0.5,
        }
    }

    /// The style used for superscripts/subscripts of this style.
    pub fn script_style(self) -> MathStyle {
        match self {
            MathStyle::Display | MathStyle::Text => MathStyle::Script,
            MathStyle::Script | MathStyle::ScriptScript => MathStyle::ScriptScript,
        }
    }

    /// The style used for numerators/denominators.
    pub fn frac_style(self) -> MathStyle {
        match self {
            MathStyle::Display => MathStyle::Text,
            MathStyle::Text => MathStyle::Script,
            MathStyle::Script | MathStyle::ScriptScript => MathStyle::ScriptScript,
        }
    }
}

/// Lay out a MathNode at the given base font size and style.
pub fn layout_math(node: &MathNode, base_font_size: f32, display: bool) -> MathLayout {
    let style = if display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    layout_node(node, base_font_size, style)
}

fn layout_node(node: &MathNode, base_size: f32, style: MathStyle) -> MathLayout {
    let font_size = base_size * style.size_factor();
    match node {
        MathNode::Symbol(ch) => layout_symbol(*ch, font_size, true),
        MathNode::Number(s) => layout_text_str(s, font_size, false),
        MathNode::Greek(ch) => layout_symbol(*ch, font_size, true),
        MathNode::Operator(name) => layout_text_str(name, font_size, false),
        MathNode::Text(text) => layout_text_str(text, font_size, false),
        MathNode::Space(em) => {
            let w = em * font_size;
            MathLayout {
                glyphs: vec![],
                width: w,
                ascent: 0.0,
                descent: 0.0,
            }
        }
        MathNode::LargeOp { symbol, .. } => layout_large_op(*symbol, font_size, style),
        MathNode::Superscript { base, sup } => layout_superscript(base, sup, base_size, style),
        MathNode::Subscript { base, sub } => layout_subscript(base, sub, base_size, style),
        MathNode::SubSup { base, sub, sup } => layout_subsup(base, sub, sup, base_size, style),
        MathNode::Fraction {
            numerator,
            denominator,
        } => layout_fraction(numerator, denominator, base_size, style),
        MathNode::Root { index, radicand } => {
            layout_radical(index.as_deref(), radicand, base_size, style)
        }
        MathNode::Row(nodes) => layout_row(nodes, base_size, style),
        MathNode::Delimited { open, close, body } => {
            layout_delimited(*open, *close, body, base_size, style)
        }
        MathNode::Accent { accent, body } => layout_accent(*accent, body, base_size, style),
        MathNode::Matrix {
            rows, delimiters, ..
        } => layout_matrix(rows, *delimiters, base_size, style),
    }
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

/// Approximate character width in points (using Helvetica-like proportions).
fn char_width_approx(ch: char, font_size: f32) -> f32 {
    // Rough proportional widths as fraction of font_size
    let factor = if ch.is_ascii_uppercase() {
        0.72
    } else if ch.is_ascii_lowercase() {
        0.50
    } else if ch.is_ascii_digit() {
        0.56
    } else {
        match ch {
            '+' | '=' | '<' | '>' | '\u{2264}' | '\u{2265}' | '\u{2260}' | '\u{2248}' => 0.58,
            '-' | '\u{2212}' => 0.33,
            '(' | ')' | '[' | ']' => 0.33,
            '{' | '}' => 0.33,
            '|' => 0.26,
            ',' | ';' => 0.28,
            '.' => 0.28,
            ' ' => 0.28,
            '\u{2211}' | '\u{220F}' | '\u{2210}' => 0.83, // sum, prod
            '\u{222B}' | '\u{222C}' | '\u{222D}' | '\u{222E}' => 0.44, // integrals
            '\u{221E}' => 0.72,                           // infinity
            _ if ('\u{0391}'..='\u{03C9}').contains(&ch) => 0.58, // Greek
            _ => 0.56,
        }
    };
    factor * font_size
}

fn layout_symbol(ch: char, font_size: f32, italic: bool) -> MathLayout {
    let w = char_width_approx(ch, font_size);
    let ascent = font_size * 0.72;
    let descent = font_size * 0.22;
    MathLayout {
        glyphs: vec![MathGlyph::Char {
            ch,
            x: 0.0,
            y: 0.0,
            font_size,
            italic,
        }],
        width: w,
        ascent,
        descent,
    }
}

fn layout_text_str(s: &str, font_size: f32, italic: bool) -> MathLayout {
    if s.is_empty() {
        return MathLayout {
            glyphs: vec![],
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
        };
    }

    // For text, use a single Text glyph
    let width: f32 = s.chars().map(|c| char_width_approx(c, font_size)).sum();
    let ascent = font_size * 0.72;
    let descent = font_size * 0.22;

    if italic {
        // Render char by char for italic
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        for ch in s.chars() {
            glyphs.push(MathGlyph::Char {
                ch,
                x,
                y: 0.0,
                font_size,
                italic: true,
            });
            x += char_width_approx(ch, font_size);
        }
        MathLayout {
            glyphs,
            width,
            ascent,
            descent,
        }
    } else {
        MathLayout {
            glyphs: vec![MathGlyph::Text {
                text: s.to_string(),
                x: 0.0,
                y: 0.0,
                font_size,
            }],
            width,
            ascent,
            descent,
        }
    }
}

fn layout_large_op(symbol: char, font_size: f32, style: MathStyle) -> MathLayout {
    let scale = match style {
        MathStyle::Display => 1.5,
        _ => 1.0,
    };
    let op_size = font_size * scale;
    let w = char_width_approx(symbol, op_size);
    let ascent = op_size * 0.75;
    let descent = op_size * 0.25;

    MathLayout {
        glyphs: vec![MathGlyph::Char {
            ch: symbol,
            x: 0.0,
            y: 0.0,
            font_size: op_size,
            italic: false,
        }],
        width: w,
        ascent,
        descent,
    }
}

fn layout_row(nodes: &[MathNode], base_size: f32, style: MathStyle) -> MathLayout {
    if nodes.is_empty() {
        return MathLayout {
            glyphs: vec![],
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
        };
    }

    let mut layouts: Vec<MathLayout> = nodes
        .iter()
        .map(|n| layout_node(n, base_size, style))
        .collect();
    let font_size = base_size * style.size_factor();

    // Apply inter-atom spacing (Knuth's spacing table)
    let mut total_width = 0.0f32;
    let mut max_ascent = 0.0f32;
    let mut max_descent = 0.0f32;
    let mut combined_glyphs = Vec::new();

    for (i, layout) in layouts.iter_mut().enumerate() {
        // Add inter-atom spacing
        if i > 0 {
            let prev_type = nodes[i - 1].atom_type();
            let curr_type = nodes[i].atom_type();
            let space = atom_spacing(prev_type, curr_type, font_size);
            total_width += space;
        }

        layout.translate(total_width, 0.0);
        total_width += layout.width;
        max_ascent = max_ascent.max(layout.ascent);
        max_descent = max_descent.max(layout.descent);
        combined_glyphs.append(&mut layout.glyphs);
    }

    MathLayout {
        glyphs: combined_glyphs,
        width: total_width,
        ascent: max_ascent,
        descent: max_descent,
    }
}

/// Knuth's inter-atom spacing table (simplified).
/// Returns spacing in points.
fn atom_spacing(left: AtomType, right: AtomType, font_size: f32) -> f32 {
    use AtomType::*;
    let mu = font_size / 18.0; // 1mu = 1/18 em
    let thin = 3.0 * mu;
    let med = 4.0 * mu;
    let thick = 5.0 * mu;

    match (left, right) {
        // Ord × Op → thin
        (Ord, Op) => thin,
        // Op × Ord → thin
        (Op, Ord) => thin,
        // Op × Op → thin
        (Op, Op) => thin,
        // Ord × Bin → med
        (Ord, Bin) | (Inner, Bin) | (Close, Bin) => med,
        // Bin × Ord → med
        (Bin, Ord) | (Bin, Op) | (Bin, Open) | (Bin, Inner) => med,
        // Ord × Rel → thick
        (Ord, Rel) | (Close, Rel) | (Inner, Rel) => thick,
        // Rel × Ord → thick
        (Rel, Ord) | (Rel, Op) | (Rel, Open) | (Rel, Inner) => thick,
        // Close × Op → thin
        (Close, Op) => thin,
        // Punct → thin after
        (Punct, Ord) | (Punct, Op) | (Punct, Open) | (Punct, Inner) => thin,
        // Inner × Op → thin
        (Inner, Op) => thin,
        // Op × Inner → thin
        (Op, Inner) => thin,
        // Everything else → 0
        _ => 0.0,
    }
}

fn layout_superscript(
    base: &MathNode,
    sup: &MathNode,
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    let mut base_layout = layout_node(base, base_size, style);
    let mut sup_layout = layout_node(sup, base_size, style.script_style());

    let font_size = base_size * style.size_factor();
    // TeX: superscript shift up is ~0.45em of the base style
    let shift_up = font_size * 0.45;

    sup_layout.translate(base_layout.width, shift_up);

    let ascent = base_layout.ascent.max(sup_layout.ascent + shift_up);
    let descent = base_layout
        .descent
        .max(sup_layout.descent - shift_up)
        .max(0.0);
    let width = base_layout.width + sup_layout.width;

    let mut glyphs = Vec::new();
    glyphs.append(&mut base_layout.glyphs);
    glyphs.append(&mut sup_layout.glyphs);

    MathLayout {
        glyphs,
        width,
        ascent,
        descent,
    }
}

fn layout_subscript(
    base: &MathNode,
    sub: &MathNode,
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    let mut base_layout = layout_node(base, base_size, style);
    let mut sub_layout = layout_node(sub, base_size, style.script_style());

    let font_size = base_size * style.size_factor();
    let shift_down = font_size * 0.25;

    sub_layout.translate(base_layout.width, -shift_down);

    let ascent = base_layout
        .ascent
        .max(sub_layout.ascent - shift_down)
        .max(0.0);
    let descent = base_layout.descent.max(sub_layout.descent + shift_down);
    let width = base_layout.width + sub_layout.width;

    let mut glyphs = Vec::new();
    glyphs.append(&mut base_layout.glyphs);
    glyphs.append(&mut sub_layout.glyphs);

    MathLayout {
        glyphs,
        width,
        ascent,
        descent,
    }
}

fn layout_subsup(
    base: &MathNode,
    sub: &MathNode,
    sup: &MathNode,
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    let mut base_layout = layout_node(base, base_size, style);
    let script_style = style.script_style();
    let mut sup_layout = layout_node(sup, base_size, script_style);
    let mut sub_layout = layout_node(sub, base_size, script_style);

    let font_size = base_size * style.size_factor();
    let shift_up = font_size * 0.45;
    let shift_down = font_size * 0.25;

    let script_x = base_layout.width;
    sup_layout.translate(script_x, shift_up);
    sub_layout.translate(script_x, -shift_down);

    let script_width = sup_layout.width.max(sub_layout.width);
    let ascent = base_layout.ascent.max(sup_layout.ascent + shift_up);
    let descent = base_layout.descent.max(sub_layout.descent + shift_down);
    let width = base_layout.width + script_width;

    let mut glyphs = Vec::new();
    glyphs.append(&mut base_layout.glyphs);
    glyphs.append(&mut sup_layout.glyphs);
    glyphs.append(&mut sub_layout.glyphs);

    MathLayout {
        glyphs,
        width,
        ascent,
        descent,
    }
}

fn layout_fraction(num: &MathNode, den: &MathNode, base_size: f32, style: MathStyle) -> MathLayout {
    let frac_style = style.frac_style();
    let mut num_layout = layout_node(num, base_size, frac_style);
    let mut den_layout = layout_node(den, base_size, frac_style);

    let font_size = base_size * style.size_factor();
    let rule_thickness = font_size * 0.04; // fraction bar thickness
    let gap = font_size * 0.15; // gap between bar and content
    let axis_height = font_size * 0.25; // math axis (center of operators like +, =)

    let frac_width = num_layout.width.max(den_layout.width) + font_size * 0.2;

    // Center numerator above the bar
    let num_x = (frac_width - num_layout.width) / 2.0;
    let num_y = axis_height + gap + rule_thickness / 2.0 + num_layout.descent;
    num_layout.translate(num_x, num_y);

    // Center denominator below the bar
    let den_x = (frac_width - den_layout.width) / 2.0;
    let den_y = axis_height - gap - rule_thickness / 2.0 - den_layout.ascent;
    den_layout.translate(den_x, den_y);

    let ascent = num_y + num_layout.ascent;
    let descent = (-(den_y - den_layout.descent)).max(0.0);

    let mut glyphs = Vec::new();
    glyphs.append(&mut num_layout.glyphs);
    glyphs.append(&mut den_layout.glyphs);
    // Fraction bar
    glyphs.push(MathGlyph::Rule {
        x: 0.0,
        y: axis_height,
        width: frac_width,
        thickness: rule_thickness,
    });

    MathLayout {
        glyphs,
        width: frac_width,
        ascent,
        descent,
    }
}

fn layout_radical(
    index: Option<&MathNode>,
    radicand: &MathNode,
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    let mut rad_layout = layout_node(radicand, base_size, style);
    let font_size = base_size * style.size_factor();

    let pad_top = font_size * 0.1;
    let rule_thickness = font_size * 0.04;
    let surd_width = font_size * 0.6; // width of the radical sign
    let content_width = rad_layout.width + font_size * 0.1; // small padding

    let total_height = rad_layout.ascent + rad_layout.descent + pad_top + rule_thickness;

    // Shift radicand content to the right of the surd
    rad_layout.translate(surd_width, 0.0);

    let mut glyphs = Vec::new();

    // Radical sign
    glyphs.push(MathGlyph::Radical {
        x: 0.0,
        y: 0.0,
        width: surd_width,
        height: total_height,
        font_size,
    });

    // Overline above radicand
    glyphs.push(MathGlyph::Rule {
        x: surd_width,
        y: rad_layout.ascent + pad_top,
        width: content_width,
        thickness: rule_thickness,
    });

    glyphs.append(&mut rad_layout.glyphs);

    let total_width = surd_width + content_width;
    let ascent = rad_layout.ascent + pad_top + rule_thickness;
    let descent = rad_layout.descent;

    let mut result = MathLayout {
        glyphs,
        width: total_width,
        ascent,
        descent,
    };

    // Index (e.g. cube root)
    if let Some(idx) = index {
        let mut idx_layout = layout_node(idx, base_size, MathStyle::ScriptScript);
        let idx_x = surd_width * 0.1;
        let idx_y = total_height * 0.55;
        idx_layout.translate(idx_x, idx_y);
        result.glyphs.append(&mut idx_layout.glyphs);
        // Widen if the index extends left
        // (typically it doesn't extend beyond the surd)
    }

    result
}

fn layout_delimited(
    open: char,
    close: char,
    body: &MathNode,
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    let mut body_layout = layout_node(body, base_size, style);
    let font_size = base_size * style.size_factor();

    let delim_height = body_layout.height().max(font_size);
    let delim_width = font_size * 0.35;

    let mut glyphs = Vec::new();

    // Open delimiter
    if open != '.' {
        glyphs.push(MathGlyph::Delimiter {
            ch: open,
            x: 0.0,
            y: 0.0,
            height: delim_height,
            font_size,
        });
    }

    let body_x = if open != '.' { delim_width } else { 0.0 };
    body_layout.translate(body_x, 0.0);
    glyphs.append(&mut body_layout.glyphs);

    let close_x = body_x + body_layout.width;
    if close != '.' {
        glyphs.push(MathGlyph::Delimiter {
            ch: close,
            x: close_x,
            y: 0.0,
            height: delim_height,
            font_size,
        });
    }

    let total_width = close_x + if close != '.' { delim_width } else { 0.0 };

    MathLayout {
        glyphs,
        width: total_width,
        ascent: body_layout.ascent.max(delim_height / 2.0),
        descent: body_layout.descent.max(delim_height / 2.0),
    }
}

fn layout_accent(accent: char, body: &MathNode, base_size: f32, style: MathStyle) -> MathLayout {
    let mut body_layout = layout_node(body, base_size, style);
    let font_size = base_size * style.size_factor();

    let accent_height = font_size * 0.15;
    let accent_y = body_layout.ascent + accent_height * 0.5;

    // Render accent as a character centered above the body
    let accent_x = body_layout.width / 2.0 - char_width_approx(accent, font_size) / 2.0;
    body_layout.glyphs.push(MathGlyph::Char {
        ch: accent_to_visual(accent),
        x: accent_x.max(0.0),
        y: accent_y,
        font_size: font_size * 0.8,
        italic: false,
    });

    let ascent = accent_y + accent_height;

    MathLayout {
        glyphs: body_layout.glyphs,
        width: body_layout.width,
        ascent,
        descent: body_layout.descent,
    }
}

/// Map combining accent codepoints to visual standalone characters.
fn accent_to_visual(accent: char) -> char {
    match accent {
        '\u{0302}' => '\u{005E}', // circumflex → ^
        '\u{0304}' => '\u{00AF}', // macron → ¯
        '\u{0303}' => '~',        // tilde
        '\u{0307}' => '\u{02D9}', // dot above → ˙
        '\u{0308}' => '\u{00A8}', // diaeresis → ¨
        '\u{20D7}' => '\u{2192}', // combining arrow → →
        _ => accent,
    }
}

fn layout_matrix(
    rows: &[Vec<MathNode>],
    delimiters: (char, char),
    base_size: f32,
    style: MathStyle,
) -> MathLayout {
    if rows.is_empty() {
        return MathLayout {
            glyphs: vec![],
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
        };
    }

    let font_size = base_size * style.size_factor();
    let col_gap = font_size * 0.8;
    let row_gap = font_size * 0.4;

    // Layout all cells
    let mut cell_layouts: Vec<Vec<MathLayout>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| layout_node(cell, base_size, style))
                .collect()
        })
        .collect();

    let num_cols = cell_layouts.iter().map(|r| r.len()).max().unwrap_or(0);

    // Compute column widths
    let mut col_widths = vec![0.0f32; num_cols];
    for row in &cell_layouts {
        for (j, cell) in row.iter().enumerate() {
            col_widths[j] = col_widths[j].max(cell.width);
        }
    }

    // Compute row heights
    let row_metrics: Vec<(f32, f32)> = cell_layouts
        .iter()
        .map(|row| {
            let asc = row.iter().map(|c| c.ascent).fold(0.0f32, f32::max);
            let desc = row.iter().map(|c| c.descent).fold(0.0f32, f32::max);
            (asc, desc)
        })
        .collect();

    // Total content dimensions
    let total_row_height: f32 = row_metrics.iter().map(|(a, d)| a + d).sum::<f32>()
        + row_gap * (row_metrics.len().saturating_sub(1) as f32);
    let total_col_width: f32 =
        col_widths.iter().sum::<f32>() + col_gap * (num_cols.saturating_sub(1) as f32);

    // Position cells
    let mut glyphs = Vec::new();
    let mut y_cursor = total_row_height / 2.0; // start from top, centered on baseline

    for (i, row) in cell_layouts.iter_mut().enumerate() {
        let (row_asc, row_desc) = row_metrics[i];
        y_cursor -= row_asc;

        let mut x_cursor = 0.0;
        for (j, cell) in row.iter_mut().enumerate() {
            // Center cell in column
            let cell_x = x_cursor + (col_widths[j] - cell.width) / 2.0;
            cell.translate(cell_x, y_cursor);
            glyphs.append(&mut cell.glyphs);
            x_cursor += col_widths[j] + col_gap;
        }

        y_cursor -= row_desc + row_gap;
    }

    let matrix_layout = MathLayout {
        glyphs,
        width: total_col_width,
        ascent: total_row_height / 2.0 + font_size * 0.25,
        descent: total_row_height / 2.0 - font_size * 0.25,
    };

    // Wrap in delimiters
    if delimiters.0 != '.' || delimiters.1 != '.' {
        let delim_height = matrix_layout.height();
        let delim_width = font_size * 0.35;
        let mut all_glyphs = Vec::new();

        if delimiters.0 != '.' {
            all_glyphs.push(MathGlyph::Delimiter {
                ch: delimiters.0,
                x: 0.0,
                y: 0.0,
                height: delim_height,
                font_size,
            });
        }

        let body_x = if delimiters.0 != '.' {
            delim_width
        } else {
            0.0
        };
        let mut body = matrix_layout;
        body.translate(body_x, 0.0);
        all_glyphs.append(&mut body.glyphs);

        let close_x = body_x + body.width;
        if delimiters.1 != '.' {
            all_glyphs.push(MathGlyph::Delimiter {
                ch: delimiters.1,
                x: close_x,
                y: 0.0,
                height: delim_height,
                font_size,
            });
        }

        let total_width = close_x
            + if delimiters.1 != '.' {
                delim_width
            } else {
                0.0
            };

        MathLayout {
            glyphs: all_glyphs,
            width: total_width,
            ascent: body.ascent,
            descent: body.descent,
        }
    } else {
        matrix_layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::math::parse_math;

    #[test]
    fn simple_symbol_layout() {
        let layout = layout_math(&parse_math("x"), 12.0, false);
        assert!(layout.width > 0.0);
        assert!(layout.ascent > 0.0);
        assert_eq!(layout.glyphs.len(), 1);
    }

    #[test]
    fn fraction_layout() {
        let layout = layout_math(&parse_math("\\frac{a}{b}"), 12.0, true);
        assert!(layout.width > 0.0);
        assert!(layout.ascent > layout.descent);
        // Should have: num glyph + den glyph + fraction bar
        assert!(layout.glyphs.len() >= 3);
    }

    #[test]
    fn superscript_smaller() {
        let base = layout_math(&parse_math("x"), 12.0, false);
        let sup = layout_math(&parse_math("x^2"), 12.0, false);
        // Superscript expression should be wider than just base
        assert!(sup.width > base.width);
    }

    #[test]
    fn display_larger_than_inline() {
        let display = layout_math(&parse_math("\\sum"), 12.0, true);
        let inline = layout_math(&parse_math("\\sum"), 12.0, false);
        // Display sum should be taller
        assert!(display.height() > inline.height());
    }

    #[test]
    fn row_spacing() {
        let layout = layout_math(&parse_math("a + b = c"), 12.0, false);
        // Should have spacing around + and =
        let no_space = layout_math(&parse_math("abc"), 12.0, false);
        assert!(layout.width > no_space.width);
    }

    #[test]
    fn radical_has_sign() {
        let layout = layout_math(&parse_math("\\sqrt{x}"), 12.0, false);
        let has_radical = layout
            .glyphs
            .iter()
            .any(|g| matches!(g, MathGlyph::Radical { .. }));
        assert!(has_radical);
    }

    #[test]
    fn matrix_layout_basic() {
        let layout = layout_math(
            &parse_math("\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}"),
            12.0,
            true,
        );
        assert!(layout.width > 0.0);
        assert!(layout.glyphs.len() >= 4); // at least 4 cells
    }

    #[test]
    fn empty_expression() {
        let layout = layout_math(&parse_math(""), 12.0, false);
        assert_eq!(layout.width, 0.0);
    }

    #[test]
    fn style_factors() {
        assert_eq!(MathStyle::Display.size_factor(), 1.0);
        assert_eq!(MathStyle::Script.size_factor(), 0.7);
        assert_eq!(MathStyle::ScriptScript.size_factor(), 0.5);
    }

    #[test]
    fn script_style_progression() {
        assert_eq!(MathStyle::Display.script_style(), MathStyle::Script);
        assert_eq!(MathStyle::Script.script_style(), MathStyle::ScriptScript);
        assert_eq!(
            MathStyle::ScriptScript.script_style(),
            MathStyle::ScriptScript
        );
    }

    #[test]
    fn complex_expression_no_panic() {
        let exprs = [
            "\\frac{\\sum_{i=1}^{n} x_i}{n}",
            "\\sqrt{\\frac{a^2 + b^2}{c}}",
            "\\int_0^\\infty e^{-x^2} dx",
            "\\left(\\frac{a}{b}\\right)^2",
            "E = mc^2",
            "\\hat{x} + \\bar{y}",
        ];
        for expr in &exprs {
            let layout = layout_math(&parse_math(expr), 12.0, true);
            assert!(layout.width > 0.0, "Failed for: {expr}");
        }
    }
}
