//! Obtaining the aside, dispatch and palette binaries and placing them in the binary folder.
//!
//! Owns the three binary modes: `prebuilt` (download from the slate release,
//! verify against `checksums.txt`), `build` (cargo build in a slate checkout) and
//! `skip`. Owns the safe replacement of a binary that a running server may hold.
//! It does not register servers; `harness::*` does.
//!
//! Main entry points: [`install`], [`Mode`], [`Fetch`], [`exe_name`] and
//! [`replace_binary`].

use crate::env::Env;
use crate::error::{Error, IoContext, Result};
use crate::ui::Ui;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where binaries come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Download release archives.
    Prebuilt,
    /// Build from a slate checkout with cargo.
    Build,
    /// Install nothing and register nothing.
    Skip,
}

impl Mode {
    /// Parses `prebuilt`, `build` or `skip`.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "prebuilt" => Some(Mode::Prebuilt),
            "build" => Some(Mode::Build),
            "skip" => Some(Mode::Skip),
            _ => None,
        }
    }

    /// The word the command line uses.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Prebuilt => "prebuilt",
            Mode::Build => "build",
            Mode::Skip => "skip",
        }
    }
}

/// What to install and from where.
#[derive(Debug, Clone)]
pub struct Request {
    /// The mode.
    pub mode: Mode,
    /// Binary names to install (`aside`, `dispatch`, `palette`).
    pub names: Vec<String>,
    /// Destination folder.
    pub bin_dir: PathBuf,
    /// Slate release the prebuilt binaries come from.
    pub slate_version: String,
    /// Slate checkout for `build`.
    pub slate_dir: Option<PathBuf>,
}

/// The result of installing binaries.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// Paths of the installed binaries.
    pub paths: Vec<PathBuf>,
    /// Facts the report should show (release fallback, signing warnings).
    pub notes: Vec<String>,
}

/// File name of binary `name` for `platform`.
pub fn exe_name(platform: &str, name: &str) -> String {
    if platform.contains("windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Fetches release assets. Implemented over HTTP; tests substitute an in-memory map.
pub trait Fetch {
    /// Returns the body of `url`, or `None` when the server answers 404.
    fn get(&self, url: &str) -> Result<Option<Vec<u8>>>;
}

/// [`Fetch`] over HTTPS with rustls.
pub struct HttpFetch {
    client: reqwest::blocking::Client,
}

impl HttpFetch {
    /// Creates a client with timeouts and the installer's user agent.
    pub fn new() -> Result<HttpFetch> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("slate-setup/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| Error::network(format!("cannot create the HTTP client: {e}")))?;
        Ok(HttpFetch { client })
    }
}

fn net_err(url: &str, e: impl std::fmt::Display) -> Error {
    Error::network(format!("downloading {url} failed: {e}"))
        .with_state("no binary was replaced")
        .with_fix("check the connection and re-run, or install with `--binaries build --slate-dir <slate checkout>`")
}

impl Fetch for HttpFetch {
    fn get(&self, url: &str) -> Result<Option<Vec<u8>>> {
        let resp = self.client.get(url).send().map_err(|e| net_err(url, e))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(net_err(url, format!("HTTP {}", resp.status())));
        }
        resp.bytes()
            .map(|b| Some(b.to_vec()))
            .map_err(|e| net_err(url, e))
    }
}

/// A [`Fetch`] for modes that download nothing.
pub struct NoFetch;

impl Fetch for NoFetch {
    fn get(&self, url: &str) -> Result<Option<Vec<u8>>> {
        Err(Error::network(format!(
            "nothing should be downloaded in this mode ({url})"
        )))
    }
}

/// The release download bases: the exact version, then the latest release.
pub fn release_bases(env: &Env, version: &str) -> (String, String) {
    let host = env
        .get("SLATE_RELEASE_BASE_URL")
        .unwrap_or("https://github.com")
        .trim_end_matches('/');
    let repo = env
        .get("SLATE_RELEASE_REPO")
        .unwrap_or("saltyming/slate-agent-kit");
    (
        format!("{host}/{repo}/releases/download/v{version}"),
        format!("{host}/{repo}/releases/latest/download"),
    )
}

/// Finds the SHA-256 of `file` in the text of a `checksums.txt` (`<hex>  <name>` lines).
pub fn find_checksum(checksums: &str, file: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        (name == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;

fn extract(archive: &[u8], wanted: &str, zip_format: bool) -> Result<Vec<u8>> {
    let missing = || Error::network(format!("the archive does not contain `{wanted}`"));
    if zip_format {
        let mut zip = zip::ZipArchive::new(Cursor::new(archive))
            .map_err(|e| Error::network(format!("cannot read the zip archive: {e}")))?;
        for i in 0..zip.len() {
            let mut file = zip
                .by_index(i)
                .map_err(|e| Error::network(format!("cannot read the zip archive: {e}")))?;
            let name = file
                .name()
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_string();
            if file.is_file() && name == wanted {
                let mut out = Vec::new();
                file.by_ref()
                    .take(MAX_BINARY_BYTES)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::network(format!("cannot extract `{wanted}`: {e}")))?;
                return Ok(out);
            }
        }
        Err(missing())
    } else {
        let gz = flate2::read::GzDecoder::new(archive);
        let mut tar = tar::Archive::new(gz);
        let entries = tar
            .entries()
            .map_err(|e| Error::network(format!("cannot read the tar archive: {e}")))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| Error::network(format!("cannot read the tar archive: {e}")))?;
            let path = entry
                .path()
                .map_err(|e| Error::network(format!("cannot read the tar archive: {e}")))?;
            let is_match = entry.header().entry_type().is_file()
                && path.file_name().is_some_and(|n| n == wanted);
            if is_match {
                let mut out = Vec::new();
                entry
                    .take(MAX_BINARY_BYTES)
                    .read_to_end(&mut out)
                    .map_err(|e| Error::network(format!("cannot extract `{wanted}`: {e}")))?;
                return Ok(out);
            }
        }
        Err(missing())
    }
}

