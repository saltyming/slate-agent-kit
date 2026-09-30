//! The install manifest, `<home>/.<kit>-manifest.toml`, and the reader for the
//! older line-format manifest.
//!
//! Owns what an install records so uninstall can reverse it: kit and version,
//! every installed file and folder, backups, binaries, registrations and edited
//! configuration keys. It does not decide what to install or perform removals.
//!
//! Main entry points: [`Manifest::load`], [`Manifest::save`], [`Manifest::remove_files`]
//! and the `record_*` methods.

use crate::config::ConfigRecord;
use crate::env::Harness;
use crate::error::{Error, IoContext, Result};
use crate::util::{now_iso, write_atomic};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Whether a manifest entry is a file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// A single file.
    File,
    /// A folder installed as a whole (a skill).
    Dir,
}

/// An installed file or folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute path.
    pub path: PathBuf,
    /// File or folder.
    pub kind: EntryKind,
    /// True when the user owns it (prefs, custom rules): uninstall asks first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub user: bool,
}

/// A backup the installer made before replacing a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupEntry {
    /// The backup file.
    pub path: PathBuf,
    /// The file it was copied from.
    pub original: PathBuf,
}

/// A server registration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registration {
    /// `mcp` for a CLI-registered server, `kimi-plugin` for the Kimi plugin.
    pub kind: String,
    /// Harness name.
    pub harness: String,
    /// Server name for `mcp`; empty for the plugin.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server: String,
    /// Plugin folder for `kimi-plugin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Everything one install recorded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Kit name.
    pub kit: String,
    /// Kit version; empty for a manifest read from the line format.
    #[serde(default)]
    pub version: String,
    /// Slate release the binaries came from.
    #[serde(default)]
    pub slate_version: String,
    /// When the manifest was last written.
    #[serde(default)]
    pub updated_at: String,
    /// Binary folder used by the install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin_dir: Option<PathBuf>,
    /// Custom rules folder chosen by the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_rules_dir: Option<PathBuf>,
    /// Workspace roots given for dispatch and palette.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// Custom rule files the installer copied into `rules/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_rules: Vec<PathBuf>,
    /// Installed files and folders.
    #[serde(default)]
    pub files: Vec<FileEntry>,
    /// Backups.
    #[serde(default)]
    pub backups: Vec<BackupEntry>,
    /// Installed binaries.
    #[serde(default)]
    pub binaries: Vec<PathBuf>,
    /// Server registrations.
    #[serde(default)]
    pub registrations: Vec<Registration>,
    /// Edited configuration keys.
    #[serde(default)]
    pub config: Vec<ConfigRecord>,
    /// Folders the installer created; uninstall removes them when they are empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_dirs: Vec<PathBuf>,
    /// Files the installer created and may remove when they hold nothing of anyone else's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_files: Vec<PathBuf>,
    /// True when this manifest was read from the line format (never serialised).
    #[serde(skip)]
    pub from_legacy: bool,
}

/// Path of the TOML manifest of `kit` in `home`.
pub fn manifest_path(home: &Path, kit: &str) -> PathBuf {
    home.join(format!(".{kit}-manifest.toml"))
}

/// Paths where an earlier installer wrote its line-format manifest.
pub fn legacy_manifest_paths(home: &Path, kit: &str, harness: Harness) -> Vec<PathBuf> {
    let mut v = vec![home.join(format!(".{kit}-manifest"))];
    if harness == Harness::Kimi {
        // The 0.7.x Kimi installer named it after the harness, not the kit.
        v.push(home.join(".kimi-code-agent-kit-manifest"));
    }
    v
}

impl Manifest {
    /// A new, empty manifest for `kit`.
    pub fn new(kit: &str) -> Manifest {
        Manifest {
            kit: kit.to_string(),
            ..Manifest::default()
        }
    }

