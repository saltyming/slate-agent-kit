//! Line-preserving access to the `## Heading` / `**value**` format of prefs files.
//!
//! Owns reading and replacing value lines and copying whole sections, keeping
//! every other line, its text and its line ending untouched. It does not know
//! which headings are settings; `schema` supplies them.
//!
//! Main entry points: [`PrefsDoc::parse`], [`PrefsDoc::get`], [`PrefsDoc::set`],
//! [`PrefsDoc::headings`], [`PrefsDoc::section`] and [`PrefsDoc::replace_section`].

use crate::util::detect_eol;

/// One line of text and the line ending that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The line without its terminator.
    pub text: String,
    /// `\n`, `\r\n`, or empty for an unterminated last line.
    pub eol: String,
}

/// A prefs file split into lines.
#[derive(Debug, Clone)]
pub struct PrefsDoc {
    lines: Vec<Line>,
    eol: &'static str,
}

fn is_heading(text: &str) -> bool {
    text.starts_with("## ")
}

fn value_of(text: &str) -> Option<&str> {
    let t = text.trim();
    (t.len() >= 4 && t.starts_with("**") && t.ends_with("**")).then(|| &t[2..t.len() - 2])
}

impl PrefsDoc {
    /// Splits `text` into lines, remembering each line's own ending.
    pub fn parse(text: &str) -> PrefsDoc {
        let mut lines = Vec::new();
        for piece in text.split_inclusive('\n') {
            let (body, eol) = if let Some(b) = piece.strip_suffix("\r\n") {
                (b, "\r\n")
            } else if let Some(b) = piece.strip_suffix('\n') {
                (b, "\n")
            } else {
                (piece, "")
            };
            lines.push(Line {
                text: body.to_string(),
                eol: eol.to_string(),
            });
        }
        PrefsDoc {
            lines,
            eol: detect_eol(text),
        }
    }