struct Resolved {
    base: String,
    checksums: Option<String>,
    note: Option<String>,
    /// The first asset, when resolving already downloaded it as a probe.
    probed: Option<(String, Vec<u8>)>,
}

fn resolve(
    fetch: &dyn Fetch,
    versioned: &str,
    latest: &str,
    version: &str,
    probe_asset: &str,
) -> Result<Resolved> {
    if let Some(bytes) = fetch.get(&format!("{versioned}/checksums.txt"))? {
        return Ok(Resolved {
            base: versioned.to_string(),
            checksums: Some(String::from_utf8_lossy(&bytes).into_owned()),
            note: None,
            probed: None,
        });
    }
    // No checksums.txt: an existing release is recognised by having the asset itself.
    // The body is kept so the asset is not downloaded twice.
    if let Some(bytes) = fetch.get(&format!("{versioned}/{probe_asset}"))? {
        return Ok(Resolved {
            base: versioned.to_string(),
            checksums: None,
            note: None,
            probed: Some((probe_asset.to_string(), bytes)),
        });
    }
    let checksums = fetch
        .get(&format!("{latest}/checksums.txt"))?
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(Resolved {
        base: latest.to_string(),
        checksums,
        note: Some(format!(
            "slate release v{version} does not exist; the binaries come from the latest release"
        )),
        probed: None,
    })
}

/// Extracted binaries as `(name, bytes)`, and the notes gathered while fetching them.
pub type Downloaded = (Vec<(String, Vec<u8>)>, Vec<String>);

