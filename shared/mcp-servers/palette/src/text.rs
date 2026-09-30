//! Byte-exact line model: a file is a list of lines, each with its own line ending.
//!
//! Owns decoding UTF-8 bytes into lines, re-encoding them unchanged, and picking
//! the line ending that newly inserted lines should use. Does not know RST.
//! Entry points: [`Source::from_bytes`], [`Source::to_bytes`], [`Source::native_eol`].

/// The line ending that terminated one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    /// The last line of a file that does not end with a newline.
    None,
    /// `\n`.
    Lf,
    /// `\r\n`.
    CrLf,
}

impl Eol {
    /// The literal bytes of this line ending.
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::None => "",
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
        }
    }
}

/// One line of text without its line ending, plus the ending it had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The text of the line.
    pub text: String,
    /// The line ending that followed the text.
    pub eol: Eol,
}

/// A whole file as lines. Re-encoding an unmodified `Source` reproduces the
/// original bytes exactly, including mixed line endings and a missing final newline.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Source {
    bom: bool,
    /// The lines of the file, in order.
    pub lines: Vec<Line>,
}

impl Source {
    /// Decodes `bytes` as UTF-8. On invalid UTF-8 returns the 1-based line that holds
    /// the first invalid byte.
    pub fn from_bytes(bytes: &[u8]) -> Result<Source, usize> {
        match std::str::from_utf8(bytes) {
            Ok(s) => Ok(Source::from_text(s)),
            Err(e) => {
                let upto = &bytes[..e.valid_up_to()];
                Err(1 + upto.iter().filter(|b| **b == b'\n').count())
            }
        }
    }

    /// Splits `text` into lines. A leading byte order mark is remembered and restored
    /// by [`Source::to_bytes`].
    pub fn from_text(text: &str) -> Source {
        let (bom, text) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let mut lines = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            match rest.find('\n') {
                Some(i) => {
                    let (raw, tail) = rest.split_at(i);
                    let (t, eol) = match raw.strip_suffix('\r') {
                        Some(t) => (t, Eol::CrLf),
                        None => (raw, Eol::Lf),
                    };
                    lines.push(Line {
                        text: t.to_string(),
                        eol,
                    });
                    rest = &tail[1..];
                }
                None => {
                    lines.push(Line {
                        text: rest.to_string(),
                        eol: Eol::None,
                    });
                    rest = "";
                }
            }
        }
        Source { bom, lines }
    }

    /// Builds a `Source` from line texts, all ending with `eol` (the last one too).
    pub fn from_lines<S: AsRef<str>>(texts: &[S], eol: Eol) -> Source {
        Source {
            bom: false,
            lines: texts
                .iter()
                .map(|t| Line {
                    text: t.as_ref().to_string(),
                    eol,
                })
                .collect(),
        }
    }

    /// The exact text of the file.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if self.bom {
            out.push('\u{feff}');
        }
        for l in &self.lines {
            out.push_str(&l.text);
            out.push_str(l.eol.as_str());
        }
        out
    }

    /// The exact bytes of the file.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_text().into_bytes()
    }

    /// The file's text with every line ending normalized to `\n`; used to compare
    /// generated files independently of a checkout's line-ending conversion.
    pub fn to_text_lf(&self) -> String {
        let mut out = String::new();
        for l in &self.lines {
            out.push_str(&l.text);
            if l.eol != Eol::None {
                out.push('\n');
            }
        }
        out
    }

    /// The line ending new lines should use: the most common terminator in the file,
    /// `\n` when the file has none.
    pub fn native_eol(&self) -> Eol {
        let crlf = self.lines.iter().filter(|l| l.eol == Eol::CrLf).count();
        let lf = self.lines.iter().filter(|l| l.eol == Eol::Lf).count();
        if crlf > lf { Eol::CrLf } else { Eol::Lf }
    }

    /// Number of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Whether the file has no lines.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Text of line `i` (0-based).
    pub fn text(&self, i: usize) -> &str {
        &self.lines[i].text
    }

    /// Replaces lines `range` with `new` (text only); new lines use the file's native
    /// ending. If the replaced range ended the file without a newline and nothing
    /// follows it, the final new line also has no newline.
    pub fn splice(&mut self, range: std::ops::Range<usize>, new: &[String]) {
        let native = self.native_eol();
        let at_end = range.end >= self.lines.len();
        let replaces_tail = at_end && range.end > range.start;
        let last_eol = if replaces_tail {
            self.lines[range.end - 1].eol
        } else {
            native
        };
        // Inserting after an unterminated last line terminates that line first.
        if range.start == self.lines.len()
            && let Some(prev) = self.lines.last_mut()
            && prev.eol == Eol::None
        {
            prev.eol = native;
        }
        let count = new.len();
        let replacement = new.iter().enumerate().map(|(i, t)| Line {
            text: t.clone(),
            eol: if i + 1 == count && replaces_tail {
                last_eol
            } else {
                native
            },
        });
        self.lines.splice(range, replacement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_mixed_endings_and_missing_final_newline() {
        let raw = "a\r\nb\nc\r\nd";
        let s = Source::from_text(raw);
        assert_eq!(s.lines.len(), 4);
        assert_eq!(s.lines[3].eol, Eol::None);
        assert_eq!(s.to_text(), raw);
    }

    #[test]
    fn bom_is_kept() {
        let raw = "\u{feff}a\nb\n";
        assert_eq!(Source::from_text(raw).to_text(), raw);
    }

    #[test]
    fn invalid_utf8_reports_line() {
        let bytes = b"ok\nok\n\xff\n";
        assert_eq!(Source::from_bytes(bytes), Err(3));
    }

    #[test]
    fn splice_uses_native_ending_and_terminates_previous_last_line() {
        let mut s = Source::from_text("a\r\nb\r\nc");
        s.splice(3..3, &["d".to_string()]);
        assert_eq!(s.to_text(), "a\r\nb\r\nc\r\nd\r\n");
    }
}
