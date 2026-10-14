//! Where each harness keeps the files the installer writes.
//!
//! Owns [`Layout`], the per-harness folders the shared install code consults
//! instead of assuming `<home>/rules`, `<home>/<primary>`, `<home>/skills` and a
//! manifest directly under the home: the manual folder, the rules folder, the
//! skills folder, the manifest folder, the folder backups go to, the servers'
//! state home and the native configuration file. Also owns
//! [`manifest_dirs`], every folder another kit's manifest may sit in, and
//! [`check_home`], the rule that agy's home cannot be moved. It resolves paths
//! only; it reads and writes nothing.
//!
//! Main entry points: [`Layout::new`], [`manifest_dirs`] and [`check_home`].

use crate::env::{Env, Harness, process_home_var};
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

/// The folders and files one harness home uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Target harness.
    pub harness: Harness,
    /// The harness home (`~/.claude`, `~/.codex`, `~/.kimi-code`, `~/.gemini`).
    pub home: PathBuf,
    /// The folder of the primary instruction file.
    pub manual_dir: PathBuf,
    /// The folder of the rule and prefs files.
    pub rules_dir: PathBuf,
    /// The folder that holds one folder per skill.
    pub skills_dir: PathBuf,
    /// The folder of the install manifests (the kit's and the `mcp` command's).
    pub manifest_dir: PathBuf,
    /// The folder a backup is written to; `None` writes it beside the original file.
    pub backup_dir: Option<PathBuf>,
    /// The servers' state home, `SLATE_AGENT_STATE_HOME`.
    pub state_home: PathBuf,
    /// The native configuration file the installer edits.
    pub config_file: PathBuf,
}

impl Layout {
    /// The layout of `harness` with its home at `home`.
    pub fn new(harness: Harness, home: &Path) -> Layout {
        let home = home.to_path_buf();
        let kit_state = home.join("slate-agent-kit");
        match harness {
            Harness::Claude => Layout {
                harness,
                manual_dir: home.clone(),
                rules_dir: home.join("rules"),
                skills_dir: home.join("skills"),
                manifest_dir: home.clone(),
                backup_dir: None,
                state_home: home.clone(),
                config_file: home.join("settings.json"),
                home,
            },
            Harness::Codex | Harness::Kimi => Layout {
                harness,
                manual_dir: home.clone(),
                rules_dir: home.join("rules"),
                skills_dir: home.join("skills"),
                manifest_dir: home.clone(),
                backup_dir: None,
                state_home: kit_state,
                config_file: home.join("config.toml"),
                home,
            },
            // agy shares `config/` with the Antigravity app and keeps its own
            // settings in `antigravity-cli/`; the kit's manifest, backups and the
            // servers' state go to a folder of their own beside them.
            Harness::Agy => Layout {
                harness,
                manual_dir: home.join("config"),
                rules_dir: home.join("config").join("rules"),
                skills_dir: home.join("config").join("skills"),
                manifest_dir: kit_state.clone(),
                backup_dir: Some(kit_state.join("backups")),
                state_home: kit_state,
                config_file: home.join("antigravity-cli").join("settings.json"),
                home,
            },
        }
    }

    /// The installed primary instruction file.
    pub fn primary(&self) -> PathBuf {
        self.manual_dir.join(self.harness.primary_file())
    }

    /// The installed folder of skill `name`.
    pub fn skill(&self, name: &str) -> PathBuf {
        self.skills_dir.join(name)
    }

    /// The path of the TOML manifest of `kit`.
    pub fn manifest_path(&self, kit: &str) -> PathBuf {
        crate::manifest::manifest_path(&self.manifest_dir, kit)
    }
}

/// Refuses a `--home` the harness cannot use.
///
/// agy has no variable that moves its folder: it always reads `.gemini` under
/// the process home, so the only home it accepts is that folder.
pub fn check_home(env: &Env, harness: Harness, home: &Path) -> Result<()> {
    if harness.home_var().is_some() {
        return Ok(());
    }
    let only = env.default_home(harness);
    if crate::util::same_path(home, &only) {
        return Ok(());
    }
    Err(Error::usage(format!(
        "{} always uses {}, so `--home {}` cannot be honoured",
        harness.product(),
        only.display(),
        home.display()
    ))
    .with_state("nothing was changed")
    .with_fix(format!(
        "drop `--home`, or set {} to the folder whose `{}` you want and re-run",
        process_home_var(),
        harness.default_dir_name()
    )))
}

