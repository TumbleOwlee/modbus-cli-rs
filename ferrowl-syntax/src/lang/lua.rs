//! Lua 5.4 lexer. Best-effort: never panics on malformed input, doesn't need to reject
//! every invalid form. Operates one line at a time, threading [`LineState`] across calls
//! for long strings/comments that span lines.

use crate::{LineState, LuaCarry, SyntaxKind};

use super::scan;

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local",
    "not", "or", "repeat", "return", "then", "until", "while",
];
const LITERALS: &[&str] = &["true", "false", "nil"];

/// Multi-char operators, longest first so matching greedily picks the right one.
const OPS: &[&str] = &[
    "...", "..", "::", "==", "~=", "<=", ">=", "<<", ">>", "+", "-", "*", "/", "%", "^", "#", "&",
    "~", "|", "<", ">", "=", "(", ")", "{", "}", "[", "]", ";", ":", ",", ".",
];

pub(crate) fn highlight_line(
    line: &str,
    state: LineState,
) -> (Vec<(usize, usize, SyntaxKind)>, LineState) {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut spans = Vec::new();
    let mut i = 0usize;
    let mut carry = state.0;

    // Resume a long string/comment opened on a previous line.
    if let Some(level) = match carry {
        LuaCarry::LongString(l) | LuaCarry::LongComment(l) => Some(l),
        LuaCarry::None => None,
    } {
        let kind = match carry {
            LuaCarry::LongComment(_) => SyntaxKind::Comment,
            _ => SyntaxKind::String,
        };
        match find_long_close(&chars, 0, level) {
            Some(end) => {
                spans.push((0, end, kind));
                i = end;
                carry = LuaCarry::None;
            }
            None => {
                spans.push((0, len, kind));
                return (spans, LineState(carry));
            }
        }
    }

    while i < len {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }

        if c == '-' && i + 1 < len && chars[i + 1] == '-' {
            if let Some((level, open_end)) = match_long_open(&chars, i + 2) {
                match find_long_close(&chars, open_end, level) {
                    Some(end) => {
                        spans.push((i, end, SyntaxKind::Comment));
                        i = end;
                    }
                    None => {
                        spans.push((i, len, SyntaxKind::Comment));
                        carry = LuaCarry::LongComment(level);
                        break;
                    }
                }
            } else {
                spans.push((i, len, SyntaxKind::Comment));
                break;
            }
            continue;
        }

        if c == '['
            && let Some((level, open_end)) = match_long_open(&chars, i)
        {
            match find_long_close(&chars, open_end, level) {
                Some(end) => {
                    spans.push((i, end, SyntaxKind::String));
                    i = end;
                }
                None => {
                    spans.push((i, len, SyntaxKind::String));
                    carry = LuaCarry::LongString(level);
                    break;
                }
            }
            continue;
        }

        if c == '"' || c == '\'' {
            let start = i;
            i = scan::scan_quoted(&chars, i, c);
            spans.push((start, i, SyntaxKind::String));
            continue;
        }

        if c.is_ascii_digit() || (c == '.' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let start = i;
            i = parse_number(&chars, i);
            spans.push((start, i, SyntaxKind::Number));
            continue;
        }

        if c == '_' || c.is_alphabetic() {
            let start = i;
            while i < len && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let kind = if KEYWORDS.contains(&word.as_str()) {
                SyntaxKind::Keyword
            } else if LITERALS.contains(&word.as_str()) {
                SyntaxKind::Literal
            } else {
                SyntaxKind::Ident
            };
            spans.push((start, i, kind));
            continue;
        }

        if let Some(op) = OPS.iter().find(|op| starts_with(&chars, i, op)) {
            let end = i + op.chars().count();
            spans.push((i, end, SyntaxKind::Punct));
            i = end;
            continue;
        }

        // Unrecognized character: no span, just skip it.
        i += 1;
    }

    contextual_pass(&chars, &mut spans);
    (spans, LineState(carry))
}

