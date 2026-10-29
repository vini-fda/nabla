//! Rust syntax highlighting for the source window, in the Catppuccin colors: the language
//! defaults of its style guide (<https://github.com/catppuccin/catppuccin/blob/main/docs/style-guide.md>),
//! as far as a tokenizer without a parser can tell the kinds of tokens apart. Unlike the
//! fallback highlighter in `egui_extras`, it tells types, functions, macros, constants and
//! enum variants apart instead of coloring every identifier alike.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, TextStyle};

use crate::theme::palette;

/// What a token is, as far as its spelling and neighbors tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Comment,
    Keyword,
    String,
    Escape,
    /// Numbers, `true`/`false`, and `SCREAMING_CASE` constants.
    Constant,
    Operator,
    /// Braces, brackets, parentheses, and `,` `;` `:` `::` `.`.
    Delimiter,
    /// Function and method names, where they are called or defined.
    Function,
    /// `self`.
    Builtin,
    /// Primitive types and `CamelCase` names.
    Type,
    /// `CamelCase` names after `CamelCase::`.
    Variant,
    /// A macro name with its `!`.
    Macro,
    /// `#[...]` and `#![...]`.
    Attribute,
    Lifetime,
    /// Whitespace and everything else, such as variable names.
    Plain,
}

fn color(kind: Kind, text_color: Color32) -> Color32 {
    match kind {
        Kind::Comment | Kind::Delimiter => palette::OVERLAY1,
        Kind::Keyword => palette::MAUVE,
        Kind::String => palette::GREEN,
        Kind::Escape => palette::PINK,
        Kind::Constant => palette::PEACH,
        Kind::Operator => palette::SKY,
        Kind::Function => palette::BLUE,
        Kind::Builtin | Kind::Lifetime => palette::RED,
        Kind::Type | Kind::Attribute => palette::YELLOW,
        Kind::Variant => palette::TEAL,
        Kind::Macro => palette::ROSEWATER,
        Kind::Plain => text_color,
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
];

const PRIMITIVES: &[&str] = &[
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "str", "u8", "u16",
    "u32", "u64", "u128", "usize", "Self",
];

const OPERATOR_CHARS: &str = "+-*/%=<>!&|^?";

/// Splits `code` into tokens, which together are exactly `code`.
fn tokens(code: &str) -> Vec<(&str, Kind)> {
    let mut out: Vec<(&str, Kind)> = Vec::new();
    let mut rest = code;
    // Whether the last tokens (whitespace aside) were a `CamelCase` name and `::`.
    let mut after_type_path = false;
    while let Some(c) = rest.chars().next() {
        let (len, kind) = if rest.starts_with("//") {
            (rest.find('\n').unwrap_or(rest.len()), Kind::Comment)
        } else if rest.starts_with("/*") {
            (rest.find("*/").map_or(rest.len(), |i| i + 2), Kind::Comment)
        } else if c == '"' {
            let len = string(rest, &mut out);
            rest = &rest[len..];
            after_type_path = false;
            continue;
        } else if c == '\'' {
            quote(rest)
        } else if rest.starts_with("#[") || rest.starts_with("#![") {
            (attribute(rest), Kind::Attribute)
        } else if c.is_ascii_digit() {
            (number(rest), Kind::Constant)
        } else if c.is_alphabetic() || c == '_' {
            let (len, kind) = identifier(rest, after_type_path);
            after_type_path = kind == Kind::Type && rest[len..].starts_with("::");
            out.push((&rest[..len], kind));
            rest = &rest[len..];
            continue;
        } else if c.is_whitespace() {
            (prefix_len(rest, char::is_whitespace), Kind::Plain)
        } else if rest.starts_with("::") {
            (2, Kind::Delimiter)
        } else if OPERATOR_CHARS.contains(c) {
            (
                prefix_len(rest, |c| OPERATOR_CHARS.contains(c)),
                Kind::Operator,
            )
        } else {
            (c.len_utf8(), Kind::Delimiter)
        };
        if !(kind == Kind::Plain || rest.starts_with("::")) {
            after_type_path = false;
        }
        out.push((&rest[..len], kind));
        rest = &rest[len..];
    }
    out
}

/// Byte length of the longest prefix of `s` whose characters all satisfy `f`.
fn prefix_len(s: &str, f: impl Fn(char) -> bool) -> usize {
    s.find(|c: char| !f(c)).unwrap_or(s.len())
}

/// Pushes the string literal at the start of `s` (opening quote included), its escape
/// sequences apart; returns its byte length.
fn string<'a>(s: &'a str, out: &mut Vec<(&'a str, Kind)>) -> usize {
    let mut start = 0;
    let mut i = 1;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap_or('"');
        if c == '\\' {
            let len = escape(&s[i..]);
            out.push((&s[start..i], Kind::String));
            out.push((&s[i..i + len], Kind::Escape));
            i += len;
            start = i;
        } else {
            i += c.len_utf8();
            if c == '"' {
                break;
            }
        }
    }
    out.push((&s[start..i], Kind::String));
    i
}