/// Every folder where a kit's manifest may list a shared binary: the manifest
/// folder of each harness at its resolved and at its default home, plus `extra`.
pub fn manifest_dirs(env: &Env, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Harness::ALL
        .iter()
        .map(|h| Layout::new(*h, &env.harness_home(*h)).manifest_dir)
        .collect();
    dirs.extend(
        Harness::ALL
            .iter()
            .map(|h| Layout::new(*h, &env.default_home(*h)).manifest_dir),
    );
    dirs.extend(extra.iter().cloned());
    dirs.sort();
    dirs.dedup();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    /// The three original harnesses keep the paths they had before the layout existed.
    #[test]
    fn existing_harnesses_keep_their_paths() {
        let home = p(&["h", ".claude"]);
        let l = Layout::new(Harness::Claude, &home);
        assert_eq!(l.manual_dir, home);
        assert_eq!(l.primary(), home.join("CLAUDE.md"));
        assert_eq!(l.rules_dir, home.join("rules"));
        assert_eq!(l.skill("x"), home.join("skills").join("x"));
        assert_eq!(l.manifest_dir, home);
        assert_eq!(l.manifest_path("k"), home.join(".k-manifest.toml"));
        assert_eq!(l.backup_dir, None);
        assert_eq!(l.state_home, home);
        assert_eq!(l.config_file, home.join("settings.json"));
        for (h, dir) in [(Harness::Codex, ".codex"), (Harness::Kimi, ".kimi-code")] {
            let home = p(&["h", dir]);
            let l = Layout::new(h, &home);
            assert_eq!(l.manual_dir, home);
            assert_eq!(l.primary(), home.join("AGENTS.md"));
            assert_eq!(l.rules_dir, home.join("rules"));
            assert_eq!(l.skills_dir, home.join("skills"));
            assert_eq!(l.manifest_dir, home);
            assert_eq!(l.backup_dir, None);
            assert_eq!(l.state_home, home.join("slate-agent-kit"));
            assert_eq!(l.config_file, home.join("config.toml"));
        }
    }

    #[test]
    fn agy_keeps_its_manual_rules_and_skills_under_config_and_its_state_beside() {
        let home = p(&["h", ".gemini"]);
        let l = Layout::new(Harness::Agy, &home);
        assert_eq!(l.primary(), home.join("config").join("AGENTS.md"));
        assert_eq!(l.rules_dir, home.join("config").join("rules"));
        assert_eq!(
            l.skill("palette-init"),
            home.join("config").join("skills").join("palette-init")
        );
        let state = home.join("slate-agent-kit");
        assert_eq!(l.manifest_dir, state);
        assert_eq!(
            l.manifest_path("agy-agent-kit"),
            state.join(".agy-agent-kit-manifest.toml")
        );
        assert_eq!(l.backup_dir, Some(state.join("backups")));
        assert_eq!(l.state_home, state);
        assert_eq!(
            l.config_file,
            home.join("antigravity-cli").join("settings.json")
        );
    }

    #[test]
    fn agy_accepts_only_its_own_folder_as_home() {
        let env = Env::for_home(p(&["u"]));
        assert!(check_home(&env, Harness::Agy, &p(&["u", ".gemini"])).is_ok());
        let err = check_home(&env, Harness::Agy, &p(&["x", ".gemini"])).unwrap_err();
        let fix = err.fix.clone().unwrap_or_default();
        assert!(fix.contains(process_home_var()), "{fix}");
        assert!(err.to_string().contains("cannot be honoured"), "{err}");
        // The harnesses with a home variable take any folder.
        assert!(check_home(&env, Harness::Codex, &p(&["x"])).is_ok());
    }

    #[test]
    fn manifest_dirs_cover_resolved_and_default_homes() {
        let mut env = Env::for_home("/u");
        env.set_var("CODEX_HOME", "/elsewhere");
        let dirs = manifest_dirs(&env, &[PathBuf::from("/x")]);
        assert!(dirs.contains(&PathBuf::from("/elsewhere")));
        assert!(dirs.contains(&Path::new("/u").join(".codex")));
        assert!(dirs.contains(&Path::new("/u").join(".claude")));
        assert!(dirs.contains(&PathBuf::from("/x")));
        assert!(dirs.contains(&Path::new("/u").join(".gemini").join("slate-agent-kit")));
    }
}