/// Second pass: reclassify plain identifiers by syntactic context. An identifier
/// directly followed by a call (`(`, `{`, or a string argument) or directly preceded by
/// `:` (method position) becomes [`SyntaxKind::Function`]; an identifier followed by
/// `.`/`:` plus another identifier becomes [`SyntaxKind::Object`] (e.g. `C_Register` in
/// `C_Register:Set(1)`).
fn contextual_pass(chars: &[char], spans: &mut [(usize, usize, SyntaxKind)]) {
    for k in 0..spans.len() {
        if spans[k].2 != SyntaxKind::Ident {
            continue;
        }
        let method_pos = k > 0 && is_punct(chars, spans[k - 1], ":");
        let call_pos = spans.get(k + 1).is_some_and(|n| {
            n.2 == SyntaxKind::String || is_punct(chars, *n, "(") || is_punct(chars, *n, "{")
        });
        let access_pos = spans
            .get(k + 1)
            .zip(spans.get(k + 2))
            .is_some_and(|(n, n2)| {
                (is_punct(chars, *n, ".") || is_punct(chars, *n, ":")) && n2.2 == SyntaxKind::Ident
            });
        if method_pos || call_pos {
            spans[k].2 = SyntaxKind::Function;
        } else if access_pos {
            spans[k].2 = SyntaxKind::Object;
        }
    }
}

/// True when the span is a `Punct` whose text is exactly `pat` (so `.` never matches
/// the `..` concat operator or `:` the `::` label marker).
fn is_punct(chars: &[char], span: (usize, usize, SyntaxKind), pat: &str) -> bool {
    span.2 == SyntaxKind::Punct
        && span.1 - span.0 == pat.chars().count()
        && chars[span.0..span.1].iter().copied().eq(pat.chars())
}

/// Checks whether `chars[pos..]` starts with a long-bracket opener `[`, `=`*N, `[` and, if
/// so, returns `(N, index_after_opener)`.
fn match_long_open(chars: &[char], pos: usize) -> Option<(u8, usize)> {
    let len = chars.len();
    if pos >= len || chars[pos] != '[' {
        return None;
    }
    let mut p = pos + 1;
    let mut level = 0u8;
    while p < len && chars[p] == '=' {
        level += 1;
        p += 1;
    }
    if p < len && chars[p] == '[' {
        Some((level, p + 1))
    } else {
        None
    }
}

/// Scans `chars[from..]` for a closing long bracket `]`, `=`*level, `]` at the exact
/// level given. Returns the index just past the closer, or `None` if not found on this
/// line.
fn find_long_close(chars: &[char], from: usize, level: u8) -> Option<usize> {
    let len = chars.len();
    let level = level as usize;
    let mut p = from;
    while p < len {
        if chars[p] == ']' {
            let eq_end = p + 1 + level;
            if eq_end < len
                && chars[p + 1..eq_end].iter().all(|&c| c == '=')
                && chars[eq_end] == ']'
            {
                return Some(eq_end + 1);
            }
        }
        p += 1;
    }
    None
}

