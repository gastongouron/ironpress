//! LaTeX math expression parser.
//!
//! Parses a subset of LaTeX math into a [`MathNode`] AST suitable for
//! typographic layout following TeX conventions.

/// A node in the math expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum MathNode {
    /// A single character (letter, digit, operator symbol).
    Symbol(char),
    /// A number literal (sequence of digits and dots).
    Number(String),
    /// A named operator rendered upright: sin, cos, log, lim, etc.
    Operator(String),
    /// A large operator with optional limits: \sum, \prod, \int.
    LargeOp { symbol: char, limits: bool },
    /// A Greek letter: \alpha → α, etc.
    Greek(char),
    /// Superscript: base^{exponent}.
    Superscript {
        base: Box<MathNode>,
        sup: Box<MathNode>,
    },
    /// Subscript: base_{subscript}.
    Subscript {
        base: Box<MathNode>,
        sub: Box<MathNode>,
    },
    /// Both super- and subscript on the same base.
    SubSup {
        base: Box<MathNode>,
        sub: Box<MathNode>,
        sup: Box<MathNode>,
    },
    /// Fraction: \frac{num}{den}.
    Fraction {
        numerator: Box<MathNode>,
        denominator: Box<MathNode>,
    },
    /// Square root or nth root: \sqrt{x}, \sqrt[3]{x}.
    Root {
        index: Option<Box<MathNode>>,
        radicand: Box<MathNode>,
    },
    /// A group of nodes rendered sequentially.
    Row(Vec<MathNode>),
    /// Parenthesized / delimited group: \left( ... \right).
    Delimited {
        open: char,
        close: char,
        body: Box<MathNode>,
    },
    /// An accent above a base: \hat{x}, \bar{x}, etc.
    Accent { accent: char, body: Box<MathNode> },
    /// Text in math mode: \text{...}, \mathrm{...}.
    Text(String),
    /// Explicit space: \, \; \quad \qquad.
    Space(f32),
    /// A matrix/array environment.
    Matrix {
        rows: Vec<Vec<MathNode>>,
        delimiters: (char, char),
    },
}

/// TeX atom type for inter-atom spacing (Knuth's classification).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomType {
    Ord,   // ordinary symbol
    Op,    // large operator
    Bin,   // binary operator
    Rel,   // relation
    Open,  // opening delimiter
    Close, // closing delimiter
    Punct, // punctuation
    Inner, // fraction, \left..\right
}

impl MathNode {
    /// Return the TeX atom type for spacing purposes.
    pub fn atom_type(&self) -> AtomType {
        match self {
            MathNode::Symbol(c) => match c {
                '+' | '\u{2212}' => AtomType::Bin, // + and −
                '-' => AtomType::Bin,
                '*' | '\u{00D7}' | '\u{22C5}' => AtomType::Bin,
                '=' | '<' | '>' | '\u{2264}' | '\u{2265}' | '\u{2260}' | '\u{2248}'
                | '\u{2261}' | '\u{221D}' | '\u{2282}' | '\u{2283}' | '\u{2286}' | '\u{2287}'
                | '\u{2208}' | '\u{2209}' | '\u{22A2}' | '\u{22A8}' | '\u{2192}' | '\u{2190}'
                | '\u{2194}' | '\u{21D2}' | '\u{21D0}' | '\u{21D4}' => AtomType::Rel,
                '(' | '[' | '{' => AtomType::Open,
                ')' | ']' | '}' => AtomType::Close,
                ',' | ';' => AtomType::Punct,
                _ => AtomType::Ord,
            },
            MathNode::LargeOp { .. } | MathNode::Operator(_) => AtomType::Op,
            MathNode::Fraction { .. } | MathNode::Delimited { .. } => AtomType::Inner,
            MathNode::Root { .. } => AtomType::Ord,
            MathNode::Space(_) => AtomType::Ord,
            MathNode::Text(_) => AtomType::Ord,
            MathNode::Matrix { .. } => AtomType::Inner,
            _ => AtomType::Ord,
        }
    }
}

