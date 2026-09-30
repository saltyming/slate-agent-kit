//! An order-preserving JSON document model with a reader and a writer.
//!
//! Owns lossless round trips of JSON configuration files: object keys stay in
//! file order, number literals keep their text, and only the indentation and
//! trailing newline are normalised. It exists because enabling `serde_json`'s
//! `preserve_order` would change key ordering for every crate in the workspace
//! through feature unification. It does not know which keys matter to any harness.
//!
//! Main entry points: [`Json::parse`], [`Json::to_pretty`], [`Json::to_compact`]
//! and [`detect_indent`].

use crate::error::{Error, Result};

const MAX_DEPTH: usize = 128;

/// A JSON value whose objects keep insertion order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, kept as its literal text.
    Number(String),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object as an ordered list of members.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Parses a complete JSON document.
    pub fn parse(text: &str) -> Result<Json> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut p = Parser {
            src: text.as_bytes(),
            pos: 0,
        };
        p.skip_ws();
        let v = p.value(0)?;
        p.skip_ws();
        if p.pos != p.src.len() {
            return Err(p.err("unexpected trailing characters"));
        }
        Ok(v)
    }

    /// Creates a string value.
    pub fn str(s: impl Into<String>) -> Json {
        Json::String(s.into())
    }

    /// Creates an empty object.
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    /// Returns the member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns the member `key` of an object mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(m) => m.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets member `key` in place when it exists, else appends it. No-op on non-objects.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Object(m) = self {
            match m.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => m.push((key.to_string(), value)),
            }
        }
    }

    /// Removes member `key` and returns it.
    pub fn remove(&mut self, key: &str) -> Option<Json> {
        if let Json::Object(m) = self {
            let idx = m.iter().position(|(k, _)| k == key)?;
            return Some(m.remove(idx).1);
        }
        None
    }

    /// The string content of a string value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The elements of an array, mutably.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Array(a) => Some(a),
            _ => None,
        }
    }

    /// True for an object.
    pub fn is_object(&self) -> bool {
        matches!(self, Json::Object(_))
    }

    /// True for an object or array with no members.
    pub fn is_empty_container(&self) -> bool {
        match self {
            Json::Object(m) => m.is_empty(),
            Json::Array(a) => a.is_empty(),
            _ => false,
        }
    }

    /// Serialises on one line with no insignificant whitespace.
    pub fn to_compact(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, None, 0);
        out
    }

    /// Serialises with `indent` per level and a trailing newline.
    pub fn to_pretty(&self, indent: &str) -> String {
        let mut out = String::new();
        self.write(&mut out, Some(indent), 0);
        out.push('\n');
        out
    }

    fn write(&self, out: &mut String, indent: Option<&str>, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(n) => out.push_str(n),
            Json::String(s) => write_string(out, s),
            Json::Array(a) if a.is_empty() => out.push_str("[]"),
            Json::Object(m) if m.is_empty() => out.push_str("{}"),
            Json::Array(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, indent, depth + 1);
                    v.write(out, indent, depth + 1);
                }
                newline(out, indent, depth);
                out.push(']');
            }
            Json::Object(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    newline(out, indent, depth + 1);
                    write_string(out, k);
                    out.push(':');
                    if indent.is_some() {
                        out.push(' ');
                    }
                    v.write(out, indent, depth + 1);
                }
                newline(out, indent, depth);
                out.push('}');
            }
        }
    }
}

