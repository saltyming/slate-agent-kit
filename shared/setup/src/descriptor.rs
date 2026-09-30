//! The kit descriptor `dist/kit.toml` and the payload folder it describes.
//!
//! Owns parsing and validation of the descriptor and the layout of a payload
//! (`primary`, `rules/`, `skills/`, `prefs/`). It does not install anything;
//! `install` reads the files this module locates.
//!
//! Main entry points: [`Descriptor::load`], [`Payload`] and [`LoadMode`].

use crate::env::Harness;
use crate::error::{Error, IoContext, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// How a harness loads instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadMode {
    /// The harness loads every file in its rules folder.
    RulesDir,
    /// The harness loads only its primary file, which the installer concatenates.
    Concat,
}

/// The parsed contents of `dist/kit.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Descriptor {
    /// File prefix and manifest name.
    pub kit: String,
    /// Target harness name.
    pub harness: String,
    /// Kit version.
    pub version: String,
    /// Slate release that provides the binaries.
    pub slate_version: String,
    /// Primary instruction file inside `dist/`.
    pub primary: String,
    /// How the harness loads instructions.
    pub load: LoadMode,
    /// Rule files in concatenation order.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Skill folder names.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Prefs file names, without the `-prefs` suffix.
    #[serde(default)]
    pub prefs: Vec<String>,
    /// Servers to register.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Legacy cleanups to run.
    #[serde(default)]
    pub legacy: Vec<String>,
}

/// The servers the installer knows how to install and register.
pub const KNOWN_SERVERS: [&str; 3] = ["aside", "dispatch", "palette"];

/// The prefs files the installer knows how to configure.
pub const KNOWN_PREFS: [&str; 5] = ["aside", "dispatch", "subagent", "git", "comment"];

impl Descriptor {
    /// Reads and validates `<payload>/kit.toml`.
    pub fn load(payload: &Path) -> Result<Descriptor> {
        let path = payload.join("kit.toml");
        let text = fs::read_to_string(&path).map_err(|e| {
            Error::payload(format!(
                "cannot read the kit descriptor {}: {e}",
                path.display()
            ))
            .with_fix("pass --payload <dir> with the kit's dist/ folder")
        })?;
        let desc: Descriptor = toml::from_str(&text).map_err(|e| {
            Error::payload(format!("invalid kit descriptor {}: {e}", path.display()))
        })?;
        desc.validate()?;
        Ok(desc)
    }

    fn validate(&self) -> Result<()> {
        if self.kit.is_empty() || self.kit.contains(['/', '\\']) {
            return Err(Error::payload("kit.toml: `kit` must be a plain name"));
        }
        if Harness::parse(&self.harness).is_none() {
            return Err(Error::payload(format!(
                "kit.toml: unknown harness `{}` (expected claude, codex or kimi)",
                self.harness
            )));
        }
        for name in self
            .rules
            .iter()
            .chain(&self.skills)
            .chain(&self.servers)
            .chain(&self.prefs)
        {
            if name.is_empty() || name.contains(['/', '\\']) || name == ".." || name == "." {
                return Err(Error::payload(format!(
                    "kit.toml: `{name}` is not a plain file name"
                )));
            }
        }
        if self.primary.contains(['/', '\\']) {
            return Err(Error::payload(
                "kit.toml: `primary` must be a plain file name",
            ));
        }
        for s in &self.servers {
            if !KNOWN_SERVERS.contains(&s.as_str()) {
                return Err(Error::payload(format!("kit.toml: unknown server `{s}`")));
            }
        }
        for p in &self.prefs {
            if !KNOWN_PREFS.contains(&p.as_str()) {
                return Err(Error::payload(format!(
                    "kit.toml: unknown prefs file `{p}`"
                )));
            }
        }
        Ok(())
    }

    /// The target harness.
    pub fn harness(&self) -> Harness {
        Harness::parse(&self.harness).unwrap_or(Harness::Claude)
    }
}

/// A validated payload folder and its descriptor.
#[derive(Debug, Clone)]
pub struct Payload {
    /// The `dist/` folder.
    pub dir: PathBuf,
    /// Its descriptor.
    pub desc: Descriptor,
}

impl Payload {
    /// Loads the descriptor and checks that every file it names exists.
    pub fn load(dir: &Path) -> Result<Payload> {
        let desc = Descriptor::load(dir)?;
        let payload = Payload {
            dir: dir.to_path_buf(),
            desc,
        };
        let mut missing = Vec::new();
        if !dir.join(&payload.desc.primary).is_file() {
            missing.push(payload.desc.primary.clone());
        }
        for r in &payload.desc.rules {
            if !payload.rule_path(r).is_file() {
                missing.push(format!("rules/{r}"));
            }
        }
        for s in &payload.desc.skills {
            if !payload.skill_dir(s).join("SKILL.md").is_file() {
                missing.push(format!("skills/{s}/SKILL.md"));
            }
        }
        for p in &payload.desc.prefs {
            if !payload.prefs_template(p).is_file() {
                missing.push(format!("prefs/{p}-prefs.md"));
            }
        }
        if !missing.is_empty() {
            return Err(Error::payload(format!(
                "the payload {} lacks files that kit.toml names: {}",
                dir.display(),
                missing.join(", ")
            )));
        }
        Ok(payload)
    }