/// Byte length of the escape sequence at the start of `s`, such as `\n` or `\u{3b5}`.
fn escape(s: &str) -> usize {
    if s.starts_with("\\u{") {
        s.find('}').map_or(s.len(), |i| i + 1)
    } else {
        1 + s[1..].chars().next().map_or(0, char::len_utf8)
    }
}

/// A character literal such as `'x'` or `'\n'`, or else a lifetime such as `'a`.
fn quote(s: &str) -> (usize, Kind) {
    let body = &s[1..];
    let mut chars = body.chars();
    match (chars.next(), chars.next()) {
        (Some('\\'), _) => {
            let len = 1 + escape(body);
            let close = usize::from(s[len..].starts_with('\''));
            (len + close, Kind::String)
        }
        (Some(c), Some('\'')) => (2 + c.len_utf8(), Kind::String),
        _ => (
            1 + prefix_len(body, |c| c.is_alphanumeric() || c == '_'),
            Kind::Lifetime,
        ),
    }
}

/// Byte length of the attribute at the start of `s`, up to its matching `]`.
fn attribute(s: &str) -> usize {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    s.len()
}

/// Byte length of the number at the start of `s`: digits, `_`, a type suffix such as `f64`,
/// and a decimal point if a digit follows it (so `0..n` is not taken as a number).
fn number(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let decimal_point = b == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
        if !(b.is_ascii_alphanumeric() || b == b'_' || decimal_point) {
            break;
        }
        i += 1;
    }
    i
}

/// The identifier at the start of `s` (with the `!` of a macro), told apart by its spelling
/// and what follows it.
fn identifier(s: &str, after_type_path: bool) -> (usize, Kind) {
    let len = prefix_len(s, |c| c.is_alphanumeric() || c == '_');
    let word = &s[..len];
    if s[len..].starts_with('!') && !s[len..].starts_with("!=") {
        return (len + 1, Kind::Macro);
    }
    let next = s[len..].trim_start();
    let kind = if word == "self" {
        Kind::Builtin
    } else if word == "true" || word == "false" {
        Kind::Constant
    } else if KEYWORDS.contains(&word) {
        Kind::Keyword
    } else if PRIMITIVES.contains(&word) {
        Kind::Type
    } else if word.chars().any(char::is_alphabetic)
        && !word.chars().any(char::is_lowercase)
        && len > 1
    {
        Kind::Constant
    } else if word.starts_with(char::is_uppercase) {
        if after_type_path {
            Kind::Variant
        } else {
            Kind::Type
        }
    } else if next.starts_with('(') || next.starts_with("::<") {
        Kind::Function
    } else {
        Kind::Plain
    };
    (len, kind)
}