    /// Loads the manifest of `kit` from `home`, reading the line format when no TOML manifest exists.
    pub fn load(home: &Path, kit: &str, harness: Harness) -> Result<Option<Manifest>> {
        let path = manifest_path(home, kit);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut m: Manifest = toml::from_str(&text).map_err(|e| {
                    Error::config(format!("cannot parse the manifest {}: {e}", path.display()))
                        .with_fix(
                            "move the manifest away and re-run; the installed files stay untouched",
                        )
                })?;
                if m.kit.is_empty() {
                    m.kit = kit.to_string();
                }
                return Ok(Some(m));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
        }
        for legacy in legacy_manifest_paths(home, kit, harness) {
            if let Some(text) = crate::util::read_text_opt(&legacy)? {
                let mut m = Manifest::parse_legacy(kit, &text);
                m.from_legacy = true;
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// Parses the line format: one path per line, `## backup: <path>` lines and other `## ` comments.
    pub fn parse_legacy(kit: &str, text: &str) -> Manifest {
        let mut m = Manifest::new(kit);
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("## backup:") {
                let backup = PathBuf::from(rest.trim());
                let original = backup
                    .to_string_lossy()
                    .rsplit_once(".bak-")
                    .map(|(orig, _)| PathBuf::from(orig))
                    .unwrap_or_else(|| backup.clone());
                m.backups.push(BackupEntry {
                    path: backup,
                    original,
                });
                continue;
            }
            if line.starts_with("## ") {
                continue;
            }
            let path = PathBuf::from(line);
            let kind = if path.is_dir() || path.join("SKILL.md").exists() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let user = name.ends_with("-prefs.md") || is_probably_custom(&path);
            if !m.files.iter().any(|f| f.path == path) {
                m.files.push(FileEntry { path, kind, user });
            }
        }
        m
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.backups.is_empty()
            && self.binaries.is_empty()
            && self.registrations.is_empty()
            && self.config.is_empty()
            && self.custom_rules.is_empty()
            && self.created_dirs.is_empty()
            && self.created_files.is_empty()
    }

    /// Records a folder the installer created.
    pub fn record_created_dir(&mut self, path: &Path) {
        if !self.created_dirs.iter().any(|d| d == path) {
            self.created_dirs.push(path.to_path_buf());
        }
    }

    /// Records a file the installer created.
    pub fn record_created_file(&mut self, path: &Path) {
        if !self.created_files.iter().any(|f| f == path) {
            self.created_files.push(path.to_path_buf());
        }
    }

    /// Writes the manifest atomically.
    pub fn save(&mut self, home: &Path) -> Result<()> {
        self.updated_at = now_iso();
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::config(format!("cannot serialise the manifest: {e}")))?;
        let header = "# Written by slate-setup. Uninstall reads it; do not edit.\n";
        write_atomic(
            &manifest_path(home, &self.kit),
            format!("{header}{text}").as_bytes(),
        )
    }

