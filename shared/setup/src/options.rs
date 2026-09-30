//! The parsed command line, independent of how it was parsed.
//!
//! Owns [`Options`], the values every command reads, and [`Kind`], the four
//! commands. `cli` fills it from arguments; tests construct it directly. It
//! holds no behaviour beyond small conversions.
//!
//! Main entry points: [`Options`], [`Kind`] and [`parse_set`].

use crate::binaries::Mode;
use crate::env::Harness;
use crate::error::{Error, Result};
use crate::prefs::schema;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The four commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Full install or reinstall.
    Install,
    /// Prefs, custom rules, native configuration and server registration only.
    Configure,
    /// Reverse what install recorded.
    Uninstall,
    /// Binaries and server registration only.
    Mcp,
}

/// Values shared by all commands.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The `dist/` folder of the kit.
    pub payload: Option<PathBuf>,
    /// The harness home.
    pub home: Option<PathBuf>,
    /// The binary folder.
    pub bin_dir: Option<PathBuf>,
    /// The binaries mode, when given.
    pub binaries: Option<Mode>,
    /// The slate checkout to build from.
    pub slate_dir: Option<PathBuf>,
    /// Workspace roots as an OS path list, when given.
    pub roots: Option<String>,
    /// `--set` values by prefs file and key.
    pub set: BTreeMap<String, BTreeMap<String, String>>,
    /// The custom rules folder; an empty path clears it.
    pub custom_rules: Option<PathBuf>,
    /// Never ask.
    pub yes: bool,
    /// Print the summary and stop.
    pub dry_run: bool,
    /// Harnesses for `mcp`.
    pub harnesses: Vec<Harness>,
    /// For `mcp`: remove instead of install.
    pub uninstall: bool,
    /// For `mcp`: the slate release to download; `None` uses the latest release.
    pub slate_version: Option<String>,
}

/// Parses `<file>.<key>=<value>` into its parts, validating the key.
pub fn parse_set(arg: &str) -> Result<(String, String, String)> {
    let (key, value) = arg.split_once('=').ok_or_else(|| {
        Error::usage(format!("`--set {arg}` must look like <file>.<key>=<value>"))
    })?;
    let setting = schema::find(key).ok_or_else(|| {
        let keys: Vec<String> = schema::SETTINGS
            .iter()
            .map(|s| format!("{}.{}", s.file, s.key))
            .collect();
        Error::usage(format!(
            "unknown prefs key `{key}`; keys are: {}",
            keys.join(", ")
        ))
    })?;
    Ok((
        setting.file.to_string(),
        setting.key.to_string(),
        value.to_string(),
    ))
}

impl Options {
    /// Records one `--set` argument.
    pub fn add_set(&mut self, arg: &str) -> Result<()> {
        let (file, key, value) = parse_set(arg)?;
        self.set.entry(file).or_default().insert(key, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_arguments_are_split_at_the_first_equals_sign() {
        let (f, k, v) = parse_set("git.commit-format=<type>: a=b").unwrap();
        assert_eq!(
            (f.as_str(), k.as_str(), v.as_str()),
            ("git", "commit-format", "<type>: a=b")
        );
        let mut o = Options::default();
        o.add_set("aside.codex.model=gpt-6").unwrap();
        o.add_set("aside.level=auto").unwrap();
        assert_eq!(o.set["aside"]["codex.model"], "gpt-6");
        assert_eq!(o.set["aside"].len(), 2);
    }

    #[test]
    fn bad_set_arguments_name_the_problem() {
        assert!(
            parse_set("aside.level")
                .unwrap_err()
                .to_string()
                .contains("<file>.<key>=<value>")
        );
        let e = parse_set("aside.nope=1").unwrap_err().to_string();
        assert!(e.contains("unknown prefs key") && e.contains("aside.level"));
    }
}
