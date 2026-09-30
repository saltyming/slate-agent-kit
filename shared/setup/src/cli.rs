//! The command line: argument definitions and their conversion to [`Options`].
//!
//! Owns the `clap` definitions of `install`, `configure`, `uninstall` and `mcp`
//! and the checks that need no other module. It runs nothing.
//!
//! Main entry points: [`parse`] and [`Parsed`].

use crate::binaries::Mode;
use crate::env::Harness;
use crate::error::{Error, Result};
use crate::options::{Kind, Options};
use clap::{Args, Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

/// Installer for the slate agent kits.
#[derive(Parser, Debug)]
#[command(name = "slate-setup", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Full install or reinstall of a kit.
    Install(KitArgs),
    /// Prefs, custom rules, native configuration and server registration only.
    Configure(KitArgs),
    /// Reverse what install recorded.
    Uninstall(KitArgs),
    /// Binaries and server registration only, for one or more harnesses.
    Mcp(McpArgs),
}

#[derive(Args, Debug, Default)]
struct Common {
    /// The kit's dist/ folder (default: ./dist, else dist/ beside this program).
    #[arg(long, value_name = "DIR", overrides_with = "payload")]
    payload: Option<PathBuf>,
    /// The harness home (default: the harness's own variable, else its default folder).
    #[arg(long, value_name = "DIR", overrides_with = "home")]
    home: Option<PathBuf>,
    /// Where binaries go (default: ~/.local/bin).
    #[arg(long, value_name = "DIR", overrides_with = "bin_dir")]
    bin_dir: Option<PathBuf>,
    /// Where binaries come from.
    #[arg(long, value_name = "MODE", value_parser = ["prebuilt", "build", "skip"], overrides_with = "binaries")]
    binaries: Option<String>,
    /// The slate checkout to build from.
    #[arg(long, value_name = "DIR", overrides_with = "slate_dir")]
    slate_dir: Option<PathBuf>,
    /// Workspace roots for dispatch and palette, as an OS path list (':' or ';').
    #[arg(long, value_name = "PATHS", overrides_with = "roots")]
    roots: Option<String>,
    /// Set a prefs value: <file>.<key>=<value> (repeatable).
    #[arg(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// Take every answer from its current or default value; never ask.
    #[arg(long, short = 'y')]
    yes: bool,
    /// Print the summary and exit without changing anything.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args, Debug)]
struct KitArgs {
    #[command(flatten)]
    common: Common,
    /// A folder of your own *.md rule files; `none` stops using a folder.
    #[arg(long, value_name = "DIR", overrides_with = "custom_rules")]
    custom_rules: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct McpArgs {
    #[command(flatten)]
    common: Common,
    /// The harness to register with (repeatable): claude, codex or kimi.
    #[arg(long, value_name = "HARNESS", value_delimiter = ',')]
    harness: Vec<String>,
    /// Remove the registrations instead of installing.
    #[arg(long)]
    uninstall: bool,
    /// The slate release to download binaries from (default: the latest release).
    #[arg(long, value_name = "VERSION")]
    slate_version: Option<String>,
}

/// The parsed command line.
#[derive(Debug)]
pub struct Parsed {
    /// Which command runs.
    pub kind: Kind,
    /// Its options.
    pub options: Options,
}

fn from_common(c: Common) -> Result<Options> {
    let binaries = match c.binaries.as_deref() {
        Some(m) => Some(
            Mode::parse(m).ok_or_else(|| Error::usage(format!("unknown binaries mode `{m}`")))?,
        ),
        None => None,
    };
    let mut o = Options {
        payload: c.payload,
        home: c.home,
        bin_dir: c.bin_dir,
        binaries,
        slate_dir: c.slate_dir,
        roots: c.roots,
        yes: c.yes,
        dry_run: c.dry_run,
        ..Options::default()
    };
    for s in &c.set {
        o.add_set(s)?;
    }
    Ok(o)
}

/// Parses `args` (including the program name). `--help` and `--version` are reported as `Err(Ok(text))`.
pub fn parse<I: IntoIterator<Item = OsString>>(args: I) -> std::result::Result<Parsed, ParseExit> {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            return Err(ParseExit::Clap {
                text: e.render().to_string(),
                code,
            });
        }
    };
    let built = (|| -> Result<Parsed> {
        Ok(match cli.command {
            Cmd::Install(a) => Parsed {
                kind: Kind::Install,
                options: Options {
                    custom_rules: a.custom_rules,
                    ..from_common(a.common)?
                },
            },
            Cmd::Configure(a) => {
                let options = Options {
                    custom_rules: a.custom_rules,
                    ..from_common(a.common)?
                };
                if matches!(options.binaries, Some(Mode::Prebuilt | Mode::Build)) {
                    return Err(Error::usage(
                        "`configure` does not install binaries; use `slate-setup install`",
                    ));
                }
                Parsed {
                    kind: Kind::Configure,
                    options,
                }
            }
            Cmd::Uninstall(a) => Parsed {
                kind: Kind::Uninstall,
                options: Options {
                    custom_rules: a.custom_rules,
                    ..from_common(a.common)?
                },
            },
            Cmd::Mcp(a) => {
                let mut harnesses = Vec::new();
                for h in &a.harness {
                    harnesses.push(Harness::parse(h).ok_or_else(|| {
                        Error::usage(format!(
                            "unknown harness `{h}`; expected claude, codex or kimi"
                        ))
                    })?);
                }
                let mut options = from_common(a.common)?;
                options.harnesses = harnesses;
                options.uninstall = a.uninstall;
                options.slate_version = a.slate_version;
                Parsed {
                    kind: Kind::Mcp,
                    options,
                }
            }
        })
    })();
    built.map_err(ParseExit::Usage)
}