    /// Deletes the line-format manifests, which the TOML manifest replaces.
    pub fn delete_legacy(home: &Path, kit: &str, harness: Harness) -> Result<()> {
        for p in legacy_manifest_paths(home, kit, harness) {
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(format!("removing {}", p.display()), e)),
            }
        }
        Ok(())
    }

    /// Deletes the TOML manifest and any line-format manifest.
    pub fn delete(home: &Path, kit: &str, harness: Harness) -> Result<()> {
        let mut paths = vec![manifest_path(home, kit)];
        paths.extend(legacy_manifest_paths(home, kit, harness));
        for p in paths {
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(format!("removing {}", p.display()), e)),
            }
        }
        Ok(())
    }

    /// Records an installed file or folder, replacing an earlier entry for the same path.
    pub fn record_file(&mut self, path: &Path, kind: EntryKind, user: bool) {
        self.files.retain(|f| f.path != path);
        self.files.push(FileEntry {
            path: path.to_path_buf(),
            kind,
            user,
        });
    }

    /// Records a backup.
    pub fn record_backup(&mut self, path: &Path, original: &Path) {
        if !self.backups.iter().any(|b| b.path == path) {
            self.backups.push(BackupEntry {
                path: path.to_path_buf(),
                original: original.to_path_buf(),
            });
        }
    }

    /// Records an installed binary.
    pub fn record_binary(&mut self, path: &Path) {
        if !self.binaries.iter().any(|b| b == path) {
            self.binaries.push(path.to_path_buf());
        }
    }

    /// Records a registration, replacing an earlier identical one.
    pub fn record_registration(&mut self, reg: Registration) {
        self.registrations.retain(|r| *r != reg);
        self.registrations.push(reg);
    }

    /// Records an edited configuration key.
    ///
    /// When the key was already edited by an earlier install and still held the
    /// value that install wrote, the original previous value is kept, so a
    /// reinstall never turns the installer's own earlier write into "the
    /// original".
    pub fn record_config(&mut self, mut rec: ConfigRecord) {
        if let Some(pos) = self
            .config
            .iter()
            .position(|c| c.file == rec.file && c.path == rec.path)
        {
            let old = self.config.remove(pos);
            let still_ours = rec.previous.as_deref() == Some(old.written.as_str())
                || (!old.added.is_empty() && rec.previous.is_some());
            if still_ours {
                rec.previous = old.previous;
                rec.created_parents = old.created_parents;
                for a in old.added {
                    if !rec.added.contains(&a) {
                        rec.added.push(a);
                    }
                }
            }
        }
        self.config.push(rec);
    }

    /// Drops the record of a key whose edit was undone.
    pub fn forget_config(&mut self, file: &Path, path: &[String]) {
        self.config.retain(|c| !(c.file == file && c.path == path));
    }

    /// The record of an edited key, if any.
    pub fn config_record(&self, file: &Path, path: &[&str]) -> Option<&ConfigRecord> {
        self.config
            .iter()
            .find(|c| c.file == file && c.path.iter().map(String::as_str).eq(path.iter().copied()))
    }

    /// True when `path` is listed as an installed binary.
    pub fn lists_binary(&self, path: &Path) -> bool {
        self.binaries
            .iter()
            .any(|b| crate::util::same_path(b, path))
    }
}

fn is_probably_custom(path: &Path) -> bool {
    // A file the line-format installer recorded that carries the custom signature
    // is user-owned; the signature check at uninstall time is authoritative, this
    // only seeds the `user` flag for listing.
    fs::read_to_string(path)
        .ok()
        .and_then(|t| t.lines().next().map(|l| l.contains("-custom:")))
        .unwrap_or(false)
}

/// Finds every TOML manifest that lists `binary`, other than `own`.
pub fn other_manifests_listing(homes: &[PathBuf], own: &Path, binary: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for home in homes {
        let Ok(entries) = fs::read_dir(home) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with('.') && name.ends_with("-manifest.toml")) {
                continue;
            }
            let path = entry.path();
            if crate::util::same_path(&path, own) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(m) = toml::from_str::<Manifest>(&text) else {
                continue;
            };
            if m.lists_binary(binary) {
                found.push(path);
            }
        }
    }
    found
}