    /// The dominant line ending of the source text.
    pub fn eol(&self) -> &'static str {
        self.eol
    }

    /// The text with every line ending as it was.
    pub fn render(&self) -> String {
        self.lines
            .iter()
            .map(|l| format!("{}{}", l.text, l.eol))
            .collect()
    }

    fn heading_index(&self, heading: &str) -> Option<usize> {
        let want = format!("## {heading}");
        self.lines.iter().position(|l| l.text.trim_end() == want)
    }

    fn section_end(&self, start: usize) -> usize {
        self.lines[start + 1..]
            .iter()
            .position(|l| is_heading(&l.text))
            .map_or(self.lines.len(), |p| start + 1 + p)
    }

    fn value_index(&self, heading: &str) -> Option<usize> {
        let start = self.heading_index(heading)?;
        let end = self.section_end(start);
        (start + 1..end).find(|&i| value_of(&self.lines[i].text).is_some())
    }

    /// The text of every `## <heading>` line, in file order, without the `## ` prefix.
    pub fn headings(&self) -> impl Iterator<Item = &str> + '_ {
        self.lines
            .iter()
            .filter_map(|l| l.text.trim_end().strip_prefix("## "))
    }

    /// True when the file has a `## <heading>` line.
    pub fn has_heading(&self, heading: &str) -> bool {
        self.heading_index(heading).is_some()
    }

    /// The value line under `heading`, without the surrounding `**`.
    pub fn get(&self, heading: &str) -> Option<String> {
        let i = self.value_index(heading)?;
        value_of(&self.lines[i].text).map(str::to_string)
    }

    /// Replaces the value line under `heading`. Returns whether the text changed.
    pub fn set(&mut self, heading: &str, value: &str) -> Result<bool, String> {
        let i = self
            .value_index(heading)
            .ok_or_else(|| format!("no value line under `## {heading}`"))?;
        let new = format!("**{value}**");
        let line = &mut self.lines[i];
        // Keep leading indentation, if any, the same way the value line was written.
        let lead: String = line
            .text
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        let new = format!("{lead}{new}");
        if line.text == new {
            return Ok(false);
        }
        line.text = new;
        Ok(true)
    }

    /// The first `**value**` span anywhere in the section under `heading`.
    ///
    /// Older prefs files put the value inside a sentence, such as
    /// `Default backend when ... advisor: **codex**`, instead of on a line of its own.
    pub fn inline_value(&self, heading: &str) -> Option<String> {
        let start = self.heading_index(heading)?;
        let end = self.section_end(start);
        self.lines[start + 1..end].iter().find_map(|l| {
            let open = l.text.find("**")?;
            let after = &l.text[open + 2..];
            let close = after.find("**")?;
            Some(after[..close].to_string())
        })
    }

    fn list_line_rest(&self, label: &str) -> Option<&str> {
        let want = format!("- {label}:");
        let line = self
            .lines
            .iter()
            .find(|l| l.text.trim_start().starts_with(&want))?;
        Some(&line.text.trim_start()[want.len()..])
    }

    /// The label of every list item `- <label>: ...`, in file order.
    pub fn list_labels(&self) -> impl Iterator<Item = &str> + '_ {
        self.lines.iter().filter_map(|l| {
            let (label, _) = l.text.trim_start().strip_prefix("- ")?.split_once(':')?;
            Some(label)
        })
    }

    /// True when a list item `- <label>: ...` exists, whether or not its value can be read.
    pub fn has_list_line(&self, label: &str) -> bool {
        self.list_line_rest(label).is_some()
    }

    /// Reads the value of a list item such as `- codex default model: **X**`.
    ///
    /// `label` is the text between `- ` and the colon. The value is either bold
    /// (`**X**`, possibly empty as `****`) or bare text after the colon that ends
    /// at the first run of two or more spaces followed by `(` (a trailing hint)
    /// or at the end of the line. Returns `None` when the line is missing or a
    /// bold value is not closed; [`PrefsDoc::has_list_line`] tells the two apart.
    pub fn list_value(&self, label: &str) -> Option<String> {
        parse_list_value(self.list_line_rest(label)?)
    }

    /// The lines under `heading`, up to the next heading, including trailing blank lines.
    pub fn section(&self, heading: &str) -> Option<Vec<Line>> {
        let start = self.heading_index(heading)?;
        let end = self.section_end(start);
        Some(self.lines[start + 1..end].to_vec())
    }

    /// Replaces the body of `heading` with `body`, or appends the section when it is missing.
    ///
    /// Every line takes the document's line ending. When another heading follows
    /// and `body` does not end with a blank line, one blank line is added before it.
    pub fn replace_section(&mut self, heading: &str, body: &[Line]) {
        let eol = self.eol.to_string();
        let mut new_body: Vec<Line> = body
            .iter()
            .map(|l| Line {
                text: l.text.clone(),
                eol: eol.clone(),
            })
            .collect();
        match self.heading_index(heading) {
            Some(start) => {
                let end = self.section_end(start);
                let tail_is_eof = end == self.lines.len();
                if tail_is_eof {
                    self.ensure_terminated();
                } else if new_body.last().is_none_or(|l| !l.text.trim().is_empty()) {
                    // Keep one blank line before the heading that follows.
                    new_body.push(Line {
                        text: String::new(),
                        eol: eol.clone(),
                    });
                }
                self.lines.splice(start + 1..end, new_body);
            }
            None => {
                self.ensure_terminated();
                if self.lines.last().is_some_and(|l| !l.text.is_empty()) {
                    self.lines.push(Line {
                        text: String::new(),
                        eol: eol.clone(),
                    });
                }
                self.lines.push(Line {
                    text: format!("## {heading}"),
                    eol: eol.clone(),
                });
                self.lines.append(&mut new_body);
            }
        }
        self.ensure_terminated();
    }

    fn ensure_terminated(&mut self) {
        let eol = self.eol;
        if let Some(last) = self.lines.last_mut()
            && last.eol.is_empty()
        {
            last.eol = eol.to_string();
        }
    }

    /// Appends every section of `other` whose heading `keep` accepts, in `other`'s order.
    ///
    /// Each section is copied byte for byte, heading line included, with its own
    /// line endings; only a separating blank line and a terminator on the line
    /// before it are added when the document needs them. Returns the appended headings.
    pub fn append_sections_from(
        &mut self,
        other: &PrefsDoc,
        keep: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let mut appended = Vec::new();
        for start in (0..other.lines.len()).filter(|&i| is_heading(&other.lines[i].text)) {
            let heading = other.lines[start]
                .text
                .strip_prefix("## ")
                .unwrap_or_default()
                .trim_end()
                .to_string();
            if !keep(&heading) {
                continue;
            }
            self.ensure_terminated();
            if self.lines.last().is_some_and(|l| !l.text.is_empty()) {
                self.lines.push(Line {
                    text: String::new(),
                    eol: self.eol.to_string(),
                });
            }
            let end = other.section_end(start);
            self.lines.extend_from_slice(&other.lines[start..end]);
            appended.push(heading);
        }
        appended
    }

    /// Converts every line ending to `eol`.
    pub fn set_eol(&mut self, eol: &'static str) {
        for l in &mut self.lines {
            if !l.eol.is_empty() {
                l.eol = eol.to_string();
            }
        }
        self.eol = eol;
    }

    /// Copies the section `heading` from `other`, with its body verbatim.
    pub fn copy_section_from(&mut self, other: &PrefsDoc, heading: &str) -> bool {
        match other.section(heading) {
            Some(body) => {
                self.replace_section(heading, &body);
                true
            }
            None => false,
        }
    }
}