/// Maximum nesting depth for math expressions to prevent stack overflow.
const MAX_MATH_DEPTH: usize = 50;

/// Parse a LaTeX math string into a MathNode AST.
pub fn parse_math(input: &str) -> MathNode {
    let tokens = tokenize(input);
    let mut pos = 0;
    let mut depth = 0;
    let nodes = parse_expression(&tokens, &mut pos, &mut depth);
    if nodes.len() == 1 {
        nodes.into_iter().next().unwrap()
    } else {
        MathNode::Row(nodes)
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Char(char),
    Command(String), // e.g. "frac", "alpha"
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Caret,
    Underscore,
    Ampersand,
    Backslash, // \\ (line break in matrix)
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            '\\' => {
                chars.next();
                if let Some(&next) = chars.peek() {
                    if next == '\\' {
                        chars.next();
                        tokens.push(Token::Backslash);
                    } else if next == '{'
                        || next == '}'
                        || next == '&'
                        || next == '%'
                        || next == '#'
                        || next == '_'
                    {
                        chars.next();
                        tokens.push(Token::Char(next));
                    } else if next == ',' {
                        chars.next();
                        tokens.push(Token::Command("thinspace".into()));
                    } else if next == ';' {
                        chars.next();
                        tokens.push(Token::Command("thickspace".into()));
                    } else if next == '!' {
                        chars.next();
                        tokens.push(Token::Command("negspace".into()));
                    } else if next == ' ' {
                        chars.next();
                        tokens.push(Token::Command("space".into()));
                    } else if next.is_ascii_alphabetic() {
                        let mut cmd = String::new();
                        while let Some(&c) = chars.peek() {
                            if c.is_ascii_alphabetic() {
                                cmd.push(c);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        // Consume optional trailing space after command
                        if let Some(&' ') = chars.peek() {
                            chars.next();
                        }
                        tokens.push(Token::Command(cmd));
                    } else {
                        // Unknown escape, emit as char
                        chars.next();
                        tokens.push(Token::Char(next));
                    }
                }
            }
            '{' => {
                chars.next();
                tokens.push(Token::LBrace);
            }
            '}' => {
                chars.next();
                tokens.push(Token::RBrace);
            }
            '[' => {
                chars.next();
                tokens.push(Token::LBracket);
            }
            ']' => {
                chars.next();
                tokens.push(Token::RBracket);
            }
            '^' => {
                chars.next();
                tokens.push(Token::Caret);
            }
            '_' => {
                chars.next();
                tokens.push(Token::Underscore);
            }
            '&' => {
                chars.next();
                tokens.push(Token::Ampersand);
            }
            ' ' | '\t' | '\n' | '\r' => {
                // Skip whitespace (TeX ignores spaces in math mode)
                chars.next();
            }
            _ => {
                chars.next();
                tokens.push(Token::Char(ch));
            }
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse_expression(tokens: &[Token], pos: &mut usize, depth: &mut usize) -> Vec<MathNode> {
    *depth += 1;
    if *depth > MAX_MATH_DEPTH {
        return vec![];
    }
    let mut nodes = Vec::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::RBrace | Token::RBracket => break,
            Token::Ampersand | Token::Backslash => break,
            _ => {
                if *depth > MAX_MATH_DEPTH {
                    break;
                }
                if let Some(node) = parse_atom(tokens, pos, depth) {
                    let node = parse_scripts(node, tokens, pos, depth);
                    nodes.push(node);
                }
            }
        }
    }
    *depth -= 1;
    nodes
}

fn parse_atom(tokens: &[Token], pos: &mut usize, depth: &mut usize) -> Option<MathNode> {
    if *pos >= tokens.len() || *depth > MAX_MATH_DEPTH {
        return None;
    }
    match &tokens[*pos] {
        Token::Char(ch) => {
            let ch = *ch;
            *pos += 1;
            if ch.is_ascii_digit() {
                let mut num = String::new();
                num.push(ch);
                while *pos < tokens.len() {
                    if let Token::Char(c) = &tokens[*pos] {
                        if c.is_ascii_digit() || *c == '.' {
                            num.push(*c);
                            *pos += 1;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Some(MathNode::Number(num))
            } else {
                Some(MathNode::Symbol(ch))
            }
        }
        Token::Command(cmd) => {
            let cmd = cmd.clone();
            *pos += 1;
            parse_command(&cmd, tokens, pos, depth)
        }
        Token::LBrace => {
            *pos += 1;
            let inner = parse_expression(tokens, pos, depth);
            if *pos < tokens.len() && tokens[*pos] == Token::RBrace {
                *pos += 1;
            }
            let node = if inner.len() == 1 {
                inner.into_iter().next().unwrap()
            } else {
                MathNode::Row(inner)
            };
            Some(node)
        }
        _ => {
            *pos += 1;
            None
        }
    }
}

fn parse_group(tokens: &[Token], pos: &mut usize, depth: &mut usize) -> MathNode {
    if *pos < tokens.len() && tokens[*pos] == Token::LBrace {
        *pos += 1;
        let inner = parse_expression(tokens, pos, depth);
        if *pos < tokens.len() && tokens[*pos] == Token::RBrace {
            *pos += 1;
        }
        if inner.len() == 1 {
            inner.into_iter().next().unwrap()
        } else {
            MathNode::Row(inner)
        }
    } else if let Some(atom) = parse_atom(tokens, pos, depth) {
        atom
    } else {
        MathNode::Row(vec![])
    }
}

fn parse_scripts(base: MathNode, tokens: &[Token], pos: &mut usize, depth: &mut usize) -> MathNode {
    let mut node = base;
    loop {
        if *pos >= tokens.len() {
            break;
        }
        match &tokens[*pos] {
            Token::Caret => {
                *pos += 1;
                let sup = parse_group(tokens, pos, depth);
                // Check if there's also a subscript
                if *pos < tokens.len() && tokens[*pos] == Token::Underscore {
                    *pos += 1;
                    let sub = parse_group(tokens, pos, depth);
                    node = MathNode::SubSup {
                        base: Box::new(node),
                        sub: Box::new(sub),
                        sup: Box::new(sup),
                    };
                } else {
                    node = MathNode::Superscript {
                        base: Box::new(node),
                        sup: Box::new(sup),
                    };
                }
            }
            Token::Underscore => {
                *pos += 1;
                let sub = parse_group(tokens, pos, depth);
                // Check if there's also a superscript
                if *pos < tokens.len() && tokens[*pos] == Token::Caret {
                    *pos += 1;
                    let sup = parse_group(tokens, pos, depth);
                    node = MathNode::SubSup {
                        base: Box::new(node),
                        sub: Box::new(sub),
                        sup: Box::new(sup),
                    };
                } else {
                    node = MathNode::Subscript {
                        base: Box::new(node),
                        sub: Box::new(sub),
                    };
                }
            }
            _ => break,
        }
    }
    node
}

fn parse_command(
    cmd: &str,
    tokens: &[Token],
    pos: &mut usize,
    depth: &mut usize,
) -> Option<MathNode> {
    match cmd {
        // Fractions
        "frac" | "dfrac" | "tfrac" => {
            let num = parse_group(tokens, pos, depth);
            let den = parse_group(tokens, pos, depth);
            Some(MathNode::Fraction {
                numerator: Box::new(num),
                denominator: Box::new(den),
            })
        }
        // Roots
        "sqrt" => {
            // Optional index: \sqrt[3]{x}
            let index = if *pos < tokens.len() && tokens[*pos] == Token::LBracket {
                *pos += 1;
                let inner = parse_expression(tokens, pos, depth);
                if *pos < tokens.len() && tokens[*pos] == Token::RBracket {
                    *pos += 1;
                }
                let node = if inner.len() == 1 {
                    inner.into_iter().next().unwrap()
                } else {
                    MathNode::Row(inner)
                };
                Some(Box::new(node))
            } else {
                None
            };
            let radicand = parse_group(tokens, pos, depth);
            Some(MathNode::Root {
                index,
                radicand: Box::new(radicand),
            })
        }
        // Greek lowercase
        "alpha" => Some(MathNode::Greek('\u{03B1}')),
        "beta" => Some(MathNode::Greek('\u{03B2}')),
        "gamma" => Some(MathNode::Greek('\u{03B3}')),
        "delta" => Some(MathNode::Greek('\u{03B4}')),
        "epsilon" | "varepsilon" => Some(MathNode::Greek('\u{03B5}')),
        "zeta" => Some(MathNode::Greek('\u{03B6}')),
        "eta" => Some(MathNode::Greek('\u{03B7}')),
        "theta" | "vartheta" => Some(MathNode::Greek('\u{03B8}')),
        "iota" => Some(MathNode::Greek('\u{03B9}')),
        "kappa" => Some(MathNode::Greek('\u{03BA}')),
        "lambda" => Some(MathNode::Greek('\u{03BB}')),
        "mu" => Some(MathNode::Greek('\u{03BC}')),
        "nu" => Some(MathNode::Greek('\u{03BD}')),
        "xi" => Some(MathNode::Greek('\u{03BE}')),
        "pi" | "varpi" => Some(MathNode::Greek('\u{03C0}')),
        "rho" | "varrho" => Some(MathNode::Greek('\u{03C1}')),
        "sigma" | "varsigma" => Some(MathNode::Greek('\u{03C3}')),
        "tau" => Some(MathNode::Greek('\u{03C4}')),
        "upsilon" => Some(MathNode::Greek('\u{03C5}')),
        "phi" | "varphi" => Some(MathNode::Greek('\u{03C6}')),
        "chi" => Some(MathNode::Greek('\u{03C7}')),
        "psi" => Some(MathNode::Greek('\u{03C8}')),
        "omega" => Some(MathNode::Greek('\u{03C9}')),
        // Greek uppercase
        "Gamma" => Some(MathNode::Greek('\u{0393}')),
        "Delta" => Some(MathNode::Greek('\u{0394}')),
        "Theta" => Some(MathNode::Greek('\u{0398}')),
        "Lambda" => Some(MathNode::Greek('\u{039B}')),
        "Xi" => Some(MathNode::Greek('\u{039E}')),
        "Pi" => Some(MathNode::Greek('\u{03A0}')),
        "Sigma" => Some(MathNode::Greek('\u{03A3}')),
        "Upsilon" => Some(MathNode::Greek('\u{03A5}')),
        "Phi" => Some(MathNode::Greek('\u{03A6}')),
        "Psi" => Some(MathNode::Greek('\u{03A8}')),
        "Omega" => Some(MathNode::Greek('\u{03A9}')),
        // Large operators
        "sum" => Some(MathNode::LargeOp {
            symbol: '\u{2211}',
            limits: true,
        }),
        "prod" => Some(MathNode::LargeOp {
            symbol: '\u{220F}',
            limits: true,
        }),
        "int" => Some(MathNode::LargeOp {
            symbol: '\u{222B}',
            limits: false,
        }),
        "iint" => Some(MathNode::LargeOp {
            symbol: '\u{222C}',
            limits: false,
        }),
        "iiint" => Some(MathNode::LargeOp {
            symbol: '\u{222D}',
            limits: false,
        }),
        "oint" => Some(MathNode::LargeOp {
            symbol: '\u{222E}',
            limits: false,
        }),
        "bigcup" => Some(MathNode::LargeOp {
            symbol: '\u{22C3}',
            limits: true,
        }),
        "bigcap" => Some(MathNode::LargeOp {
            symbol: '\u{22C2}',
            limits: true,
        }),
        "coprod" => Some(MathNode::LargeOp {
            symbol: '\u{2210}',
            limits: true,
        }),
        // Named operators (rendered upright)
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh"
        | "cosh" | "tanh" | "log" | "ln" | "exp" | "det" | "dim" | "ker" | "hom" | "deg"
        | "arg" | "gcd" | "inf" | "sup" | "min" | "max" | "mod" => {
            Some(MathNode::Operator(cmd.to_string()))
        }
        "lim" => Some(MathNode::Operator("lim".into())),
        "limsup" => Some(MathNode::Operator("lim sup".into())),
        "liminf" => Some(MathNode::Operator("lim inf".into())),
        // Relation symbols
        "le" | "leq" => Some(MathNode::Symbol('\u{2264}')),
        "ge" | "geq" => Some(MathNode::Symbol('\u{2265}')),
        "ne" | "neq" => Some(MathNode::Symbol('\u{2260}')),
        "approx" => Some(MathNode::Symbol('\u{2248}')),
        "equiv" => Some(MathNode::Symbol('\u{2261}')),
        "propto" => Some(MathNode::Symbol('\u{221D}')),
        "subset" => Some(MathNode::Symbol('\u{2282}')),
        "supset" => Some(MathNode::Symbol('\u{2283}')),
        "subseteq" => Some(MathNode::Symbol('\u{2286}')),
        "supseteq" => Some(MathNode::Symbol('\u{2287}')),
        "in" => Some(MathNode::Symbol('\u{2208}')),
        "notin" => Some(MathNode::Symbol('\u{2209}')),
        "vdash" => Some(MathNode::Symbol('\u{22A2}')),
        "models" => Some(MathNode::Symbol('\u{22A8}')),
        // Arrows
        "to" | "rightarrow" => Some(MathNode::Symbol('\u{2192}')),
        "leftarrow" => Some(MathNode::Symbol('\u{2190}')),
        "leftrightarrow" => Some(MathNode::Symbol('\u{2194}')),
        "Rightarrow" => Some(MathNode::Symbol('\u{21D2}')),
        "Leftarrow" => Some(MathNode::Symbol('\u{21D0}')),
        "Leftrightarrow" | "iff" => Some(MathNode::Symbol('\u{21D4}')),
        "mapsto" => Some(MathNode::Symbol('\u{21A6}')),
        // Binary operators
        "times" => Some(MathNode::Symbol('\u{00D7}')),
        "div" => Some(MathNode::Symbol('\u{00F7}')),
        "cdot" => Some(MathNode::Symbol('\u{22C5}')),
        "pm" => Some(MathNode::Symbol('\u{00B1}')),
        "mp" => Some(MathNode::Symbol('\u{2213}')),
        "circ" => Some(MathNode::Symbol('\u{2218}')),
        "oplus" => Some(MathNode::Symbol('\u{2295}')),
        "otimes" => Some(MathNode::Symbol('\u{2297}')),
        "cup" => Some(MathNode::Symbol('\u{222A}')),
        "cap" => Some(MathNode::Symbol('\u{2229}')),
        "wedge" | "land" => Some(MathNode::Symbol('\u{2227}')),
        "vee" | "lor" => Some(MathNode::Symbol('\u{2228}')),
        // Misc symbols
        "infty" => Some(MathNode::Symbol('\u{221E}')),
        "partial" => Some(MathNode::Symbol('\u{2202}')),
        "nabla" => Some(MathNode::Symbol('\u{2207}')),
        "forall" => Some(MathNode::Symbol('\u{2200}')),
        "exists" => Some(MathNode::Symbol('\u{2203}')),
        "neg" | "lnot" => Some(MathNode::Symbol('\u{00AC}')),
        "emptyset" | "varnothing" => Some(MathNode::Symbol('\u{2205}')),
        "aleph" => Some(MathNode::Symbol('\u{2135}')),
        "ell" => Some(MathNode::Symbol('\u{2113}')),
        "hbar" => Some(MathNode::Symbol('\u{210F}')),
        "Re" => Some(MathNode::Symbol('\u{211C}')),
        "Im" => Some(MathNode::Symbol('\u{2111}')),
        "dots" | "ldots" => Some(MathNode::Symbol('\u{2026}')),
        "cdots" => Some(MathNode::Symbol('\u{22EF}')),
        "vdots" => Some(MathNode::Symbol('\u{22EE}')),
        "ddots" => Some(MathNode::Symbol('\u{22F1}')),
        "prime" => Some(MathNode::Symbol('\u{2032}')),
        // Spacing
        "thinspace" => Some(MathNode::Space(3.0 / 18.0)), // 3mu
        "thickspace" => Some(MathNode::Space(5.0 / 18.0)), // 5mu
        "negspace" => Some(MathNode::Space(-3.0 / 18.0)),
        "space" => Some(MathNode::Space(4.0 / 18.0)),
        "quad" => Some(MathNode::Space(1.0)),
        "qquad" => Some(MathNode::Space(2.0)),
        // Accents
        "hat" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{0302}', // combining circumflex
                body: Box::new(body),
            })
        }
        "bar" | "overline" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{0304}', // combining macron
                body: Box::new(body),
            })
        }
        "vec" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{20D7}', // combining right arrow above
                body: Box::new(body),
            })
        }
        "dot" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{0307}', // combining dot above
                body: Box::new(body),
            })
        }
        "ddot" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{0308}', // combining diaeresis
                body: Box::new(body),
            })
        }
        "tilde" | "widetilde" => {
            let body = parse_group(tokens, pos, depth);
            Some(MathNode::Accent {
                accent: '\u{0303}', // combining tilde
                body: Box::new(body),
            })
        }
        // Delimiters
        "left" => {
            let open = parse_delimiter(tokens, pos);
            let inner = parse_expression(tokens, pos, depth);
            // Expect \right
            let close = if *pos < tokens.len() {
                if let Token::Command(ref c) = tokens[*pos] {
                    if c == "right" {
                        *pos += 1;
                        parse_delimiter(tokens, pos)
                    } else {
                        ')'
                    }
                } else {
                    ')'
                }
            } else {
                ')'
            };
            let body = if inner.len() == 1 {
                inner.into_iter().next().unwrap()
            } else {
                MathNode::Row(inner)
            };
            Some(MathNode::Delimited {
                open,
                close,
                body: Box::new(body),
            })
        }
        "right" => None, // handled by \left
        // Text
        "text" | "mathrm" | "textrm" | "textit" | "mathit" | "textbf" | "mathbf" | "mathsf"
        | "mathtt" | "mathcal" | "mathbb" | "mathfrak" => {
            let text = parse_text_group(tokens, pos);
            Some(MathNode::Text(text))
        }
        // Matrix environments
        "begin" => {
            let env = parse_text_group(tokens, pos);
            parse_environment(&env, tokens, pos, depth)
        }
        "end" => {
            // Skip the environment name
            let _ = parse_text_group(tokens, pos);
            None
        }
        // Unknown command: render as text
        _ => Some(MathNode::Text(format!("\\{cmd}"))),
    }
}

