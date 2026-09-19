//! A `.env`/`.env.example`-file parser, more thorough than the minimal
//! one-quote-layer parsers copied around this monorepo (see `strictsubst`'s
//! and `procrun`'s own copies) because here the parsing *is* the product —
//! `envdrift` diffs two of these files against each other, so getting the
//! edge cases right (quoting, `export`, multi-line values) directly
//! determines whether a reported "drift" is real or a parsing artifact.
//!
//! Supported:
//! - blank lines and `#`-prefixed comment lines, skipped
//! - a leading `export ` (shell-style) prefix on the key, stripped
//! - unquoted values, taken verbatim to end-of-line (trailing whitespace
//!   trimmed); a literal `#` inside an unquoted value is **not** treated as
//!   a comment starting mid-line — many real values (URLs with fragments,
//!   connection strings) legitimately contain `#`, and real dotenv loaders
//!   (e.g. the Node `dotenv` package) don't strip inline comments either
//! - `"double quoted"` and `'single quoted'` values, with the surrounding
//!   quotes removed
//! - a small set of backslash escapes inside double-quoted values only
//!   (`\"`, `\\`, `\n`, `\t`) — matching that single-quoted values are
//!   literal in real shells, and double-quoted ones support escapes
//! - a quoted value whose closing quote is on a *later* line (a genuine
//!   multi-line value); the intervening physical lines are joined with `\n`
//!
//! Later duplicate keys overwrite earlier ones, matching both real shell
//! `source`-ing of the file and every mainstream dotenv loader.
//!
//! Not supported (see the crate README's "Not done" section for the
//! reasoning): `${OTHER_VAR}` interpolation some dotenv dialects support
//! inside values, and command substitution — both are treated as inert
//! literal text here, which is the safe reading for a *linter* that must
//! never execute anything found in a `.env` file.

use std::collections::BTreeMap;

/// One raw `KEY=value` assignment, in source order, before being collapsed
/// into a map (later duplicates win in the map; `parse_entries` keeps both
/// so callers who care about order/duplicates can see them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEntry {
    pub key: String,
    pub value: String,
}

/// Parses `.env`-style text into a `KEY -> value` map. Duplicate keys: last
/// one wins.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for entry in parse_entries(text) {
        map.insert(entry.key, entry.value);
    }
    map
}

/// Parses `.env`-style text into an ordered list of assignments, preserving
/// duplicates and source order (the map-returning [`parse`] is what most
/// callers want; this is here for anything that cares about order).
pub fn parse_entries(text: &str) -> Vec<EnvEntry> {
    let lines: Vec<&str> = text.lines().collect();
    let mut entries = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        i += 1;
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let rest = strip_export(trimmed);
        let Some((key_part, value_part)) = rest.split_once('=') else {
            continue;
        };
        let key = key_part.trim();
        if key.is_empty() {
            continue;
        }
        let value_part = value_part.trim_start();
        let (value, next) = parse_value(value_part, &lines, i);
        i = next;
        entries.push(EnvEntry {
            key: key.to_string(),
            value,
        });
    }
    entries
}

fn strip_export(line: &str) -> &str {
    line.strip_prefix("export ")
        .or_else(|| line.strip_prefix("export\t"))
        .map(|rest| rest.trim_start())
        .unwrap_or(line)
}

/// Parses the value portion of a `KEY=<here>` line. `lines`/`next` give
/// access to subsequent physical lines in case a quoted value doesn't close
/// on the same line. Returns the parsed value and the index of the first
/// line not consumed.
fn parse_value(first: &str, lines: &[&str], next: usize) -> (String, usize) {
    let mut chars = first.chars();
    match chars.next() {
        Some(quote @ ('"' | '\'')) => {
            let body_start = chars.as_str();
            if let Some(end) = find_unescaped(body_start, quote) {
                return (unescape(&body_start[..end], quote), next);
            }
            // No closing quote on this line: a genuine multi-line value.
            // Keep consuming physical lines, joined with '\n', until one
            // contains the matching closing quote.
            let mut collected = String::from(body_start);
            let mut i = next;
            while i < lines.len() {
                let line = lines[i];
                i += 1;
                if let Some(end) = find_unescaped(line, quote) {
                    collected.push('\n');
                    collected.push_str(&line[..end]);
                    return (unescape(&collected, quote), i);
                }
                collected.push('\n');
                collected.push_str(line);
            }
            // Never closed — treat the rest of the file as the value rather
            // than panicking or silently truncating.
            (unescape(&collected, quote), i)
        }
        _ => (first.trim_end().to_string(), next),
    }
}