/// Why parsing did not produce a command.
#[derive(Debug)]
pub enum ParseExit {
    /// `clap` rendered help, a version or an error.
    Clap {
        /// The text to print.
        text: String,
        /// The exit code `clap` chose.
        code: i32,
    },
    /// A value that `clap` accepted was rejected.
    Usage(Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> std::result::Result<Parsed, ParseExit> {
        parse(
            std::iter::once("slate-setup")
                .chain(args.iter().copied())
                .map(OsString::from),
        )
    }

    #[test]
    fn install_options_are_parsed() {
        let parsed = p(&[
            "install",
            "--payload",
            "/k/dist",
            "--home",
            "/h",
            "--bin-dir",
            "/b",
            "--binaries",
            "skip",
            "--roots",
            "/w",
            "--set",
            "aside.level=auto",
            "--set",
            "git.signing=default",
            "--yes",
            "--dry-run",
            "--custom-rules",
            "/rules",
        ])
        .unwrap();
        assert_eq!(parsed.kind, Kind::Install);
        let o = parsed.options;
        assert_eq!(o.payload, Some(PathBuf::from("/k/dist")));
        assert_eq!(o.binaries, Some(Mode::Skip));
        assert!(o.yes && o.dry_run);
        assert_eq!(o.set["aside"]["level"], "auto");
        assert_eq!(o.set["git"]["signing"], "default");
        assert_eq!(o.custom_rules, Some(PathBuf::from("/rules")));
    }

    #[test]
    fn the_last_repetition_of_a_single_valued_option_wins() {
        let parsed = p(&["install", "--payload", "/one", "--payload", "/two"]).unwrap();
        assert_eq!(parsed.options.payload, Some(PathBuf::from("/two")));
    }

    #[test]
    fn mcp_takes_harness_lists() {
        let parsed = p(&[
            "mcp",
            "--harness",
            "claude,codex",
            "--harness",
            "kimi",
            "--uninstall",
        ])
        .unwrap();
        assert_eq!(
            parsed.options.harnesses,
            vec![Harness::Claude, Harness::Codex, Harness::Kimi]
        );
        assert!(parsed.options.uninstall);
        assert!(matches!(
            p(&["mcp", "--harness", "vim"]),
            Err(ParseExit::Usage(_))
        ));
    }

    #[test]
    fn bad_values_are_usage_errors() {
        assert!(matches!(
            p(&["install", "--set", "aside.level"]),
            Err(ParseExit::Usage(_))
        ));
        assert!(matches!(
            p(&["install", "--set", "nofile.key=1"]),
            Err(ParseExit::Usage(_))
        ));
        assert!(matches!(
            p(&["install", "--binaries", "maybe"]),
            Err(ParseExit::Clap { code: 2, .. })
        ));
        assert!(matches!(
            p(&["configure", "--binaries", "build"]),
            Err(ParseExit::Usage(_))
        ));
        assert!(p(&["configure", "--binaries", "skip"]).is_ok());
    }

    #[test]
    fn help_and_missing_commands_are_reported() {
        assert!(matches!(
            p(&["--help"]),
            Err(ParseExit::Clap { code: 0, .. })
        ));
        assert!(matches!(p(&[]), Err(ParseExit::Clap { .. })));
    }
}
