//! Prefs files whose name changed between releases, and the move from the old name.
//!
//! Owns the table of renamed prefs files (the consultation prefs were named
//! aside before the aside server was retired), the rewrite of the signature
//! line from the old name to the new one, and the move itself. Discovery
//! (`kit`) reads the old file as if it already had the new name, so the wizard
//! and the plan see the user's values; the apply step moves it. It decides
//! nothing about the values inside the file.
//!
//! Main entry points: [`earlier_name`], [`rewrite_signature`] and [`move_file`].

use crate::error::{IoContext, Result};
use crate::signature::custom_signature;
use std::fs;
use std::path::Path;

/// Prefs files that earlier releases shipped under another name, as `(old, new)`.
const RENAMED: [(&str, &str); 1] = [("aside", "consult")];

/// The name an earlier release gave the prefs file now called `name`, if it had another.
pub fn earlier_name(name: &str) -> Option<&'static str> {
    RENAMED
        .iter()
        .find(|(_, new)| *new == name)
        .map(|(old, _)| *old)
}

/// Rewrites the first line of `bytes` from the old prefs signature to the new one.
///
/// The first line is compared without a leading byte-order mark and without its
/// line ending; both are kept, as is every other byte. Returns `None` when the
/// first line is not the old signature, so the file stays as it is.
pub fn rewrite_signature(kit: &str, old: &str, new: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    let start = if bytes.starts_with(BOM) { BOM.len() } else { 0 };
    let end = bytes[start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| start + p);
    let line_end = if end > start && bytes[end - 1] == b'\r' {
        end - 1
    } else {
        end
    };
    let want = custom_signature(kit, &format!("{old}-prefs"));
    if &bytes[start..line_end] != want.as_bytes() {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + new.len());
    out.extend_from_slice(&bytes[..start]);
    out.extend_from_slice(custom_signature(kit, &format!("{new}-prefs")).as_bytes());
    out.extend_from_slice(&bytes[line_end..]);
    Some(out)
}

/// The text discovery shows for an old-named file: the file with its signature
/// rewritten, or the file as it is when its first line is not the old signature.
pub fn renamed_text(kit: &str, old: &str, new: &str, text: &str) -> String {
    rewrite_signature(kit, old, new, text.as_bytes())
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| text.to_string())
}

/// What [`move_file`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    /// The signature line was rewritten; false when the first line was something else.
    pub signature_rewritten: bool,
}

/// Moves the prefs file `from` (named `old`) to `to` (named `new`) and rewrites its signature line.
///
/// The file is renamed first, so it keeps its permissions and every byte
/// after the first line; then only the first line is rewritten in place.
/// `to` must not exist; the caller checked that.
pub fn move_file(kit: &str, old: &str, new: &str, from: &Path, to: &Path) -> Result<Moved> {
    fs::rename(from, to).ctx(|| format!("renaming {} to {}", from.display(), to.display()))?;
    let bytes = fs::read(to).ctx(|| format!("reading {}", to.display()))?;
    match rewrite_signature(kit, old, new, &bytes) {
        Some(new_bytes) => {
            crate::util::write_atomic(to, &new_bytes)?;
            Ok(Moved {
                signature_rewritten: true,
            })
        }
        None => Ok(Moved {
            signature_rewritten: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "<!-- k-custom:aside-prefs -->\n# Aside Preferences\n\n## Level\n\n**auto**\n\n## Notes\n\nmine\n";

    #[test]
    fn only_the_consult_file_has_an_earlier_name() {
        assert_eq!(earlier_name("consult"), Some("aside"));
        for n in ["dispatch", "subagent", "git", "comment", "aside"] {
            assert_eq!(earlier_name(n), None, "{n}");
        }
    }

    #[test]
    fn the_signature_line_is_rewritten_and_every_other_byte_kept() {
        let out = rewrite_signature("k", "aside", "consult", OLD.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            OLD.replace("k-custom:aside-prefs", "k-custom:consult-prefs")
        );
        let crlf = OLD.replace('\n', "\r\n");
        let out = rewrite_signature("k", "aside", "consult", crlf.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            crlf.replace("k-custom:aside-prefs", "k-custom:consult-prefs")
        );
        let bom = format!("\u{feff}{OLD}");
        let out = rewrite_signature("k", "aside", "consult", bom.as_bytes()).unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("\u{feff}<!-- k-custom:consult-prefs -->\n")
        );
        let unterminated = "<!-- k-custom:aside-prefs -->";
        assert_eq!(
            rewrite_signature("k", "aside", "consult", unterminated.as_bytes()).unwrap(),
            b"<!-- k-custom:consult-prefs -->"
        );
    }

    #[test]
    fn a_first_line_that_is_not_the_old_signature_is_left_alone() {
        for text in [
            "# Aside Preferences\n<!-- k-custom:aside-prefs -->\n",
            "<!-- other-custom:aside-prefs -->\n",
            "<!-- k-custom:aside-prefs --> extra\n",
            "",
        ] {
            assert!(
                rewrite_signature("k", "aside", "consult", text.as_bytes()).is_none(),
                "{text:?}"
            );
            assert_eq!(renamed_text("k", "aside", "consult", text), text);
        }
    }

    #[test]
    fn moving_renames_the_file_and_rewrites_only_its_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("k--aside-prefs.md");
        let to = dir.path().join("k--consult-prefs.md");
        let crlf = OLD.replace('\n', "\r\n");
        fs::write(&from, &crlf).unwrap();
        let moved = move_file("k", "aside", "consult", &from, &to).unwrap();
        assert!(moved.signature_rewritten);
        assert!(!from.exists());
        assert_eq!(
            fs::read_to_string(&to).unwrap(),
            crlf.replace("k-custom:aside-prefs", "k-custom:consult-prefs")
        );

        let from = dir.path().join("j--aside-prefs.md");
        let to = dir.path().join("j--consult-prefs.md");
        fs::write(&from, "# no signature\n").unwrap();
        let moved = move_file("j", "aside", "consult", &from, &to).unwrap();
        assert!(!moved.signature_rewritten);
        assert_eq!(fs::read_to_string(&to).unwrap(), "# no signature\n");
    }
}