/// Highlights Rust `code` in `font`, with `text_color` for plain text such as variable names.
pub fn highlight(code: &str, font: &FontId, text_color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (text, kind) in tokens(code) {
        let format = TextFormat::simple(font.clone(), color(kind, text_color));
        job.append(text, 0.0, format);
    }
    job
}

#[derive(Default)]
struct Highlighter;

impl egui::cache::ComputerMut<(&FontId, Color32, &str), LayoutJob> for Highlighter {
    fn compute(&mut self, (font, text_color, code): (&FontId, Color32, &str)) -> LayoutJob {
        highlight(code, font, text_color)
    }
}

/// Shows Rust `code`, highlighted and selectable. The highlighting is memoized, so this can
/// run every frame.
pub fn code_view_ui(ui: &mut egui::Ui, code: &str) -> egui::Response {
    let font = TextStyle::Monospace.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    let job = ui.ctx().memory_mut(|mem| {
        mem.caches
            .cache::<egui::cache::FrameCache<LayoutJob, Highlighter>>()
            .get((&font, text_color, code))
            .clone()
    });
    ui.add(egui::Label::new(job).selectable(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tokens of `code` that aren't whitespace.
    fn kinds(code: &str) -> Vec<(&str, Kind)> {
        let tokens = tokens(code);
        assert_eq!(tokens.iter().map(|(t, _)| *t).collect::<String>(), code);
        tokens
            .into_iter()
            .filter(|(t, _)| !t.trim().is_empty())
            .collect()
    }

    #[test]
    fn tells_identifiers_apart() {
        use Kind::*;
        assert_eq!(
            kinds("pub fn step(&mut self, s: &State) -> Vec2 { let f = F_MAX.min(1.0); }"),
            [
                ("pub", Keyword),
                ("fn", Keyword),
                ("step", Function),
                ("(", Delimiter),
                ("&", Operator),
                ("mut", Keyword),
                ("self", Builtin),
                (",", Delimiter),
                ("s", Plain),
                (":", Delimiter),
                ("&", Operator),
                ("State", Type),
                (")", Delimiter),
                ("->", Operator),
                ("Vec2", Type),
                ("{", Delimiter),
                ("let", Keyword),
                ("f", Plain),
                ("=", Operator),
                ("F_MAX", Constant),
                (".", Delimiter),
                ("min", Function),
                ("(", Delimiter),
                ("1.0", Constant),
                (")", Delimiter),
                (";", Delimiter),
                ("}", Delimiter),
            ]
        );
    }

    #[test]
    fn variants_macros_and_ranges() {
        use Kind::*;
        assert_eq!(
            kinds("Outcome::Reached; nabla::State; vec![0..n]; x != y"),
            [
                ("Outcome", Type),
                ("::", Delimiter),
                ("Reached", Variant),
                (";", Delimiter),
                ("nabla", Plain),
                ("::", Delimiter),
                ("State", Type),
                (";", Delimiter),
                ("vec!", Macro),
                ("[", Delimiter),
                ("0", Constant),
                (".", Delimiter),
                (".", Delimiter),
                ("n", Plain),
                ("]", Delimiter),
                (";", Delimiter),
                ("x", Plain),
                ("!=", Operator),
                ("y", Plain),
            ]
        );
    }

    #[test]
    fn strings_chars_lifetimes_comments_attributes() {
        use Kind::*;
        assert_eq!(
            kinds("#[derive(Debug)] \"a\\n\u{3b5}\" '\\'' 'x' &'a str // done \u{3b5}"),
            [
                ("#[derive(Debug)]", Attribute),
                ("\"a", String),
                ("\\n", Escape),
                ("\u{3b5}\"", String),
                ("'\\''", String),
                ("'x'", String),
                ("&", Operator),
                ("'a", Lifetime),
                ("str", Type),
                ("// done \u{3b5}", Comment),
            ]
        );
    }
}
