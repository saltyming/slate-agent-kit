//! Ownership signatures at the top of Markdown files.
//!
//! Owns the rules that tell a kit-managed file from a user-owned one: a
//! kit-managed file carries `<!-- slate-agent-kit:common -->` or `<!-- <kit> -->`,
//! a user-owned file starts with `<!-- <kit>-custom:`. The signature is the
//! first line, or the first line after a closed leading YAML frontmatter block
//! (a `---` line, lines, a `---` line), which agy requires on every rule file.
//! Also owns adding the user-owned signature to a custom rule, with or without
//! that block. It does not touch the file system; callers pass file text.
//!
//! Main entry points: [`classify`], [`classify_skill`], [`signature_line`],
//! [`frontmatter_end`], [`ensure_custom_signature`],
//! [`ensure_custom_signature_with_frontmatter`] and [`custom_signature`].

/// Who owns a file according to its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The kit wrote it and install may overwrite it.
    Managed,
    /// The user owns it; install never overwrites it.
    User,
    /// No recognised signature.
    Unrecognized,
}

/// The shared signature line of files rendered by slate.
pub const COMMON_SIGNATURE: &str = "<!-- slate-agent-kit:common -->";

/// The kit's own managed-file signature line.
pub fn kit_signature(kit: &str) -> String {
    format!("<!-- {kit} -->")
}

/// The signature line of a user-owned file, `<!-- <kit>-custom:<name> -->`.
pub fn custom_signature(kit: &str, name: &str) -> String {
    format!("<!-- {kit}-custom:{name} -->")
}

fn classify_line(kit: &str, line: &str) -> Option<Owner> {
    let line = line.trim();
    if line == COMMON_SIGNATURE || line == kit_signature(kit) {
        Some(Owner::Managed)
    } else if line.starts_with(&format!("<!-- {kit}-custom:")) {
        Some(Owner::User)
    } else {
        None
    }
}

fn first_line(text: &str) -> &str {
    text.trim_start_matches('\u{feff}')
        .lines()
        .next()
        .unwrap_or("")
}

/// The byte offset just past a closed YAML frontmatter block at the start of
/// `text`, or `None` when the text does not start with one.
///
/// The block opens with a `---` line as the first line (after a byte-order
/// mark, if any) and closes with the next `---` line; trailing whitespace and a
/// `\r` before the line feed are ignored. An opening line without a closing one
/// is not a block.
pub fn frontmatter_end(text: &str) -> Option<usize> {
    let start = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut pos = start;
    let mut lines = text[start..].split_inclusive('\n');
    let open = lines.next()?;
    if open.trim_end() != "---" || !open.ends_with('\n') {
        return None;
    }
    pos += open.len();
    for line in lines {
        pos += line.len();
        if line.trim_end() == "---" {
            return Some(pos);
        }
    }
    None
}

/// The line that carries a file's signature: the first line after a closed
/// leading frontmatter block, else the first line.
pub fn signature_line(text: &str) -> &str {
    match frontmatter_end(text) {
        Some(end) => text[end..].lines().next().unwrap_or(""),
        None => first_line(text),
    }
}

/// Classifies a Markdown file by its signature line (see [`signature_line`]).
pub fn classify(kit: &str, text: &str) -> Owner {
    classify_line(kit, signature_line(text)).unwrap_or(Owner::Unrecognized)
}

/// Classifies a `SKILL.md`, whose signature may follow YAML front matter.
pub fn classify_skill(kit: &str, text: &str) -> Owner {
    text.trim_start_matches('\u{feff}')
        .lines()
        .take(12)
        .find_map(|l| classify_line(kit, l))
        .unwrap_or(Owner::Unrecognized)
}

/// Returns `text` with a user-owned signature prepended unless its first line already is one.
///
/// For the harnesses that read rule files as they are; only the first line is
/// considered, so a frontmatter block stays below the signature.
pub fn ensure_custom_signature(kit: &str, name: &str, text: &str) -> String {
    if classify_line(kit, first_line(text)) == Some(Owner::User) {
        return text.to_string();
    }
    let eol = crate::util::detect_eol(text);
    format!("{}{eol}{text}", custom_signature(kit, name))
}

/// Quotes `s` as a YAML double-quoted scalar.
fn yaml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The always-on frontmatter block a rule file gets: `---`, `trigger: always_on`,
/// `description: "<title>: <stem>"`, `---`, each line ended with `eol`.
pub fn always_on_block(title: &str, stem: &str, eol: &str) -> String {
    format!(
        "---{eol}trigger: always_on{eol}description: {}{eol}---{eol}",
        yaml_quote(&format!("{title}: {stem}"))
    )
}

