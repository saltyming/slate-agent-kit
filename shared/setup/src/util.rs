//! Small file, time and line-ending helpers shared by the installer.
//!
//! Owns atomic file writes (with permission and symlink preservation), UTC
//! timestamp formatting without a date-time dependency, backup naming and
//! line-ending detection. It does not know about kits, manifests or harnesses.
//!
//! Main entry points: [`write_atomic`], [`backup_path`], [`utc_compact`],
//! [`utc_iso`], [`detect_eol`] and [`to_eol`].

use crate::error::{Error, IoContext, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Civil date and time in UTC.
struct Civil {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn civil_from_time(t: SystemTime) -> Civil {
    let secs = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    Civil {
        year: if m <= 2 { y + 1 } else { y },
        month: m,
        day: d,
        hour: (rem / 3_600) as u32,
        minute: (rem % 3_600 / 60) as u32,
        second: (rem % 60) as u32,
    }
}

/// Formats `t` as `YYYYMMDDTHHMMSSZ`, the suffix used in backup file names.
pub fn utc_compact(t: SystemTime) -> String {
    let c = civil_from_time(t);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// Formats `t` as an RFC 3339 UTC timestamp.
pub fn utc_iso(t: SystemTime) -> String {
    let c = civil_from_time(t);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// The current time as `YYYYMMDDTHHMMSSZ`.
pub fn now_compact() -> String {
    utc_compact(SystemTime::now())
}

/// The current time as an RFC 3339 UTC timestamp.
pub fn now_iso() -> String {
    utc_iso(SystemTime::now())
}

/// Returns `\r\n` when the text uses CRLF line endings on most lines, else `\n`.
pub fn detect_eol(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count();
    if crlf > 0 && crlf * 2 >= lf {
        "\r\n"
    } else {
        "\n"
    }
}

/// Rewrites every line ending in `text` to `eol`.
pub fn to_eol(text: &str, eol: &str) -> String {
    let normalized = text.replace("\r\n", "\n");
    if eol == "\n" {
        normalized
    } else {
        normalized.replace('\n', eol)
    }
}

/// Reads a file as UTF-8 text, returning `None` when it does not exist.
pub fn read_text_opt(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| Error::config(format!("{} is not valid UTF-8", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(format!("reading {}", path.display()), e)),
    }
}

/// Resolves a symlink at `path` so that writes replace its target and keep the link.
fn write_target(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

/// Writes `bytes` to `path` through a sibling temporary file and a rename.
///
/// Creates missing parent folders, keeps the permissions of an existing file
/// (configuration files may be private) and writes through a symlink instead of
/// replacing it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let target = write_target(path);
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
    }
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = target.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f = fs::File::create(&tmp).ctx(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)
            .ctx(|| format!("writing {}", tmp.display()))?;
        f.sync_all().ctx(|| format!("writing {}", tmp.display()))?;
        drop(f);
        if let Ok(meta) = fs::metadata(&target) {
            // Best effort: a failure to copy the mode must not block the write.
            let _ = fs::set_permissions(&tmp, meta.permissions());
        }
        fs::rename(&tmp, &target)
            .ctx(|| format!("replacing {} with {}", target.display(), tmp.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Returns `<path>.bak-<UTC timestamp>`, adding a counter when that name is taken.
pub fn backup_path(path: &Path) -> PathBuf {
    let base = format!("{}.bak-{}", path.display(), now_compact());
    let mut candidate = PathBuf::from(&base);
    let mut n = 1;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Copies `path` to a fresh backup name and returns that name.
pub fn backup_file(path: &Path) -> Result<PathBuf> {
    let dest = backup_path(path);
    fs::copy(path, &dest).ctx(|| format!("backing up {} to {}", path.display(), dest.display()))?;
    Ok(dest)
}

/// Returns true when `a` and `b` name the same path, comparing canonical forms when possible.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instant() {
        // 2026-09-29T10:15:30Z
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_790_676_930);
        assert_eq!(utc_compact(t), "20260929T101530Z");
        assert_eq!(utc_iso(t), "2026-09-29T10:15:30Z");
    }

    #[test]
    fn formats_leap_day_and_epoch() {
        assert_eq!(utc_iso(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_709_164_800);
        assert_eq!(utc_iso(t), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn eol_detection_and_conversion() {
        assert_eq!(detect_eol("a\r\nb\r\n"), "\r\n");
        assert_eq!(detect_eol("a\nb\n"), "\n");
        assert_eq!(detect_eol("single"), "\n");
        assert_eq!(to_eol("a\nb\r\nc", "\r\n"), "a\r\nb\r\nc");
        assert_eq!(to_eol("a\r\nb", "\n"), "a\nb");
    }

    #[test]
    fn atomic_write_creates_parents_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("b.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "two");
        let leftovers: Vec<_> = fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_mode_and_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        fs::write(&real, "x").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link.toml");
        symlink(&real, &link).unwrap();
        write_atomic(&link, b"y").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "y");
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn backup_names_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("CLAUDE.md");
        fs::write(&f, "a").unwrap();
        let b1 = backup_file(&f).unwrap();
        let b2 = backup_file(&f).unwrap();
        assert_ne!(b1, b2);
        assert!(b1.exists() && b2.exists());
    }
}
