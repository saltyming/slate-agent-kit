//! One kit in one harness home: where its files go and what is already there.
//!
//! Owns resolving the harness home and binary folder, locating the manifest,
//! the prefs files and the configuration file, and reading the state of the
//! existing prefs files. It does not ask questions or change anything.
//!
//! Main entry points: [`Kit::open`], [`Kit::prefs_states`] and [`PrefsState`].

use crate::descriptor::Payload;
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::manifest::Manifest;
use crate::options::Options;
use crate::prefs::{self, State, schema::ValidationCtx};
use crate::util::read_text_opt;
use std::path::{Path, PathBuf};

/// A kit's target and current state.
#[derive(Debug, Clone)]
pub struct Kit {
    /// The kit payload.
    pub payload: Payload,
    /// Target harness.
    pub harness: Harness,
    /// Harness home.
    pub home: PathBuf,
    /// `<home>/rules`.
    pub rules_dir: PathBuf,
    /// Binary folder.
    pub bin_dir: PathBuf,
    /// The manifest of an earlier install, if any.
    pub prior: Option<Manifest>,
}

/// The state of one prefs file.
#[derive(Debug, Clone)]
pub struct PrefsState {
    /// Name without the `-prefs` suffix.
    pub name: String,
    /// Installed path.
    pub dest: PathBuf,
    /// The payload template text.
    pub template: String,
    /// The installed text, if the file exists.
    pub existing: Option<String>,
    /// Its state.
    pub state: State,
}

/// Finds the payload folder: `--payload`, else `./dist`, else `dist/` beside the executable.
pub fn find_payload(opts: &Options) -> Result<PathBuf> {
    if let Some(p) = &opts.payload {
        return Ok(p.clone());
    }
    let mut candidates = vec![PathBuf::from("dist")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("dist"));
    }
    candidates
        .into_iter()
        .find(|c| c.join("kit.toml").is_file())
        .ok_or_else(|| {
            Error::usage(
                "no kit payload found: there is no dist/kit.toml here or beside the installer",
            )
            .with_fix("pass --payload <dir> with the kit's dist/ folder")
        })
}

impl Kit {
    /// Loads the payload and resolves the home and binary folder.
    pub fn open(env: &Env, opts: &Options) -> Result<Kit> {
        let dir = find_payload(opts)?;
        let payload = Payload::load(&dir)?;
        let harness = payload.desc.harness();
        let home = opts
            .home
            .clone()
            .unwrap_or_else(|| env.harness_home(harness));
        let bin_dir = opts
            .bin_dir
            .clone()
            .unwrap_or_else(|| env.default_bin_dir());
        let prior = Manifest::load(&home, &payload.desc.kit, harness)?;
        Ok(Kit {
            rules_dir: home.join("rules"),
            payload,
            harness,
            home,
            bin_dir,
            prior,
        })
    }

    /// The kit name.
    pub fn name(&self) -> &str {
        &self.payload.desc.kit
    }

    /// Where a prefs file is installed.
    pub fn prefs_dest(&self, name: &str) -> PathBuf {
        self.rules_dir
            .join(format!("{}--{name}-prefs.md", self.name()))
    }

    /// The configuration file the installer edits.
    pub fn config_file(&self) -> PathBuf {
        match self.harness {
            Harness::Claude => self.home.join("settings.json"),
            Harness::Codex | Harness::Kimi => self.home.join("config.toml"),
        }
    }

    /// The servers the kit registers.
    pub fn servers(&self) -> Vec<String> {
        self.payload.desc.servers.clone()
    }

    /// The path of the manifest.
    pub fn manifest_path(&self) -> PathBuf {
        crate::manifest::manifest_path(&self.home, self.name())
    }

    /// The Kimi model aliases in `config.toml`, when the harness is Kimi and the file parses.
    pub fn kimi_models(&self) -> Option<Vec<String>> {
        if self.harness != Harness::Kimi {
            return None;
        }
        let text = read_text_opt(&self.config_file()).ok()??;
        let doc = crate::config::TomlDoc::parse(&text).ok()?;
        Some(crate::harness::kimi::model_aliases(&doc))
    }

    /// The validation context for prefs values.
    pub fn validation_ctx(&self) -> ValidationCtx {
        ValidationCtx {
            harness: self.harness,
            kimi_models: self.kimi_models(),
        }
    }

    /// Reads the state of every prefs file the kit ships.
    pub fn prefs_states(&self) -> Result<Vec<PrefsState>> {
        let old_manifest = self.prior.as_ref().is_some_and(|m| m.from_legacy);
        let mut out = Vec::new();
        for name in &self.payload.desc.prefs {
            let dest = self.prefs_dest(name);
            let template = self.payload.read(&self.payload.prefs_template(name))?;
            let existing = read_text_opt(&dest)?;
            let listed = old_manifest
                && self
                    .prior
                    .as_ref()
                    .is_some_and(|m| m.files.iter().any(|f| f.path == dest));
            let state = prefs::classify_state(name, existing.as_deref(), listed);
            out.push(PrefsState {
                name: name.clone(),
                dest,
                template,
                existing,
                state,
            });
        }
        Ok(out)
    }
}

/// True when `dir` is on `PATH`.
pub fn on_path(env: &Env, dir: &Path) -> bool {
    env.get("PATH")
        .is_some_and(|p| std::env::split_paths(p).any(|d| crate::util::same_path(&d, dir)))
}