fn newline(out: &mut String, indent: Option<&str>, depth: usize) {
    if let Some(unit) = indent {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(unit);
        }
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Detects the indentation unit of a pretty-printed JSON file, defaulting to two spaces.
pub fn detect_indent(text: &str) -> String {
    for line in text.lines().skip(1) {
        let lead: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !lead.is_empty() && lead.len() < line.len() {
            if lead.starts_with('\t') {
                return "\t".to_string();
            }
            return lead;
        }
    }
    "  ".to_string()
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> Error {
        let line = self.src[..self.pos.min(self.src.len())]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1;
        Error::config(format!("invalid JSON at line {line}: {msg}"))
    }

    fn skip_ws(&mut self) {
        while matches!(self.src.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> Result<()> {
        if self.src.get(self.pos) == Some(&byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", byte as char)))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        match self.src.get(self.pos) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json> {
        self.eat(b'{')?;
        let mut members = Vec::new();
        self.skip_ws();
        if self.src.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.eat(b':')?;
            self.skip_ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.skip_ws();
            match self.src.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.err("expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json> {
        self.eat(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.src.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.src.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.err("expected `,` or `]`")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32> {
        let slice = self
            .src
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.err("truncated \\u escape"))?;
        let s = std::str::from_utf8(slice).map_err(|_| self.err("invalid \\u escape"))?;
        let n = u32::from_str_radix(s, 16).map_err(|_| self.err("invalid \\u escape"))?;
        self.pos += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(b) = self.src.get(self.pos) {
                if *b == b'"' || *b == b'\\' || *b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            let chunk = std::str::from_utf8(&self.src[start..self.pos])
                .map_err(|_| self.err("invalid UTF-8"))?;
            out.push_str(chunk);
            match self.src.get(self.pos) {
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = *self
                        .src
                        .get(self.pos)
                        .ok_or_else(|| self.err("truncated escape"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.src.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(self.err("unpaired surrogate"));
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            out.push(
                                char::from_u32(code)
                                    .ok_or_else(|| self.err("invalid code point"))?,
                            );
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                Some(_) => return Err(self.err("control character in string")),
                None => return Err(self.err("unterminated string")),
            }
        }
    }

    fn number(&mut self) -> Result<Json> {
        let start = self.pos;
        if self.src.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.src.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.err("invalid number")),
        }
        if self.src.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if !matches!(self.src.get(self.pos), Some(b'0'..=b'9')) {
                return Err(self.err("invalid number"));
            }
            self.digits();
        }
        if matches!(self.src.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.src.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.src.get(self.pos), Some(b'0'..=b'9')) {
                return Err(self.err("invalid number"));
            }
            self.digits();
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| self.err("invalid number"))?;
        Ok(Json::Number(text.to_string()))
    }

    fn digits(&mut self) {
        while matches!(self.src.get(self.pos), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_order_and_numbers() {
        let src = r#"{"z":1,"a":{"y":[1.50,2e3,-0],"b":null},"m":"héllo \"q\" \n","t":true}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(
            v.to_compact(),
            r#"{"z":1,"a":{"y":[1.50,2e3,-0],"b":null},"m":"héllo \"q\" \n","t":true}"#
        );
        let keys: Vec<_> = match &v {
            Json::Object(m) => m.iter().map(|(k, _)| k.as_str()).collect(),
            _ => vec![],
        };
        assert_eq!(keys, ["z", "a", "m", "t"]);
    }

    #[test]
    fn pretty_output_matches_two_space_convention() {
        let v = Json::parse(r#"{"a":[],"b":{},"c":[1,{"d":2}]}"#).unwrap();
        let expected = "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    {\n      \"d\": 2\n    }\n  ]\n}\n";
        assert_eq!(v.to_pretty("  "), expected);
    }

    #[test]
    fn surrogate_pairs_decode() {
        let v = Json::parse(r#""😀""#).unwrap();
        assert_eq!(v, Json::String("😀".into()));
        assert!(Json::parse(r#""\ud83d""#).is_err());
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "",
            "{",
            "{\"a\":}",
            "[1,]",
            "{\"a\":1,}",
            "01",
            "{\"a\" 1}",
            "[1] x",
            "\"a\nb\"",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn indentation_detection() {
        assert_eq!(detect_indent("{\n    \"a\": 1\n}"), "    ");
        assert_eq!(detect_indent("{\n\t\"a\": 1\n}"), "\t");
        assert_eq!(detect_indent("{}"), "  ");
    }

    #[test]
    fn set_replaces_in_place_and_appends() {
        let mut v = Json::parse(r#"{"a":1,"b":2}"#).unwrap();
        v.set("a", Json::str("x"));
        v.set("c", Json::Bool(true));
        assert_eq!(v.to_compact(), r#"{"a":"x","b":2,"c":true}"#);
        assert_eq!(v.remove("b"), Some(Json::Number("2".into())));
        assert_eq!(v.to_compact(), r#"{"a":"x","c":true}"#);
    }
}