fn parse_list_value(rest: &str) -> Option<String> {
    let text = rest.trim_start();
    if let Some(after) = text.strip_prefix("**") {
        let close = after.find("**")?;
        return Some(after[..close].to_string());
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            let start = i;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            if i - start >= 2 && bytes.get(i) == Some(&b'(') {
                return Some(text[..start].trim_end().to_string());
            }
        } else {
            i += 1;
        }
    }
    Some(text.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "<!-- k-custom:git-prefs -->\n# Git\n\n## Commit signing\n\n**unset**\n\nValues: `default` | `no-gpg-sign`\n\n## Branch naming\n\n**descriptive**\n\n## Notes\n\nfree text\n";

    #[test]
    fn reads_and_replaces_only_the_value_line() {
        let mut d = PrefsDoc::parse(DOC);
        assert_eq!(d.get("Commit signing").as_deref(), Some("unset"));
        assert_eq!(d.get("Branch naming").as_deref(), Some("descriptive"));
        assert_eq!(d.get("Notes"), None);
        assert!(d.set("Commit signing", "no-gpg-sign").unwrap());
        assert!(!d.set("Commit signing", "no-gpg-sign").unwrap());
        assert_eq!(d.render(), DOC.replace("**unset**", "**no-gpg-sign**"));
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let crlf = DOC.replace('\n', "\r\n");
        let mut d = PrefsDoc::parse(&crlf);
        assert_eq!(d.eol(), "\r\n");
        d.set("Branch naming", "feat/x").unwrap();
        assert_eq!(d.render(), crlf.replace("**descriptive**", "**feat/x**"));
        assert!(d.render().matches('\n').count() == d.render().matches("\r\n").count());
    }

    #[test]
    fn blank_value_is_written_and_read_back() {
        let mut d = PrefsDoc::parse("## A\n\n**x**\n");
        d.set("A", "").unwrap();
        assert_eq!(d.render(), "## A\n\n****\n");
        assert_eq!(d.get("A").as_deref(), Some(""));
    }

    #[test]
    fn a_value_line_in_the_next_section_is_not_picked_up() {
        let d = PrefsDoc::parse("## A\n\ntext only\n\n## B\n\n**b**\n");
        assert_eq!(d.get("A"), None);
        assert_eq!(d.get("B").as_deref(), Some("b"));
        let mut d = d;
        assert!(d.set("A", "x").is_err());
    }

    #[test]
    fn values_containing_asterisks_round_trip() {
        let mut d = PrefsDoc::parse("## A\n\n**x**\n");
        d.set("A", "use **bold** here").unwrap();
        assert_eq!(d.get("A").as_deref(), Some("use **bold** here"));
    }

    #[test]
    fn inline_values_are_found_inside_sentences() {
        let d = PrefsDoc::parse(
            "## Preferred third-party advisor\n\nDefault backend when X decides to ask: **legacy**\n\nValid values: `none`\n",
        );
        assert_eq!(d.get("Preferred third-party advisor"), None);
        assert_eq!(
            d.inline_value("Preferred third-party advisor").as_deref(),
            Some("legacy")
        );
        assert_eq!(d.inline_value("Absent"), None);
    }

    #[test]
    fn list_values_may_be_bold_or_bare() {
        let d = PrefsDoc::parse(
            "- a fallback: model-b(high)   (comma-separated; blank = none)\n- b fallback: **** (comma-separated)\n- c fallback: x, y(low), z\n- d fallback:\n- e fallback: **open\n- f fallback: two words  (hint)\n",
        );
        assert_eq!(d.list_value("a fallback").as_deref(), Some("model-b(high)"));
        assert_eq!(d.list_value("b fallback").as_deref(), Some(""));
        assert_eq!(d.list_value("c fallback").as_deref(), Some("x, y(low), z"));
        assert_eq!(d.list_value("d fallback").as_deref(), Some(""));
        assert_eq!(d.list_value("f fallback").as_deref(), Some("two words"));
        // An unterminated bold value cannot be read, but the line is there.
        assert_eq!(d.list_value("e fallback"), None);
        assert!(d.has_list_line("e fallback"));
        assert!(!d.has_list_line("g fallback"));
    }

    #[test]
    fn list_values_are_read_from_legacy_lines() {
        let d = PrefsDoc::parse(
            "- codex default model: **gpt-6-astra**\n- codex default model fallback: **gpt-6-sol(high)**   (comma-separated)\n- legacy default model: ****\n- default model: **m**\n- default model fallback: **f**\n",
        );
        assert_eq!(
            d.list_value("codex default model").as_deref(),
            Some("gpt-6-astra")
        );
        assert_eq!(
            d.list_value("codex default model fallback").as_deref(),
            Some("gpt-6-sol(high)")
        );
        assert_eq!(d.list_value("legacy default model").as_deref(), Some(""));
        assert_eq!(d.list_value("default model").as_deref(), Some("m"));
        assert_eq!(d.list_value("default model fallback").as_deref(), Some("f"));
        assert_eq!(d.list_value("claude default model"), None);
    }

    #[test]
    fn sections_are_copied_verbatim_into_another_document() {
        let old = PrefsDoc::parse("## Notes\n\nmy note\n\n## Repository overrides\n\n- /r: x\n");
        let mut new = PrefsDoc::parse(
            "# T\n\n## Level\n\n**suggest**\n\n## Notes\n\ndefault\n\n## Repository overrides\n\nhelp\n",
        );
        assert!(new.copy_section_from(&old, "Notes"));
        assert!(new.copy_section_from(&old, "Repository overrides"));
        assert_eq!(
            new.render(),
            "# T\n\n## Level\n\n**suggest**\n\n## Notes\n\nmy note\n\n## Repository overrides\n\n- /r: x\n"
        );
    }

    #[test]
    fn a_replaced_body_is_separated_from_the_next_heading() {
        for eol in ["\n", "\r\n"] {
            let template = "## Notes\n\ndefault\n\n## Next\n\nn\n".replace('\n', eol);
            for (body, want) in [
                ("## Notes\n\nmine", "## Notes\n\nmine\n\n## Next\n"),
                ("## Notes\n\nmine\n", "## Notes\n\nmine\n\n## Next\n"),
                ("## Notes\n\nmine\n\n", "## Notes\n\nmine\n\n## Next\n"),
                ("## Notes\n", "## Notes\n\n## Next\n"),
            ] {
                let old = PrefsDoc::parse(&body.replace('\n', eol));
                let mut new = PrefsDoc::parse(&template);
                new.copy_section_from(&old, "Notes");
                assert_eq!(
                    new.render(),
                    format!("{want}\nn\n").replace('\n', eol),
                    "{body:?} {eol:?}"
                );
            }
        }
    }

    #[test]
    fn a_missing_section_is_appended() {
        let old = PrefsDoc::parse("## Notes\n\nmy note\n");
        let mut new = PrefsDoc::parse("# T\n\n**x**\n");
        new.copy_section_from(&old, "Notes");
        assert_eq!(new.render(), "# T\n\n**x**\n\n## Notes\n\nmy note\n");
    }

    #[test]
    fn list_labels_are_listed_in_order() {
        let d = PrefsDoc::parse("## A\n\n- a b: **x**\n  - c: y\n- no colon\ntext: z\n");
        assert_eq!(d.list_labels().collect::<Vec<_>>(), vec!["a b", "c"]);
    }

    #[test]
    fn sections_are_appended_verbatim_in_order() {
        let old =
            PrefsDoc::parse("## Skip\n\nx\n\n## Mine\r\n\r\none\r\ntwo\r\n\r\n## Also\n\nlast");
        let mut new = PrefsDoc::parse("# T\n\n## Notes\n\nn");
        let appended = new.append_sections_from(&old, |h| h != "Skip");
        assert_eq!(appended, vec!["Mine", "Also"]);
        // A heading line with no name is kept too, not a panic.
        let mut bare = PrefsDoc::parse("x\n");
        assert_eq!(
            bare.append_sections_from(&PrefsDoc::parse("## \nbody\n"), |_| true),
            vec![""]
        );
        assert_eq!(bare.render(), "x\n\n## \nbody\n");
        assert_eq!(
            new.render(),
            "# T\n\n## Notes\n\nn\n\n## Mine\r\n\r\none\r\ntwo\r\n\r\n## Also\n\nlast"
        );
    }

    #[test]
    fn unterminated_last_line_stays_valid_after_edits() {
        let mut d = PrefsDoc::parse("## A\n\n**x**\n\n## Notes\n\nlast");
        let body = d.section("Notes").unwrap();
        assert_eq!(body.last().unwrap().eol, "");
        let blank = Line {
            text: String::new(),
            eol: "\n".into(),
        };
        d.replace_section(
            "A",
            &[
                blank.clone(),
                Line {
                    text: "**y**".into(),
                    eol: "\n".into(),
                },
                blank,
            ],
        );
        assert_eq!(d.render(), "## A\n\n**y**\n\n## Notes\n\nlast\n");
    }
}