/// Returns `text` as a rule file for a harness that requires a frontmatter
/// block: the block first, then the user-owned signature, then the rest.
///
/// A file that already starts with a closed block keeps it, and the signature
/// goes right after it unless it is already there. A file without a block gets
/// the always-on block named after `stem` (see [`always_on_block`]); a
/// signature already on its first line is kept below the block, and a
/// byte-order mark is dropped so that the block opens the file.
pub fn ensure_custom_signature_with_frontmatter(
    kit: &str,
    name: &str,
    title: &str,
    stem: &str,
    text: &str,
) -> String {
    let eol = crate::util::detect_eol(text);
    let sig = custom_signature(kit, name);
    if let Some(end) = frontmatter_end(text) {
        let (block, rest) = text.split_at(end);
        if classify_line(kit, rest.lines().next().unwrap_or("")) == Some(Owner::User) {
            return text.to_string();
        }
        let sep = if block.ends_with('\n') { "" } else { eol };
        return format!("{block}{sep}{sig}{eol}{rest}");
    }
    let body = text.trim_start_matches('\u{feff}');
    let block = always_on_block(title, stem, eol);
    if classify_line(kit, first_line(body)) == Some(Owner::User) {
        format!("{block}{body}")
    } else {
        format!("{block}{sig}{eol}{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_first_line() {
        assert_eq!(
            classify("k", "<!-- slate-agent-kit:common -->\nx"),
            Owner::Managed
        );
        assert_eq!(classify("k", "<!-- k -->\r\nx"), Owner::Managed);
        assert_eq!(classify("k", "<!-- k-custom:git-prefs -->\nx"), Owner::User);
        assert_eq!(classify("k", "# Title\n"), Owner::Unrecognized);
        assert_eq!(classify("k", "<!-- other -->\n"), Owner::Unrecognized);
        assert_eq!(classify("k", ""), Owner::Unrecognized);
    }

    #[test]
    fn skill_signature_may_follow_front_matter() {
        let text = "---\nname: x\ndescription: y\n---\n<!-- slate-agent-kit:common -->\nbody";
        assert_eq!(classify_skill("k", text), Owner::Managed);
        // A signature right after a closed block counts for every file (RFC-0023).
        assert_eq!(classify("k", text), Owner::Managed);
        assert_eq!(
            classify_skill("k", "---\nname: x\n---\nbody"),
            Owner::Unrecognized
        );
    }

    #[test]
    fn custom_signature_added_once() {
        let s = ensure_custom_signature("k", "user", "# Rule\n");
        assert_eq!(s, "<!-- k-custom:user -->\n# Rule\n");
        assert_eq!(ensure_custom_signature("k", "user", &s), s);
        // The harnesses without frontmatter keep their first-line rule.
        let fm = "---\na: b\n---\n<!-- k-custom:user -->\nx\n";
        assert_eq!(
            ensure_custom_signature("k", "user", fm),
            format!("<!-- k-custom:user -->\n{fm}")
        );
    }

    const AGY: &str = "agy-agent-kit";

    #[test]
    fn signature_after_a_closed_block_classifies_the_file() {
        let kit_rule = "---\ntrigger: always_on\ndescription: \"Antigravity Agent: Task execution\"\n---\n<!-- slate-agent-kit:common -->\n# Rule\n";
        assert_eq!(classify(AGY, kit_rule), Owner::Managed);
        let own =
            "---\ntrigger: always_on\ndescription: \"x\"\n---\n<!-- agy-agent-kit -->\nbody\n";
        assert_eq!(classify(AGY, own), Owner::Managed);
        let prefs = "---\ntrigger: always_on\ndescription: \"x\"\n---\n<!-- agy-agent-kit-custom:git-prefs -->\n# Git\n";
        assert_eq!(classify(AGY, prefs), Owner::User);
        assert_eq!(
            signature_line(prefs),
            "<!-- agy-agent-kit-custom:git-prefs -->"
        );
        // Another kit's signature after the block is foreign.
        let other = "---\na: b\n---\n<!-- codex-agent-kit-custom:x -->\n";
        assert_eq!(classify(AGY, other), Owner::Unrecognized);
    }

    #[test]
    fn bom_and_crlf_are_accepted_around_the_block() {
        let crlf = "---\r\ntrigger: always_on\r\ndescription: \"x\"\r\n---\r\n<!-- agy-agent-kit-custom:consult-prefs -->\r\nrest\r\n";
        assert_eq!(classify(AGY, crlf), Owner::User);
        let bom = format!("\u{feff}{crlf}");
        assert_eq!(classify(AGY, &bom), Owner::User);
        assert_eq!(frontmatter_end(&bom), Some(bom.find("<!--").unwrap()));
        let bom_kit = "\u{feff}---\na: b\n---\n<!-- slate-agent-kit:common -->\n";
        assert_eq!(classify(AGY, bom_kit), Owner::Managed);
        // Without a block, the BOM and CRLF rules are as before.
        assert_eq!(
            classify(AGY, "\u{feff}<!-- agy-agent-kit -->\r\nx"),
            Owner::Managed
        );
    }

    #[test]
    fn malformed_frontmatter_is_not_a_block() {
        // Never closed: the first line is `---`, which is no signature.
        let open = "---\ntrigger: always_on\n<!-- agy-agent-kit-custom:x -->\n";
        assert_eq!(frontmatter_end(open), None);
        assert_eq!(classify(AGY, open), Owner::Unrecognized);
        // The opening line must be the first line.
        let late = "\n---\na: b\n---\n<!-- slate-agent-kit:common -->\n";
        assert_eq!(classify(AGY, late), Owner::Unrecognized);
        // A lone `---` line with nothing after it.
        assert_eq!(frontmatter_end("---"), None);
        assert_eq!(classify(AGY, "---\n"), Owner::Unrecognized);
        // A closed block followed by a blank line, then the signature: the
        // signature must be the first line after the block.
        let gap = "---\na: b\n---\n\n<!-- slate-agent-kit:common -->\n";
        assert_eq!(classify(AGY, gap), Owner::Unrecognized);
        // A block closed at the very end of the file has nothing after it.
        assert_eq!(classify(AGY, "---\na: b\n---"), Owner::Unrecognized);
    }

    #[test]
    fn custom_rules_get_the_block_first_and_the_signature_after_it() {
        let title = "Antigravity Agent";
        let plain =
            ensure_custom_signature_with_frontmatter(AGY, "user", title, "style", "# Style\n");
        assert_eq!(
            plain,
            "---\ntrigger: always_on\ndescription: \"Antigravity Agent: style\"\n---\n<!-- agy-agent-kit-custom:user -->\n# Style\n"
        );
        assert_eq!(classify(AGY, &plain), Owner::User);
        // Idempotent.
        assert_eq!(
            ensure_custom_signature_with_frontmatter(AGY, "user", title, "style", &plain),
            plain
        );
        // An existing block is kept and the signature goes right after it.
        let fm = "---\ntrigger: glob\nglobs: \"*.rs\"\n---\n# Rust\n";
        assert_eq!(
            ensure_custom_signature_with_frontmatter(AGY, "user", title, "rust", fm),
            "---\ntrigger: glob\nglobs: \"*.rs\"\n---\n<!-- agy-agent-kit-custom:user -->\n# Rust\n"
        );
        // A block closed at the end of the file without a newline.
        assert_eq!(
            ensure_custom_signature_with_frontmatter(AGY, "user", title, "x", "---\na: b\n---"),
            "---\na: b\n---\n<!-- agy-agent-kit-custom:user -->\n"
        );
        // A signature already on the first line stays, below the block.
        let signed = "<!-- agy-agent-kit-custom:mine -->\n# Signed\n";
        assert_eq!(
            ensure_custom_signature_with_frontmatter(AGY, "user", title, "signed", signed),
            format!(
                "---\ntrigger: always_on\ndescription: \"Antigravity Agent: signed\"\n---\n{signed}"
            )
        );
        // CRLF files keep CRLF; a BOM is dropped so the block opens the file.
        let crlf = ensure_custom_signature_with_frontmatter(
            AGY,
            "user",
            title,
            "w",
            "\u{feff}# W\r\nx\r\n",
        );
        assert_eq!(
            crlf,
            "---\r\ntrigger: always_on\r\ndescription: \"Antigravity Agent: w\"\r\n---\r\n<!-- agy-agent-kit-custom:user -->\r\n# W\r\nx\r\n"
        );
        // Quotes and backslashes in the name are escaped.
        assert!(
            always_on_block(title, "a\"b\\c", "\n")
                .contains("description: \"Antigravity Agent: a\\\"b\\\\c\"\n")
        );
    }
}