/// Downloads, verifies and extracts every binary of `req`, without touching the binary folder.
pub fn download_all(
    fetch: &dyn Fetch,
    env: &Env,
    req: &Request,
    ui: &mut Ui,
) -> Result<Downloaded> {
    let platform = env.platform();
    let zip_format = platform.contains("windows");
    let ext = if zip_format { "zip" } else { "tar.gz" };
    let (versioned, latest) = release_bases(env, &req.slate_version);
    let first = req.names.first().map(String::as_str).unwrap_or("aside");
    let resolved = if req.slate_version == "latest" {
        Resolved {
            checksums: fetch
                .get(&format!("{latest}/checksums.txt"))?
                .map(|b| String::from_utf8_lossy(&b).into_owned()),
            base: latest.clone(),
            note: None,
            probed: None,
        }
    } else {
        resolve(
            fetch,
            &versioned,
            &latest,
            &req.slate_version,
            &format!("{first}-{platform}.{ext}"),
        )?
    };
    let mut notes = Vec::new();
    if let Some(n) = &resolved.note {
        notes.push(n.clone());
    }
    if resolved.checksums.is_none() {
        notes.push("the release has no checksums.txt; the binaries were not verified".to_string());
    }
    let mut out = Vec::new();
    for name in &req.names {
        let asset = format!("{name}-{platform}.{ext}");
        let url = format!("{}/{asset}", resolved.base);
        ui.detail(&format!("downloading {asset}"));
        let cached = resolved
            .probed
            .as_ref()
            .filter(|(probed_asset, _)| probed_asset == &asset)
            .map(|(_, b)| b.clone());
        let bytes = match cached {
            Some(b) => b,
            None => fetch.get(&url)?.ok_or_else(|| {
                Error::network(format!("{url} does not exist"))
                    .with_state("no binary was replaced")
                    .with_fix("use `--binaries build --slate-dir <slate checkout>` or a slate release that ships this binary")
            })?,
        };
        if let Some(sums) = &resolved.checksums {
            match find_checksum(sums, &asset) {
                Some(expected) => {
                    let actual = sha256_hex(&bytes);
                    if expected != actual {
                        return Err(Error::network(format!(
                            "checksum mismatch for {asset}: expected {expected}, got {actual}"
                        ))
                        .with_state("no binary was replaced")
                        .with_fix("re-run the installer; if it repeats, the download is corrupt or tampered with"));
                    }
                }
                None => notes.push(format!(
                    "checksums.txt has no entry for {asset}; it was not verified"
                )),
            }
        }
        let binary = extract(&bytes, &exe_name(&platform, name), zip_format)?;
        out.push((name.clone(), binary));
    }
    Ok((out, notes))
}

/// Replaces `dest` with `bytes` so that a process running the old file keeps it.
///
/// The new file is written next to `dest` and renamed over it. On Windows a
/// running executable cannot be overwritten, so it is renamed away first. On
/// macOS the new file is signed ad hoc, because an unsigned binary may be killed
/// on launch. Returns a warning when signing failed.
pub fn replace_binary(dest: &Path, bytes: &[u8]) -> Result<Option<String>> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).ctx(|| format!("creating {}", parent.display()))?;
    }
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dest.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).ctx(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))
            .ctx(|| format!("making {} executable", tmp.display()))?;
    }
    let mut warning = None;
    if cfg!(target_os = "macos") {
        let signed = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&tmp)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !matches!(signed, Ok(s) if s.success()) {
            warning = Some(format!(
                "ad-hoc codesign failed for {name}; macOS may kill an unsigned MCP binary on launch"
            ));
        }
    }
    if let Err(first) = fs::rename(&tmp, dest) {
        let aside = dest.with_file_name(format!("{name}.old-{}", std::process::id()));
        let moved = dest.exists() && fs::rename(dest, &aside).is_ok();
        if moved && fs::rename(&tmp, dest).is_ok() {
            let _ = fs::remove_file(&aside);
        } else {
            if moved {
                let _ = fs::rename(&aside, dest);
            }
            let _ = fs::remove_file(&tmp);
            return Err(Error::io(
                format!(
                    "replacing {} (is the server running and locked?)",
                    dest.display()
                ),
                first,
            ));
        }
    }
    Ok(warning)
}