fn parse_delimiter(tokens: &[Token], pos: &mut usize) -> char {
    if *pos >= tokens.len() {
        return '.';
    }
    match &tokens[*pos] {
        Token::Char(c) => {
            let c = *c;
            *pos += 1;
            c
        }
        Token::Command(cmd) => {
            *pos += 1;
            match cmd.as_str() {
                "langle" => '\u{27E8}',
                "rangle" => '\u{27E9}',
                "lfloor" => '\u{230A}',
                "rfloor" => '\u{230B}',
                "lceil" => '\u{2308}',
                "rceil" => '\u{2309}',
                "lvert" | "rvert" => '|',
                "lVert" | "rVert" => '\u{2016}',
                _ => '.',
            }
        }
        Token::LBrace => {
            *pos += 1;
            '{'
        }
        Token::RBrace => {
            *pos += 1;
            '}'
        }
        _ => {
            *pos += 1;
            '.'
        }
    }
}

fn parse_text_group(tokens: &[Token], pos: &mut usize) -> String {
    let mut text = String::new();
    if *pos < tokens.len() && tokens[*pos] == Token::LBrace {
        *pos += 1;
        let mut depth = 1;
        while *pos < tokens.len() && depth > 0 {
            match &tokens[*pos] {
                Token::LBrace => {
                    depth += 1;
                    text.push('{');
                }
                Token::RBrace => {
                    depth -= 1;
                    if depth > 0 {
                        text.push('}');
                    }
                }
                Token::Char(c) => text.push(*c),
                Token::Command(c) => {
                    text.push('\\');
                    text.push_str(c);
                }
                Token::Caret => text.push('^'),
                Token::Underscore => text.push('_'),
                Token::Ampersand => text.push('&'),
                Token::Backslash => text.push_str("\\\\"),
                Token::LBracket => text.push('['),
                Token::RBracket => text.push(']'),
            }
            *pos += 1;
        }
    }
    text
}

