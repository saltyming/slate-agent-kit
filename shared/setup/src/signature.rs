//! Ownership signatures at the top of Markdown files.
//!
//! Owns the rules that tell a kit-managed file from a user-owned one: a
//! kit-managed file carries `<!-- slate-agent-kit:common -->` or `<!-- <kit> -->`,
//! a user-owned file starts with `<!-- <kit>-custom:`. It does not touch the
//! file system; callers pass file text.
//!
//! Main entry points: [`classify`], [`classify_skill`] and [`custom_signature`].

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

/// Classifies a Markdown file by its first line.
pub fn classify(kit: &str, text: &str) -> Owner {
    let first = text
        .trim_start_matches('\u{feff}')
        .lines()
        .next()
        .unwrap_or("");
    classify_line(kit, first).unwrap_or(Owner::Unrecognized)
}

/// Classifies a `SKILL.md`, whose signature may follow YAML front matter.
pub fn classify_skill(kit: &str, text: &str) -> Owner {
    text.trim_start_matches('\u{feff}')
        .lines()
        .take(12)
        .find_map(|l| classify_line(kit, l))
        .unwrap_or(Owner::Unrecognized)
}

/// Returns `text` with a user-owned signature prepended unless it already has one.
pub fn ensure_custom_signature(kit: &str, name: &str, text: &str) -> String {
    if classify(kit, text) == Owner::User {
        return text.to_string();
    }
    let eol = crate::util::detect_eol(text);
    format!("{}{eol}{text}", custom_signature(kit, name))
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
        assert_eq!(classify("k", text), Owner::Unrecognized);
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
    }
}