/// Reads a manifest file by path, for callers that already know where it is.
pub fn read_manifest_file(path: &Path) -> Result<Manifest> {
    let text = fs::read_to_string(path).ctx(|| format!("reading {}", path.display()))?;
    toml::from_str(&text)
        .map_err(|e| Error::config(format!("cannot parse the manifest {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Manifest::new("k");
        m.version = "1.2.3".into();
        m.record_file(&dir.path().join("a.md"), EntryKind::File, false);
        m.record_file(&dir.path().join("skills"), EntryKind::Dir, true);
        m.record_backup(&dir.path().join("a.md.bak-1"), &dir.path().join("a.md"));
        m.record_binary(&dir.path().join("aside"));
        m.record_registration(Registration {
            kind: "mcp".into(),
            harness: "codex".into(),
            server: "aside".into(),
            path: None,
        });
        m.record_config(ConfigRecord {
            file: dir.path().join("config.toml"),
            path: vec!["agents".into(), "default_subagent_model".into()],
            previous: None,
            written: "\"x\"".into(),
            created_parents: 1,
            added: vec![],
        });
        m.save(dir.path()).unwrap();
        let back = Manifest::load(dir.path(), "k", Harness::Codex)
            .unwrap()
            .unwrap();
        m.updated_at = back.updated_at.clone();
        assert_eq!(back, m);
        assert!(!back.from_legacy);
    }

    #[test]
    fn reads_line_format_with_backups_and_comments() {
        let text = "## install @ 2026-09-19T10:00:00Z\n## backup: /h/AGENTS.md.bak-20260919T100000Z\n/h/AGENTS.md\n/h/rules/a.md\n\n/h/rules/a.md\n";
        let m = Manifest::parse_legacy("codex-agent-kit", text);
        assert_eq!(m.files.len(), 2);
        assert_eq!(m.backups.len(), 1);
        assert_eq!(m.backups[0].original, PathBuf::from("/h/AGENTS.md"));
    }

    #[test]
    fn kimi_legacy_manifest_name_is_found() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(".kimi-code-agent-kit-manifest"),
            "/x/y.md\n",
        )
        .unwrap();
        let m = Manifest::load(dir.path(), "kimi-agent-kit", Harness::Kimi)
            .unwrap()
            .unwrap();
        assert!(m.from_legacy);
        assert_eq!(m.files.len(), 1);
        // The other harnesses do not look for the Kimi name.
        assert!(
            Manifest::load(dir.path(), "kimi-agent-kit", Harness::Codex)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn toml_manifest_wins_over_legacy() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".k-manifest"), "/x/y.md\n").unwrap();
        let mut m = Manifest::new("k");
        m.version = "2".into();
        m.save(dir.path()).unwrap();
        let back = Manifest::load(dir.path(), "k", Harness::Claude)
            .unwrap()
            .unwrap();
        assert_eq!(back.version, "2");
    }

    #[test]
    fn reinstall_keeps_the_original_previous_value() {
        let mut m = Manifest::new("k");
        let file = PathBuf::from("c.toml");
        let path = vec!["agents".to_string(), "default_subagent_model".to_string()];
        m.record_config(ConfigRecord {
            file: file.clone(),
            path: path.clone(),
            previous: Some("\"orig\"".into()),
            written: "\"one\"".into(),
            created_parents: 0,
            added: vec![],
        });
        // Second install: the key still holds "one" (what we wrote), we change it to "two".
        m.record_config(ConfigRecord {
            file: file.clone(),
            path: path.clone(),
            previous: Some("\"one\"".into()),
            written: "\"two\"".into(),
            created_parents: 0,
            added: vec![],
        });
        assert_eq!(m.config.len(), 1);
        assert_eq!(m.config[0].previous.as_deref(), Some("\"orig\""));
        assert_eq!(m.config[0].written, "\"two\"");
        // Third install: the user changed it to "user" in between; that is the new original.
        m.record_config(ConfigRecord {
            file,
            path,
            previous: Some("\"user\"".into()),
            written: "\"three\"".into(),
            created_parents: 0,
            added: vec![],
        });
        assert_eq!(m.config[0].previous.as_deref(), Some("\"user\""));
    }

    #[test]
    fn other_manifests_are_found_by_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin").join("aside");
        let mut a = Manifest::new("a-kit");
        a.record_binary(&bin);
        a.save(dir.path()).unwrap();
        let mut b = Manifest::new("b-kit");
        b.record_binary(&bin);
        b.save(dir.path()).unwrap();
        let own = manifest_path(dir.path(), "a-kit");
        let others = other_manifests_listing(&[dir.path().to_path_buf()], &own, &bin);
        assert_eq!(others, vec![manifest_path(dir.path(), "b-kit")]);
        assert!(
            other_manifests_listing(
                &[dir.path().to_path_buf()],
                &own,
                &dir.path().join("dispatch")
            )
            .is_empty()
        );
    }
}