fn run_build(
    env: &Env,
    req: &Request,
    slate_dir: &Path,
    ui: &mut Ui,
) -> Result<Vec<(String, Vec<u8>)>> {
    let cargo = env.get("CARGO_BIN").unwrap_or("cargo");
    let mut cmd = env.command(cargo);
    cmd.current_dir(slate_dir).args(["build", "--release"]);
    for n in &req.names {
        cmd.args(["-p", n]);
    }
    ui.detail(&format!("cargo build --release in {}", slate_dir.display()));
    let status = cmd.status().map_err(|e| {
        Error::command(format!("cannot run `{cargo}`: {e}"))
            .with_state("no binary was replaced")
            .with_fix("install a Rust toolchain, or use `--binaries prebuilt`")
    })?;
    if !status.success() {
        return Err(Error::command(format!(
            "`{cargo} build --release` failed in {}",
            slate_dir.display()
        ))
        .with_state("no binary was replaced")
        .with_fix("fix the build error above and re-run, or use `--binaries prebuilt`"));
    }
    let target = env
        .get("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| slate_dir.join("target"));
    let platform = env.platform();
    let mut out = Vec::new();
    for n in &req.names {
        let built = target.join("release").join(exe_name(&platform, n));
        let bytes = fs::read(&built).map_err(|e| {
            Error::io(format!("reading the built binary {}", built.display()), e)
                .with_fix("check that the slate checkout builds this package")
        })?;
        out.push((n.clone(), bytes));
    }
    Ok(out)
}

/// Installs the binaries of `req` into its binary folder.
pub fn install(env: &Env, ui: &mut Ui, req: &Request, fetch: &dyn Fetch) -> Result<Installed> {
    let mut installed = Installed::default();
    let binaries = match req.mode {
        Mode::Skip => return Ok(installed),
        Mode::Prebuilt => {
            let (b, notes) = download_all(fetch, env, req, ui)?;
            installed.notes.extend(notes);
            b
        }
        Mode::Build => {
            let dir = req.slate_dir.as_deref().ok_or_else(|| {
                Error::usage("`--binaries build` needs `--slate-dir <slate checkout>`")
            })?;
            run_build(env, req, dir, ui)?
        }
    };
    let platform = env.platform();
    for (name, bytes) in binaries {
        let dest = req.bin_dir.join(exe_name(&platform, &name));
        if let Some(w) = replace_binary(&dest, &bytes)? {
            installed.notes.push(w);
        }
        installed.paths.push(dest);
    }
    Ok(installed)
}

/// The path binary `name` has in `bin_dir`.
pub fn binary_path(env: &Env, bin_dir: &Path, name: &str) -> PathBuf {
    bin_dir.join(exe_name(&env.platform(), name))
}

/// Runs `<palette> --read-only-tools` and returns the tool names it prints, one per line.
pub fn read_only_tools(env: &Env, palette: &Path) -> Result<Vec<String>> {
    let output = env
        .command(palette)
        .arg("--read-only-tools")
        .output()
        .map_err(|e| Error::command(format!("cannot run {}: {e}", palette.display())))?;
    if !output.status.success() {
        return Err(Error::command(format!(
            "`{} --read-only-tools` exited with {}",
            palette.display(),
            output.status
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut tools = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        if !line
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::command(format!(
                "`palette --read-only-tools` printed `{line}`, which is not a tool name"
            )));
        }
        if !tools.iter().any(|t| t == line) {
            tools.push(line.to_string());
        }
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Ui, new_sink};
    use std::collections::HashMap;
    use std::io::Write;

    struct MapFetch(HashMap<String, Vec<u8>>, std::cell::RefCell<Vec<String>>);

    impl MapFetch {
        fn new(files: HashMap<String, Vec<u8>>) -> Self {
            MapFetch(files, std::cell::RefCell::default())
        }
    }

    impl Fetch for MapFetch {
        fn get(&self, url: &str) -> Result<Option<Vec<u8>>> {
            self.1.borrow_mut().push(url.to_string());
            Ok(self.0.get(url).cloned())
        }
    }

    fn targz(name: &str, content: &[u8]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, name, content).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn zipped(name: &str, content: &[u8]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(content).unwrap();
        w.finish().unwrap().into_inner()
    }

    fn env_for(platform: &str) -> Env {
        let mut env = Env::for_home("/h");
        env.set_var("SLATE_PLATFORM", platform);
        env.set_var("SLATE_RELEASE_BASE_URL", "http://mirror");
        env
    }

    fn request(dir: &Path) -> Request {
        Request {
            mode: Mode::Prebuilt,
            names: vec!["aside".into(), "dispatch".into(), "palette".into()],
            bin_dir: dir.to_path_buf(),
            slate_version: "0.7.0".into(),
            slate_dir: None,
        }
    }

    fn release(platform: &str, base: &str, zip_format: bool, with_sums: bool) -> MapFetch {
        let mut m = HashMap::new();
        let mut sums = String::new();
        for name in ["aside", "dispatch", "palette"] {
            let exe = exe_name(platform, name);
            let body = format!("binary {name}").into_bytes();
            let (asset, bytes) = if zip_format {
                (format!("{name}-{platform}.zip"), zipped(&exe, &body))
            } else {
                (format!("{name}-{platform}.tar.gz"), targz(&exe, &body))
            };
            sums.push_str(&format!("{}  {asset}\n", sha256_hex(&bytes)));
            m.insert(format!("{base}/{asset}"), bytes);
        }
        if with_sums {
            m.insert(format!("{base}/checksums.txt"), sums.into_bytes());
        }
        MapFetch::new(m)
    }

    #[test]
    fn checksum_lines_parse_both_star_and_plain_forms() {
        let h = "a".repeat(64);
        let text = format!("{h}  aside-x.tar.gz\n{h} *dispatch-x.tar.gz\nshort  nope\n");
        assert_eq!(find_checksum(&text, "aside-x.tar.gz"), Some(h.clone()));
        assert_eq!(find_checksum(&text, "dispatch-x.tar.gz"), Some(h));
        assert_eq!(find_checksum(&text, "nope"), None);
        assert_eq!(find_checksum(&text, "absent"), None);
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn prebuilt_tar_gz_installs_into_the_bin_dir() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "x86_64-unknown-linux-musl";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let fetch = release(platform, &versioned, false, true);
        let mut ui = Ui::captured(new_sink());
        let got = install(&env, &mut ui, &request(dir.path()), &fetch).unwrap();
        assert_eq!(got.paths.len(), 3);
        assert!(
            got.notes.iter().all(|n| !n.contains("checksums")),
            "{:?}",
            got.notes
        );
        assert_eq!(
            fs::read(dir.path().join("dispatch")).unwrap(),
            b"binary dispatch"
        );
        assert_eq!(
            versioned,
            "http://mirror/saltyming/slate-agent-kit/releases/download/v0.7.0"
        );
    }

    #[test]
    fn prebuilt_zip_installs_exe_names() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "x86_64-pc-windows-msvc";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let fetch = release(platform, &versioned, true, true);
        let mut ui = Ui::captured(new_sink());
        install(&env, &mut ui, &request(dir.path()), &fetch).unwrap();
        assert_eq!(
            fs::read(dir.path().join("aside.exe")).unwrap(),
            b"binary aside"
        );
        assert!(dir.path().join("palette.exe").exists());
    }

    #[test]
    fn a_checksum_mismatch_aborts_before_any_binary_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("aside"), "old aside").unwrap();
        let platform = "x86_64-unknown-linux-gnu";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let mut fetch = release(platform, &versioned, false, true);
        // Corrupt the second archive after its checksum was recorded.
        let key = format!("{versioned}/dispatch-{platform}.tar.gz");
        fetch.0.insert(key, targz("dispatch", b"tampered"));
        let mut ui = Ui::captured(new_sink());
        let err = install(&env, &mut ui, &request(dir.path()), &fetch).unwrap_err();
        assert!(
            err.to_string().contains("checksum mismatch for dispatch-"),
            "{err}"
        );
        assert_eq!(fs::read(dir.path().join("aside")).unwrap(), b"old aside");
        assert!(!dir.path().join("dispatch").exists());
        assert!(err.state.unwrap().contains("no binary was replaced"));
    }

    #[test]
    fn a_missing_checksum_file_warns_but_installs() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "aarch64-apple-darwin";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let fetch = release(platform, &versioned, false, false);
        let mut ui = Ui::captured(new_sink());
        let got = install(&env, &mut ui, &request(dir.path()), &fetch).unwrap();
        assert!(
            got.notes.iter().any(|n| n.contains("no checksums.txt")),
            "{:?}",
            got.notes
        );
        assert!(dir.path().join("aside").exists());
    }

    #[test]
    fn a_release_without_checksums_downloads_each_asset_once() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "x86_64-unknown-linux-gnu";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let fetch = release(platform, &versioned, false, false);
        let mut ui = Ui::captured(new_sink());
        install(&env, &mut ui, &request(dir.path()), &fetch).unwrap();
        let calls = fetch.1.borrow();
        for name in ["aside", "dispatch", "palette"] {
            let asset = format!("{versioned}/{name}-{platform}.tar.gz");
            assert_eq!(
                calls.iter().filter(|c| **c == asset).count(),
                1,
                "{asset} in {calls:?}"
            );
        }
    }

    #[test]
    fn a_missing_release_falls_back_to_the_latest_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "x86_64-unknown-linux-gnu";
        let env = env_for(platform);
        let (_, latest) = release_bases(&env, "0.7.0");
        let fetch = release(platform, &latest, false, true);
        let mut ui = Ui::captured(new_sink());
        let got = install(&env, &mut ui, &request(dir.path()), &fetch).unwrap();
        assert!(
            got.notes
                .iter()
                .any(|n| n.contains("v0.7.0 does not exist") && n.contains("latest")),
            "{:?}",
            got.notes
        );
        assert!(dir.path().join("palette").exists());
    }

    #[test]
    fn an_existing_release_without_a_palette_asset_is_an_error_not_a_silent_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let platform = "x86_64-unknown-linux-gnu";
        let env = env_for(platform);
        let (versioned, _) = release_bases(&env, "0.7.0");
        let mut fetch = release(platform, &versioned, false, true);
        fetch
            .0
            .remove(&format!("{versioned}/palette-{platform}.tar.gz"));
        let mut ui = Ui::captured(new_sink());
        let err = install(&env, &mut ui, &request(dir.path()), &fetch).unwrap_err();
        assert!(err.to_string().contains("palette-"), "{err}");
    }

    #[test]
    fn archives_without_the_binary_are_rejected() {
        let e = extract(&targz("other", b"x"), "aside", false).unwrap_err();
        assert!(e.to_string().contains("does not contain `aside`"));
        assert!(extract(b"not an archive", "aside", false).is_err());
        assert!(extract(b"not an archive", "aside.exe", true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_binary_keeps_the_old_inode_for_running_processes() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("aside");
        fs::write(&dest, "old").unwrap();
        let held = fs::File::open(&dest).unwrap();
        let old_inode = held.metadata().unwrap().ino();
        replace_binary(&dest, b"new").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert_ne!(fs::metadata(&dest).unwrap().ino(), old_inode);
        let mut old = String::new();
        let mut held = held;
        held.read_to_string(&mut old).unwrap();
        assert_eq!(old, "old");
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temporary file is left: {names:?}");
    }

    #[test]
    fn skip_installs_nothing_and_build_requires_a_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_for("x86_64-unknown-linux-gnu");
        let mut ui = Ui::captured(new_sink());
        let mut req = request(dir.path());
        req.mode = Mode::Skip;
        let got = install(&env, &mut ui, &req, &MapFetch::new(HashMap::new())).unwrap();
        assert!(got.paths.is_empty());
        req.mode = Mode::Build;
        let err = install(&env, &mut ui, &req, &MapFetch::new(HashMap::new())).unwrap_err();
        assert!(err.to_string().contains("--slate-dir"));
    }

    #[test]
    fn exe_suffix_follows_the_platform() {
        assert_eq!(exe_name("x86_64-pc-windows-msvc", "aside"), "aside.exe");
        assert_eq!(exe_name("aarch64-apple-darwin", "aside"), "aside");
    }
}
