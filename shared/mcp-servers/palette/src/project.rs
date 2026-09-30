//! Project root resolution and project path checks.
//!
//! Owns finding the server's project root and extra roots from the environment,
//! accepting a tool's `project` path only inside them, and rejecting write paths
//! that use `..` or symbolic links. Does not read palette documents.
//! Entry points: [`Roots::from_env`], [`Roots::resolve_project`], [`check_write_path`].

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::errors::{ErrCode, PalError, Res};

/// Where environment variables come from; tests supply a fixed map instead of the
/// process environment.
pub trait EnvSource {
    /// The value of `key`, if set.
    fn get(&self, key: &str) -> Option<OsString>;
}

/// The process environment.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }
}

/// A fixed set of variables, for tests.
#[derive(Default)]
pub struct MapEnv(pub Vec<(String, OsString)>);

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<OsString> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

/// The server's containment boundary.
#[derive(Clone, Debug, Default)]
pub struct Roots {
    /// Canonical project root; `None` when the working directory is not plausibly a project.
    pub project_root: Option<PathBuf>,
    /// Canonical extra roots from `PALETTE_EXTRA_ROOTS`.
    pub extra_roots: Vec<PathBuf>,
}

fn home_dir(env: &dyn EnvSource) -> Option<PathBuf> {
    env.get("HOME")
        .or_else(|| env.get("USERPROFILE"))
        .map(PathBuf::from)
}

// Mirrors dispatch's `resolve_project_root` (shared/mcp-servers/dispatch/src/main.rs),
// with the environment injected so tests do not mutate the process environment.
fn resolve_project_root(env: &dyn EnvSource, cwd: &Path) -> Option<PathBuf> {
    for key in [
        "SLATE_PROJECT_DIR",
        "AGENT_KIT_PROJECT_DIR",
        "CLAUDE_PROJECT_DIR",
    ] {
        if let Some(v) = env.get(key)
            && !v.to_string_lossy().trim().is_empty()
        {
            let p = PathBuf::from(v);
            return Some(p.canonicalize().unwrap_or(p));
        }
    }
    let canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if plausible_fallback_root(env, &canon, &implausible_roots(env)) {
        Some(canon)
    } else {
        None
    }
}