/// Finds the index of the first occurrence of `quote` in `s` that isn't
/// escaped with a backslash (escaping only recognized for double quotes,
/// matching real shell semantics where single quotes can't contain an
/// escaped quote at all).
fn find_unescaped(s: &str, quote: char) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if quote == '"' && c == '\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if c == quote {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Applies the small backslash-escape set supported inside double-quoted
/// values. Single-quoted values are returned verbatim (no escape
/// processing), matching real shell behavior.
fn unescape(s: &str, quote: char) -> String {
    if quote != '"' {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('t') => {
                    out.push('\t');
                    chars.next();
                }
                Some('"') => {
                    out.push('"');
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_key_value_pairs() {
        let vars = parse("PORT=3000\nNAME=app\n");
        assert_eq!(vars.get("PORT"), Some(&"3000".to_string()));
        assert_eq!(vars.get("NAME"), Some(&"app".to_string()));
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        let vars = parse("\n# a comment\n   \nKEY=value\n# trailing comment\n");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
    }

    #[test]
    fn strips_double_quotes() {
        let vars = parse("NAME=\"hello world\"\n");
        assert_eq!(vars.get("NAME"), Some(&"hello world".to_string()));
    }

    #[test]
    fn strips_single_quotes() {
        let vars = parse("NAME='hello world'\n");
        assert_eq!(vars.get("NAME"), Some(&"hello world".to_string()));
    }

    #[test]
    fn handles_export_prefix() {
        let vars = parse("export DATABASE_URL=postgres://localhost/db\n");
        assert_eq!(
            vars.get("DATABASE_URL"),
            Some(&"postgres://localhost/db".to_string())
        );
    }

    #[test]
    fn handles_export_with_tab() {
        let vars = parse("export\tPORT=3000\n");
        assert_eq!(vars.get("PORT"), Some(&"3000".to_string()));
    }

    #[test]
    fn empty_unquoted_value() {
        let vars = parse("API_KEY=\n");
        assert_eq!(vars.get("API_KEY"), Some(&"".to_string()));
    }

    #[test]
    fn empty_quoted_value() {
        let vars = parse("API_KEY=\"\"\n");
        assert_eq!(vars.get("API_KEY"), Some(&"".to_string()));
    }

    #[test]
    fn unquoted_hash_is_not_a_comment_marker() {
        // A literal '#' inside an unquoted value (URL fragment, etc.) must
        // survive — only a line *starting* with '#' is a comment.
        let vars = parse("REDIRECT_URL=https://example.com/callback#done\n");
        assert_eq!(
            vars.get("REDIRECT_URL"),
            Some(&"https://example.com/callback#done".to_string())
        );
    }

    #[test]
    fn trailing_content_after_closing_quote_is_ignored() {
        let vars = parse("KEY=\"value\"   # trailing comment\n");
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
    }

    #[test]
    fn multiline_double_quoted_value() {
        let vars = parse("CERT=\"line one\nline two\nline three\"\nAFTER=1\n");
        assert_eq!(
            vars.get("CERT"),
            Some(&"line one\nline two\nline three".to_string())
        );
        assert_eq!(vars.get("AFTER"), Some(&"1".to_string()));
    }

    #[test]
    fn multiline_single_quoted_value() {
        let vars = parse("CERT='line one\nline two'\n");
        assert_eq!(vars.get("CERT"), Some(&"line one\nline two".to_string()));
    }

    #[test]
    fn escaped_double_quote_inside_double_quoted_value() {
        let vars = parse(r#"MSG="say \"hi\" now""#);
        assert_eq!(vars.get("MSG"), Some(&"say \"hi\" now".to_string()));
    }

    #[test]
    fn single_quotes_do_not_process_escapes() {
        // Real shells never process backslash escapes inside single quotes.
        let vars = parse(r#"MSG='say \"hi\" now'"#);
        assert_eq!(vars.get("MSG"), Some(&r#"say \"hi\" now"#.to_string()));
    }

    #[test]
    fn duplicate_keys_last_one_wins() {
        let vars = parse("KEY=first\nKEY=second\n");
        assert_eq!(vars.get("KEY"), Some(&"second".to_string()));
        assert_eq!(vars.len(), 1);
    }

    #[test]
    fn lines_without_equals_are_skipped() {
        let vars = parse("not a valid line\nKEY=value\n");
        assert_eq!(vars.len(), 1);
    }

    #[test]
    fn crlf_line_endings() {
        let vars = parse("KEY=value\r\nOTHER=\"quoted\"\r\n");
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
        assert_eq!(vars.get("OTHER"), Some(&"quoted".to_string()));
    }

    #[test]
    fn whitespace_around_key_and_value_trimmed() {
        let vars = parse("  KEY   =   value  \n");
        assert_eq!(vars.get("KEY"), Some(&"value".to_string()));
    }

    #[test]
    fn unterminated_quote_consumes_rest_of_file_without_panicking() {
        let vars = parse("KEY=\"never closed\nmore text\n");
        assert_eq!(vars.len(), 1);
        assert_eq!(
            vars.get("KEY"),
            Some(&"never closed\nmore text".to_string())
        );
    }

    #[test]
    fn parse_entries_preserves_order_and_duplicates() {
        let entries = parse_entries("A=1\nB=2\nA=3\n");
        assert_eq!(
            entries,
            vec![
                EnvEntry {
                    key: "A".into(),
                    value: "1".into()
                },
                EnvEntry {
                    key: "B".into(),
                    value: "2".into()
                },
                EnvEntry {
                    key: "A".into(),
                    value: "3".into()
                },
            ]
        );
    }
}