fn parse_environment(
    env: &str,
    tokens: &[Token],
    pos: &mut usize,
    depth: &mut usize,
) -> Option<MathNode> {
    let delims = match env {
        "pmatrix" => ('(', ')'),
        "bmatrix" => ('[', ']'),
        "Bmatrix" => ('{', '}'),
        "vmatrix" => ('|', '|'),
        "Vmatrix" => ('\u{2016}', '\u{2016}'),
        "matrix" => ('.', '.'),
        "cases" => ('{', '.'),
        _ => ('.', '.'),
    };

    let mut rows: Vec<Vec<MathNode>> = Vec::new();
    let mut current_row: Vec<MathNode> = Vec::new();

    loop {
        if *pos >= tokens.len() {
            break;
        }
        // Check for \end
        if let Token::Command(ref c) = tokens[*pos] {
            if c == "end" {
                *pos += 1;
                let _ = parse_text_group(tokens, pos);
                break;
            }
        }
        match &tokens[*pos] {
            Token::Ampersand => {
                *pos += 1;
                let cell_nodes = std::mem::take(&mut current_row);
                let cell = if cell_nodes.len() == 1 {
                    cell_nodes.into_iter().next().unwrap()
                } else {
                    MathNode::Row(cell_nodes)
                };
                // We store cells directly; build row at \\
                if let Some(last_row) = rows.last_mut() {
                    last_row.push(cell);
                } else {
                    rows.push(vec![cell]);
                }
            }
            Token::Backslash => {
                *pos += 1;
                // End current cell, end current row
                let cell_nodes = std::mem::take(&mut current_row);
                let cell = if cell_nodes.len() == 1 {
                    cell_nodes.into_iter().next().unwrap()
                } else {
                    MathNode::Row(cell_nodes)
                };
                if let Some(last_row) = rows.last_mut() {
                    last_row.push(cell);
                } else {
                    rows.push(vec![cell]);
                }
                rows.push(Vec::new()); // start new row
            }
            _ => {
                let nodes = parse_expression(tokens, pos, depth);
                current_row.extend(nodes);
            }
        }
    }

    // Flush remaining
    if !current_row.is_empty() {
        let cell = if current_row.len() == 1 {
            current_row.into_iter().next().unwrap()
        } else {
            MathNode::Row(current_row)
        };
        if let Some(last_row) = rows.last_mut() {
            last_row.push(cell);
        } else {
            rows.push(vec![cell]);
        }
    }

    // Remove empty trailing rows
    while rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }

    Some(MathNode::Matrix {
        rows,
        delimiters: delims,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_symbol() {
        assert_eq!(parse_math("x"), MathNode::Symbol('x'));
    }

    #[test]
    fn number() {
        assert_eq!(parse_math("42"), MathNode::Number("42".into()));
    }

    #[test]
    fn superscript() {
        let ast = parse_math("x^2");
        assert!(matches!(ast, MathNode::Superscript { .. }));
    }

    #[test]
    fn subscript() {
        let ast = parse_math("a_i");
        assert!(matches!(ast, MathNode::Subscript { .. }));
    }

    #[test]
    fn subsup() {
        let ast = parse_math("x_i^2");
        assert!(matches!(ast, MathNode::SubSup { .. }));
    }

    #[test]
    fn fraction() {
        let ast = parse_math("\\frac{a}{b}");
        assert!(matches!(ast, MathNode::Fraction { .. }));
    }

    #[test]
    fn sqrt_simple() {
        let ast = parse_math("\\sqrt{x}");
        match ast {
            MathNode::Root { index, .. } => assert!(index.is_none()),
            _ => panic!("Expected Root"),
        }
    }

    #[test]
    fn sqrt_with_index() {
        let ast = parse_math("\\sqrt[3]{x}");
        match ast {
            MathNode::Root { index, .. } => assert!(index.is_some()),
            _ => panic!("Expected Root"),
        }
    }

    #[test]
    fn greek() {
        assert_eq!(parse_math("\\alpha"), MathNode::Greek('\u{03B1}'));
        assert_eq!(parse_math("\\Omega"), MathNode::Greek('\u{03A9}'));
    }

    #[test]
    fn large_op() {
        let ast = parse_math("\\sum");
        assert!(matches!(ast, MathNode::LargeOp { .. }));
    }

    #[test]
    fn named_operator() {
        let ast = parse_math("\\sin");
        assert_eq!(ast, MathNode::Operator("sin".into()));
    }

    #[test]
    fn delimited() {
        let ast = parse_math("\\left(x\\right)");
        assert!(matches!(ast, MathNode::Delimited { .. }));
    }

    #[test]
    fn accent() {
        let ast = parse_math("\\hat{x}");
        assert!(matches!(ast, MathNode::Accent { .. }));
    }

    #[test]
    fn text() {
        let ast = parse_math("\\text{hello}");
        assert_eq!(ast, MathNode::Text("hello".into()));
    }

    #[test]
    fn matrix() {
        let ast = parse_math("\\begin{pmatrix}a&b\\\\c&d\\end{pmatrix}");
        match ast {
            MathNode::Matrix { rows, delimiters } => {
                assert_eq!(delimiters, ('(', ')'));
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].len(), 2);
            }
            _ => panic!("Expected Matrix"),
        }
    }

    #[test]
    fn complex_expression() {
        // E = mc^2
        let ast = parse_math("E = mc^2");
        assert!(matches!(ast, MathNode::Row(_)));
    }

    #[test]
    fn nested_fractions() {
        let ast = parse_math("\\frac{\\frac{a}{b}}{c}");
        match ast {
            MathNode::Fraction { numerator, .. } => {
                assert!(matches!(*numerator, MathNode::Fraction { .. }));
            }
            _ => panic!("Expected Fraction"),
        }
    }

    #[test]
    fn relation_symbols() {
        assert_eq!(parse_math("\\leq"), MathNode::Symbol('\u{2264}'));
        assert_eq!(parse_math("\\neq"), MathNode::Symbol('\u{2260}'));
        assert_eq!(parse_math("\\in"), MathNode::Symbol('\u{2208}'));
    }

    #[test]
    fn arrows() {
        assert_eq!(parse_math("\\to"), MathNode::Symbol('\u{2192}'));
        assert_eq!(parse_math("\\Rightarrow"), MathNode::Symbol('\u{21D2}'));
    }

    #[test]
    fn spaces() {
        let ast = parse_math("a\\,b");
        assert!(matches!(ast, MathNode::Row(_)));
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_math(""), MathNode::Row(vec![]));
    }

    #[test]
    fn mismatched_braces() {
        // Should not panic
        let _ = parse_math("{x^{2}");
        let _ = parse_math("x}}}}");
        let _ = parse_math("\\frac{a}");
    }

    #[test]
    fn atom_types() {
        assert_eq!(MathNode::Symbol('+').atom_type(), AtomType::Bin);
        assert_eq!(MathNode::Symbol('=').atom_type(), AtomType::Rel);
        assert_eq!(MathNode::Symbol('(').atom_type(), AtomType::Open);
        assert_eq!(MathNode::Symbol(')').atom_type(), AtomType::Close);
        assert_eq!(MathNode::Symbol('x').atom_type(), AtomType::Ord);
    }

    #[test]
    fn deeply_nested_braces_no_stack_overflow() {
        // Build {{{...{x}...}}} with 60 levels (beyond MAX_MATH_DEPTH of 50)
        let mut expr = "x".to_string();
        for _ in 0..60 {
            expr = format!("{{{expr}}}");
        }
        // Should not panic or stack overflow, depth limit kicks in
        let _ = parse_math(&expr);
    }
}