    /// Path of a rule file in the payload.
    pub fn rule_path(&self, name: &str) -> PathBuf {
        self.dir.join("rules").join(name)
    }

    /// Path of a skill folder in the payload.
    pub fn skill_dir(&self, name: &str) -> PathBuf {
        self.dir.join("skills").join(name)
    }

    /// Path of a prefs template in the payload.
    pub fn prefs_template(&self, name: &str) -> PathBuf {
        self.dir.join("prefs").join(format!("{name}-prefs.md"))
    }

    /// Path of the primary file in the payload.
    pub fn primary_path(&self) -> PathBuf {
        self.dir.join(&self.desc.primary)
    }

    /// Reads a payload text file.
    pub fn read(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path).ctx(|| format!("reading {}", path.display()))
    }

    /// Lists every file under a skill folder, relative to it, in sorted order.
    pub fn skill_files(&self, name: &str) -> Result<Vec<PathBuf>> {
        let root = self.skill_dir(name);
        let mut out = Vec::new();
        collect_files(&root, &root, &mut out)?;
        out.sort();
        Ok(out)
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).ctx(|| format!("reading {}", dir.display()))? {
        let entry = entry.ctx(|| format!("reading {}", dir.display()))?;
        let path = entry.path();
        let ty = entry
            .file_type()
            .ctx(|| format!("reading {}", path.display()))?;
        if ty.is_dir() {
            collect_files(root, &path, out)?;
        } else if ty.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIT: &str = r#"
kit = "claude-agent-kit"
harness = "claude"
version = "13.0.0"
slate_version = "0.7.0"
primary = "CLAUDE.md"
load = "rules-dir"
rules = ["claude-agent-kit--a.md"]
skills = ["palette-init"]
prefs = ["aside", "git"]
servers = ["aside", "dispatch", "palette"]
legacy = ["workslate"]
"#;

    fn payload(dir: &Path) {
        fs::write(dir.join("kit.toml"), KIT).unwrap();
        fs::write(dir.join("CLAUDE.md"), "x").unwrap();
        fs::create_dir_all(dir.join("rules")).unwrap();
        fs::write(dir.join("rules/claude-agent-kit--a.md"), "x").unwrap();
        fs::create_dir_all(dir.join("skills/palette-init")).unwrap();
        fs::write(dir.join("skills/palette-init/SKILL.md"), "x").unwrap();
        fs::create_dir_all(dir.join("prefs")).unwrap();
        fs::write(dir.join("prefs/aside-prefs.md"), "x").unwrap();
        fs::write(dir.join("prefs/git-prefs.md"), "x").unwrap();
    }

    #[test]
    fn loads_a_complete_payload() {
        let dir = tempfile::tempdir().unwrap();
        payload(dir.path());
        let p = Payload::load(dir.path()).unwrap();
        assert_eq!(p.desc.kit, "claude-agent-kit");
        assert_eq!(p.desc.harness(), Harness::Claude);
        assert_eq!(p.desc.load, LoadMode::RulesDir);
        assert_eq!(
            p.skill_files("palette-init").unwrap(),
            vec![PathBuf::from("SKILL.md")]
        );
    }

    #[test]
    fn reports_every_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        payload(dir.path());
        fs::remove_file(dir.path().join("rules/claude-agent-kit--a.md")).unwrap();
        fs::remove_file(dir.path().join("prefs/git-prefs.md")).unwrap();
        let err = Payload::load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("rules/claude-agent-kit--a.md"), "{err}");
        assert!(err.contains("prefs/git-prefs.md"), "{err}");
    }

    #[test]
    fn rejects_unknown_harness_and_path_like_names() {
        let bad = KIT.replace("harness = \"claude\"", "harness = \"vim\"");
        let d: Descriptor = toml::from_str(&bad).unwrap();
        assert!(d.validate().is_err());
        let bad = KIT.replace("claude-agent-kit--a.md", "../evil.md");
        let d: Descriptor = toml::from_str(&bad).unwrap();
        assert!(d.validate().is_err());
        let bad = KIT.replace("\"aside\", \"git\"", "\"aside\", \"nope\"");
        let d: Descriptor = toml::from_str(&bad).unwrap();
        assert!(d.validate().is_err());
    }

    #[test]
    fn missing_descriptor_names_the_fix() {
        let dir = tempfile::tempdir().unwrap();
        let err = Descriptor::load(dir.path()).unwrap_err();
        assert!(err.fix.unwrap().contains("--payload"));
    }
}