fn parse_number(chars: &[char], start: usize) -> usize {
    let len = chars.len();
    let mut i = start;
    if chars[i] == '0' && i + 1 < len && (chars[i + 1] == 'x' || chars[i + 1] == 'X') {
        i += 2;
        while i < len && chars[i].is_ascii_hexdigit() {
            i += 1;
        }
        if i < len && chars[i] == '.' {
            i += 1;
            while i < len && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
        }
        if i < len && (chars[i] == 'p' || chars[i] == 'P') {
            i += 1;
            if i < len && (chars[i] == '+' || chars[i] == '-') {
                i += 1;
            }
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
        return i;
    }

    while i < len && chars[i].is_ascii_digit() {
        i += 1;
    }
    scan::scan_fraction_exponent(chars, i, true)
}

fn starts_with(chars: &[char], pos: usize, pat: &str) -> bool {
    for (p, pc) in (pos..).zip(pat.chars()) {
        if p >= chars.len() || chars[p] != pc {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Language, highlight_line as top_highlight_line};

    fn spans_for(line: &str) -> Vec<(usize, usize, SyntaxKind)> {
        top_highlight_line(Language::Lua, line, LineState::default()).0
    }

    #[test]
    /// UI-R-037 — highlight spans are sorted by start, non-overlapping, and use character indices.
    fn ut_spans_sorted_non_overlapping() {
        let spans = spans_for("local x = 1 + foo -- comment");
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0);
            assert!(w[0].0 < w[0].1);
        }
    }

    #[test]
    /// UI-R-038 — a Lua long string is highlighted as that construct on every line from the one
    /// carrying the opening delimiter through the one carrying the closing delimiter, via
    /// carry-over state, rather than restarting as ordinary code partway through the closing
    /// line.
    fn ut_long_string_carries_across_lines() {
        let (spans1, state1) =
            top_highlight_line(Language::Lua, "local s = [==[", LineState::default());
        assert_eq!(state1.0, LuaCarry::LongString(2));
        assert!(spans1.iter().any(|s| s.2 == SyntaxKind::String));

        let (spans2, state2) = top_highlight_line(Language::Lua, "still inside", state1);
        assert_eq!(state2.0, LuaCarry::LongString(2));
        // The whole line should be swallowed as string, not tokenized as an identifier.
        assert_eq!(spans2, vec![(0, 12, SyntaxKind::String)]);

        let (spans3, state3) = top_highlight_line(Language::Lua, "]==]", state2);
        assert_eq!(state3.0, LuaCarry::None);
        // The closing-delimiter line itself is highlighted as String through the closer, not
        // left untokenized once the carry state is consumed.
        assert_eq!(spans3, vec![(0, 4, SyntaxKind::String)]);
    }

    #[test]
    /// UI-R-038 — a Lua long comment is highlighted as that construct on every line from the one
    /// carrying the opening delimiter through the one carrying the closing delimiter, via
    /// carry-over state, rather than restarting as ordinary code partway through the closing
    /// line.
    fn ut_long_comment_carries_across_lines() {
        let (_spans1, state1) =
            top_highlight_line(Language::Lua, "--[[ start", LineState::default());
        assert_eq!(state1.0, LuaCarry::LongComment(0));

        let (spans2, state2) = top_highlight_line(Language::Lua, "middle line", state1);
        assert_eq!(state2.0, LuaCarry::LongComment(0));
        assert_eq!(spans2, vec![(0, 11, SyntaxKind::Comment)]);

        let (spans3, state3) = top_highlight_line(Language::Lua, "]] print(1)", state2);
        assert_eq!(state3.0, LuaCarry::None);
        // Only the closing delimiter itself is Comment; what follows on that same line is
        // ordinary code, tokenized normally once the construct has actually closed.
        assert_eq!(spans3[0], (0, 2, SyntaxKind::Comment));
        assert!(spans3.iter().skip(1).all(|s| s.2 != SyntaxKind::Comment));
    }

    #[test]
    /// UI-R-038 — carry-over tracks the long-bracket level so a wrong-level closer does not end the construct.
    fn ut_long_bracket_level_mismatch_does_not_close_early() {
        let (_spans1, state1) =
            top_highlight_line(Language::Lua, "local s = [=[", LineState::default());
        assert_eq!(state1.0, LuaCarry::LongString(1));

        // Neither a plain `]]` nor a `]==]` (level 2) should close a level-1 long string.
        let (spans2, state2) = top_highlight_line(Language::Lua, "]] not closed ]==]", state1);
        assert_eq!(state2.0, LuaCarry::LongString(1));
        assert_eq!(spans2.len(), 1);
        assert_eq!(spans2[0].2, SyntaxKind::String);

        let (_spans3, state3) = top_highlight_line(Language::Lua, "]=]", state2);
        assert_eq!(state3.0, LuaCarry::None);
    }

    #[test]
    /// UI-R-037 — an escaped quote inside a string does not split the string span.
    fn ut_escaped_quote_does_not_terminate_string() {
        let spans = spans_for(r#"local s = "a\"b""#);
        let strings: Vec<_> = spans.iter().filter(|s| s.2 == SyntaxKind::String).collect();
        assert_eq!(strings.len(), 1);
    }

    #[test]
    /// UI-R-039 — hex and exponent numerals are one Number-kind span each.
    fn ut_numbers_hex_and_exponent() {
        let spans = spans_for("local a = 0x1F local b = 1e10");
        let nums: Vec<_> = spans.iter().filter(|s| s.2 == SyntaxKind::Number).collect();
        assert_eq!(nums.len(), 2);
    }

    #[test]
    /// UI-R-039 — keywords and literals map to their distinct highlight kinds.
    fn ut_keywords_vs_literals() {
        let spans = spans_for("if true then return nil end");
        let kinds: Vec<_> = spans.iter().map(|s| s.2).collect();
        assert!(kinds.contains(&SyntaxKind::Keyword));
        assert!(kinds.contains(&SyntaxKind::Literal));
    }

    #[test]
    /// UI-R-039 — an identifier in object/method position takes the Object/Function kind.
    fn ut_object_and_method_position() {
        // `C_Register:Set(1)` — object before the `:`, method after it.
        let spans = spans_for("C_Register:Set(1)");
        assert_eq!(spans[0].2, SyntaxKind::Object);
        assert_eq!(spans[2].2, SyntaxKind::Function);
        // Method position wins even without a call: `C_Register:Set`.
        let spans = spans_for("x = C_Register:Set");
        assert_eq!(spans[2].2, SyntaxKind::Object);
        assert_eq!(spans[4].2, SyntaxKind::Function);
    }

    #[test]
    /// UI-R-039 — field-access chains and call sugar resolve to Object/Function kinds.
    fn ut_field_chain_and_call_positions() {
        // `a.b.c` — everything before the last access is an object; `c` stays plain.
        let spans = spans_for("a.b.c");
        assert_eq!(spans[0].2, SyntaxKind::Object);
        assert_eq!(spans[2].2, SyntaxKind::Object);
        assert_eq!(spans[4].2, SyntaxKind::Ident);
        // Plain calls, table-arg and string-arg sugar are all functions.
        let spans = spans_for("print \"x\" foo{1} bar(2)");
        assert_eq!(spans[0].2, SyntaxKind::Function);
        assert_eq!(spans[2].2, SyntaxKind::Function);
        assert_eq!(spans[6].2, SyntaxKind::Function);
    }

    #[test]
    /// UI-R-039 — `..` concat and `::` labels do not produce Object/Function kinds.
    fn ut_concat_and_labels_are_not_access() {
        // `..` is concat, `::` is a label marker — neither makes an object/function.
        let spans = spans_for("a .. b ::lbl::");
        assert!(
            spans
                .iter()
                .all(|s| s.2 != SyntaxKind::Object && s.2 != SyntaxKind::Function)
        );
    }

    #[test]
    /// UI-R-037 — malformed input never panics and every span index stays within the line.
    fn ut_garbage_input_does_not_panic() {
        let cases = [
            "\"abc",
            "local s = \"abc",
            "]",
            "]=]",
            "",
            "--",
            "[==[ unterminated",
        ];
        for case in cases {
            let (spans, _state) = top_highlight_line(Language::Lua, case, LineState::default());
            let len = case.chars().count();
            for (start, end, _) in spans {
                assert!(start <= end);
                assert!(end <= len);
            }
        }
    }

    #[test]
    /// UI-R-037 — spans use character indices, so a multi-byte identifier is one in-range span.
    fn ut_non_ascii_identifier() {
        let line = "local café = 1";
        let spans = spans_for(line);
        let len = line.chars().count();
        for (start, end, _) in &spans {
            assert!(*start <= *end);
            assert!(*end <= len);
        }
        let idents: Vec<_> = spans.iter().filter(|s| s.2 == SyntaxKind::Ident).collect();
        assert_eq!(idents.len(), 1);
    }

    #[test]
    /// UI-R-037 — a string span over multi-byte content is delimited by character indices.
    fn ut_non_ascii_string_content() {
        let line = r#"local msg = "café 日本語""#;
        let spans = spans_for(line);
        let len = line.chars().count();
        for (start, end, _) in &spans {
            assert!(*start <= *end);
            assert!(*end <= len);
        }
        let strings: Vec<_> = spans.iter().filter(|s| s.2 == SyntaxKind::String).collect();
        assert_eq!(strings.len(), 1);
        let (s_start, s_end, _) = *strings[0];
        let string_chars: String = line.chars().skip(s_start).take(s_end - s_start).collect();
        assert!(string_chars.contains("café"));
        assert!(string_chars.contains("日本語"));
    }

    #[test]
    /// UI-R-037 — emoji (multi-byte) content keeps span indices character-based and in range.
    fn ut_emoji_in_string_and_unrecognized() {
        let line = r#"s = "hello 🚀""#;
        let spans = spans_for(line);
        let len = line.chars().count();
        for (start, end, _) in &spans {
            assert!(*start <= *end);
            assert!(*end <= len);
        }
        let strings: Vec<_> = spans.iter().filter(|s| s.2 == SyntaxKind::String).collect();
        assert_eq!(strings.len(), 1);

        let line_emoji_alone = "🚀 local x";
        let spans_emoji = spans_for(line_emoji_alone);
        let len_emoji = line_emoji_alone.chars().count();
        for (start, end, _) in &spans_emoji {
            assert!(*start <= *end);
            assert!(*end <= len_emoji);
        }
    }
}