fn implausible_roots(env: &dyn EnvSource) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(h) = home_dir(env) {
        roots.push(h.join(".claude"));
        roots.push(
            env.get("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".codex")),
        );
        roots.push(
            env.get("KIMI_CODE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".kimi-code")),
        );
        roots.push(
            env.get("SLATE_AGENT_STATE_HOME")
                .or_else(|| env.get("AGENT_KIT_STATE_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|| h.join(".slate-agent-kit")),
        );
    }
    roots
        .into_iter()
        .map(|r| r.canonicalize().unwrap_or(r))
        .collect()
}

fn plausible_fallback_root(env: &dyn EnvSource, canon: &Path, denied: &[PathBuf]) -> bool {
    if canon.parent().is_none() {
        return false;
    }
    if let Some(h) = home_dir(env) {
        let h = h.canonicalize().unwrap_or(h);
        if canon == h.as_path() {
            return false;
        }
    }
    !denied.iter().any(|d| canon.starts_with(d))
}

fn parse_extra_roots(env: &dyn EnvSource) -> Vec<PathBuf> {
    match env.get("PALETTE_EXTRA_ROOTS") {
        Some(v) => std::env::split_paths(&v)
            .filter_map(|p| p.canonicalize().ok())
            .collect(),
        None => Vec::new(),
    }
}

impl Roots {
    /// Reads the roots from `env`; `cwd` is the fallback project root when plausible.
    pub fn from_env(env: &dyn EnvSource, cwd: &Path) -> Roots {
        Roots {
            project_root: resolve_project_root(env, cwd),
            extra_roots: parse_extra_roots(env),
        }
    }

    /// Canonicalizes a tool's `project` path and accepts it only inside the roots.
    pub fn resolve_project(&self, raw: &str) -> Res<PathBuf> {
        let canon = canonical_dir(raw)?;
        if self.project_root.is_none() && self.extra_roots.is_empty() {
            return Err(PalError::new(
                ErrCode::NoProjectRoot,
                format!(
                    "no project root is configured for this palette server, so {} cannot be \
                     checked against a boundary. This harness starts MCP servers outside the \
                     project. Fix: set SLATE_PROJECT_DIR for this server, or set \
                     PALETTE_EXTRA_ROOTS to your workspace root(s) (an OS path list) at \
                     registration time, for example by re-running install-mcp.sh with --roots.",
                    canon.display()
                ),
            ));
        }
        let inside = self
            .project_root
            .iter()
            .chain(self.extra_roots.iter())
            .any(|r| canon.starts_with(r));
        if !inside {
            let root = self
                .project_root
                .as_ref()
                .map(|r| r.display().to_string())
                .unwrap_or_else(|| "(no project root)".to_string());
            return Err(PalError::new(
                ErrCode::OutsideRoots,
                format!(
                    "project {} is outside the project root ({root}) and every extra root. \
                     Add its parent to the PALETTE_EXTRA_ROOTS environment variable (an OS \
                     path list of absolute paths) for this server.",
                    canon.display()
                ),
            ));
        }
        Ok(canon)
    }
}

/// Canonicalizes an absolute directory path.
pub fn canonical_dir(raw: &str) -> Res<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(PalError::invalid(
            "project is required: the absolute path of the project folder",
        ));
    }
    let p = Path::new(raw);
    if !p.is_absolute() {
        return Err(PalError::invalid(format!(
            "project must be an absolute path, got {raw:?}"
        )));
    }
    let canon = p.canonicalize().map_err(|e| {
        PalError::not_found(format!(
            "project {raw:?} cannot be resolved (does it exist?): {e}"
        ))
    })?;
    if !canon.is_dir() {
        return Err(PalError::invalid(format!(
            "project {} is not a directory",
            canon.display()
        )));
    }
    Ok(canon)
}

/// Rejects a write path that contains `..` or any symbolic link between `root` and
/// the path itself. Components that do not exist yet are fine.
pub fn check_write_path(root: &Path, path: &Path) -> Res<()> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| PalError::invariant(format!("{} is outside the project", path.display())))?;
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(name) => {
                cur.push(name);
                match std::fs::symlink_metadata(&cur) {
                    Ok(m) if m.file_type().is_symlink() => {
                        return Err(PalError::invariant(format!(
                            "{} is a symbolic link; palette does not write through symbolic links",
                            cur.display()
                        )));
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(PalError::io(&format!("inspecting {}", cur.display()), &e));
                    }
                }
            }
            Component::CurDir => {}
            _ => {
                return Err(PalError::invariant(format!(
                    "{} contains a `..` or root component",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &Path)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.as_os_str().to_os_string()))
                .collect(),
        )
    }

    #[test]
    fn project_inside_root_is_accepted() {
        let t = tempfile::tempdir().expect("tempdir");
        let root = t.path().canonicalize().expect("canon");
        let sub = root.join("proj");
        std::fs::create_dir(&sub).expect("mkdir");
        let roots = Roots::from_env(&env(&[("SLATE_PROJECT_DIR", &root)]), Path::new("/"));
        assert_eq!(roots.project_root.as_deref(), Some(root.as_path()));
        let got = roots
            .resolve_project(&sub.to_string_lossy())
            .expect("inside");
        assert_eq!(got, sub);
    }

    #[test]
    fn project_outside_roots_is_refused() {
        let a = tempfile::tempdir().expect("a");
        let b = tempfile::tempdir().expect("b");
        let roots = Roots::from_env(&env(&[("SLATE_PROJECT_DIR", a.path())]), Path::new("/"));
        let err = roots
            .resolve_project(&b.path().to_string_lossy())
            .expect_err("outside");
        assert_eq!(err.code, ErrCode::OutsideRoots);
        assert!(err.message.contains("PALETTE_EXTRA_ROOTS"));
    }

    #[test]
    fn extra_roots_widen_the_boundary() {
        let a = tempfile::tempdir().expect("a");
        let b = tempfile::tempdir().expect("b");
        let sep = if cfg!(windows) { ";" } else { ":" };
        let list = format!("{}{}{}", b.path().display(), sep, "/does/not/exist");
        let e = MapEnv(vec![
            (
                "SLATE_PROJECT_DIR".into(),
                a.path().as_os_str().to_os_string(),
            ),
            ("PALETTE_EXTRA_ROOTS".into(), OsString::from(list)),
        ]);
        let roots = Roots::from_env(&e, Path::new("/"));
        assert_eq!(roots.extra_roots.len(), 1);
        assert!(roots.resolve_project(&b.path().to_string_lossy()).is_ok());
    }

    #[test]
    fn no_root_fails_with_instructions() {
        let t = tempfile::tempdir().expect("t");
        let roots = Roots::from_env(&MapEnv::default(), Path::new("/"));
        assert!(roots.project_root.is_none());
        let err = roots
            .resolve_project(&t.path().to_string_lossy())
            .expect_err("none");
        assert_eq!(err.code, ErrCode::NoProjectRoot);
        assert!(err.message.contains("SLATE_PROJECT_DIR"));
        assert!(err.message.contains("PALETTE_EXTRA_ROOTS"));
    }

    #[test]
    fn env_priority_is_slate_then_agent_kit_then_claude() {
        let a = tempfile::tempdir().expect("a");
        let b = tempfile::tempdir().expect("b");
        let roots = Roots::from_env(
            &env(&[
                ("CLAUDE_PROJECT_DIR", b.path()),
                ("AGENT_KIT_PROJECT_DIR", a.path()),
            ]),
            Path::new("/"),
        );
        assert_eq!(
            roots.project_root,
            Some(a.path().canonicalize().expect("canon"))
        );
    }

    #[test]
    fn cwd_fallback_rejects_home_and_harness_homes() {
        let home = tempfile::tempdir().expect("home");
        let home_c = home.path().canonicalize().expect("canon");
        let claude = home_c.join(".claude");
        std::fs::create_dir(&claude).expect("mkdir");
        let e = env(&[("HOME", &home_c)]);
        assert!(Roots::from_env(&e, &home_c).project_root.is_none());
        assert!(Roots::from_env(&e, &claude).project_root.is_none());
        let proj = home_c.join("work");
        std::fs::create_dir(&proj).expect("mkdir");
        assert_eq!(Roots::from_env(&e, &proj).project_root, Some(proj));
        assert!(Roots::from_env(&e, Path::new("/")).project_root.is_none());
    }

    #[test]
    fn relative_and_missing_projects_are_rejected() {
        let roots = Roots::default();
        assert_eq!(
            roots.resolve_project("rel/path").expect_err("rel").code,
            ErrCode::InvalidParams
        );
        assert_eq!(
            roots.resolve_project("").expect_err("empty").code,
            ErrCode::InvalidParams
        );
        let missing = std::env::temp_dir().join("palette-definitely-missing-dir");
        assert_eq!(
            roots
                .resolve_project(&missing.to_string_lossy())
                .expect_err("missing")
                .code,
            ErrCode::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_paths_through_symlinks_are_rejected() {
        let t = tempfile::tempdir().expect("t");
        let root = t.path().canonicalize().expect("canon");
        let outside = tempfile::tempdir().expect("o");
        std::os::unix::fs::symlink(outside.path(), root.join("link")).expect("symlink");
        let err = check_write_path(&root, &root.join("link").join("x.rst")).expect_err("symlink");
        assert_eq!(err.code, ErrCode::InvariantViolation);
        assert!(check_write_path(&root, &root.join("new").join("x.rst")).is_ok());
        assert!(check_write_path(&root, &root.join("..").join("x.rst")).is_err());
    }
}
