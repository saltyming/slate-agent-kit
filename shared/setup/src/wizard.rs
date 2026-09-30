//! The ask phase: binaries mode, workspace roots, each prefs file and the custom rules folder.
//!
//! Owns the questions and their order (`[2/6] MCP servers`), the rule that an
//! explicit option answers its question, the conditions under which a question
//! is skipped, and the migration prompt for old-format prefs files. It never
//! changes files; it returns [`Choices`] for `plan`.
//!
//! Main entry points: [`ask`], [`Choices`], [`CustomChoice`] and [`parse_roots`].

use crate::binaries::Mode;
use crate::env::{Env, Harness};
use crate::error::{Error, Result};
use crate::kit::{Kit, PrefsState};
use crate::options::{Kind, Options};
use crate::prefs::{self, State, schema};
use crate::ui::{Question, Ui};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What to do with the custom rules folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomChoice {
    /// No folder is given; earlier copies stay.
    Keep,
    /// Copy the rules from this folder.
    Folder(PathBuf),
    /// Stop tracking the folder; earlier copies stay.
    Clear,
}

/// The answers of the ask phase.
#[derive(Debug, Clone)]
pub struct Choices {
    /// Where binaries come from.
    pub binaries: Mode,
    /// The slate checkout for `build`.
    pub slate_dir: Option<PathBuf>,
    /// Workspace roots.
    pub roots: Vec<String>,
    /// Answers per prefs file.
    pub prefs: BTreeMap<String, prefs::Input>,
    /// The custom rules folder.
    pub custom: CustomChoice,
}

/// Splits an OS path list into roots. Every root must be absolute; an empty text or `none` means no roots.
pub fn parse_roots(raw: &str) -> std::result::Result<Vec<String>, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for p in std::env::split_paths(raw) {
        if p.as_os_str().is_empty() {
            continue;
        }
        if !p.is_absolute() {
            return Err(format!("`{}` is not an absolute path", p.display()));
        }
        out.push(p.to_string_lossy().into_owned());
    }
    Ok(out)
}

fn path_separator() -> char {
    if cfg!(windows) { ';' } else { ':' }
}

enum Step {
    Binaries,
    Servers,
    Prefs(usize),
    CustomRules,
}

fn prefs_title(name: &str) -> &'static str {
    match name {
        "aside" => "aside: consultation from another model family",
        "dispatch" => "dispatch: handing execution steps to a backend",
        "subagent" => "subagents",
        "git" => "git",
        "comment" => "code comments",
        _ => "preferences",
    }
}

/// Runs the ask phase.
pub fn ask(
    env: &Env,
    ui: &mut Ui,
    kit: &Kit,
    opts: &Options,
    kind: Kind,
    states: &[PrefsState],
) -> Result<Choices> {
    let has_servers = !kit.servers().is_empty();
    // A step exists only when it asks something: an explicit option answers its question.
    let mut steps = Vec::new();
    if kind == Kind::Install && has_servers && opts.binaries.is_none() {
        steps.push(Step::Binaries);
    }
    if has_servers && opts.roots.is_none() {
        steps.push(Step::Servers);
    }
    for i in 0..states.len() {
        steps.push(Step::Prefs(i));
    }
    if opts.custom_rules.is_none() && env.get("CUSTOM_RULES_DIR").is_none() {
        steps.push(Step::CustomRules);
    }
    let total = steps.len();
    let vctx = kit.validation_ctx();

    let mut choices = Choices {
        binaries: if has_servers {
            opts.binaries.unwrap_or(Mode::Prebuilt)
        } else {
            Mode::Skip
        },
        slate_dir: opts.slate_dir.clone(),
        roots: Vec::new(),
        prefs: BTreeMap::new(),
        custom: CustomChoice::Keep,
    };
    if kind == Kind::Configure {
        // `configure` keeps the installed binaries.
        choices.binaries = Mode::Skip;
    }

    // Answers that come from options are taken even when their step is not shown.
    if !steps.iter().any(|s| matches!(s, Step::Servers)) && has_servers {
        choices.roots = ask_roots(ui, kit, opts)?;
    }
    if !steps.iter().any(|s| matches!(s, Step::CustomRules)) {
        choices.custom = ask_custom_rules(env, ui, kit, opts)?;
    }
    if !steps.iter().any(|s| matches!(s, Step::Binaries)) && has_servers && kind == Kind::Install {
        ask_binaries(ui, opts, &mut choices)?;
    }
    for (n, step) in steps.iter().enumerate() {
        let title = match step {
            Step::Binaries => "Binaries".to_string(),
            Step::Servers => "MCP servers".to_string(),
            Step::Prefs(i) => prefs_title(&states[*i].name).to_string(),
            Step::CustomRules => "Custom rules".to_string(),
        };
        if ui.interactive() {
            ui.step(n + 1, total, &title);
        }
        match step {
            Step::Binaries => ask_binaries(ui, opts, &mut choices)?,
            Step::Servers => choices.roots = ask_roots(ui, kit, opts)?,
            Step::Prefs(i) => {
                let st = &states[*i];
                let input = ask_prefs_file(ui, opts, st, &vctx)?;
                choices.prefs.insert(st.name.clone(), input);
            }
            Step::CustomRules => choices.custom = ask_custom_rules(env, ui, kit, opts)?,
        }
    }
    for file in opts.set.keys() {
        if !states.iter().any(|s| &s.name == file) {
            return Err(Error::usage(format!(
                "`--set {file}.*` names prefs this kit does not ship (it ships: {})",
                kit.payload.desc.prefs.join(", ")
            )));
        }
    }
    Ok(choices)
}

fn ask_binaries(ui: &mut Ui, opts: &Options, choices: &mut Choices) -> Result<()> {
    if opts.binaries.is_none() && ui.interactive() {
        let validate = |v: &str| -> std::result::Result<String, String> {
            Mode::parse(v)
                .map(|m| m.name().to_string())
                .ok_or_else(|| "choose prebuilt, build or skip".to_string())
        };
        let answer = ui.ask(&Question {
            prompt: "Where do the aside, dispatch and palette binaries come from?".into(),
            help: None,
            options: vec![
                (
                    "prebuilt".into(),
                    "download the release binaries and verify their checksums".into(),
                ),
                (
                    "build".into(),
                    "build them with cargo from a slate checkout".into(),
                ),
                (
                    "skip".into(),
                    "install no binaries and register no servers".into(),
                ),
            ],
            default: "prebuilt".into(),
            allow_custom: false,
            allow_clear: false,
            validate: &validate,
        });
        choices.binaries = Mode::parse(&answer).unwrap_or(Mode::Prebuilt);
    }
    if choices.binaries == Mode::Build && choices.slate_dir.is_none() && ui.interactive() {
        let dir = ui.ask_text("Slate checkout to build from", "");
        if !dir.is_empty() {
            choices.slate_dir = Some(PathBuf::from(dir));
        }
    }
    Ok(())
}

fn ask_roots(ui: &mut Ui, kit: &Kit, opts: &Options) -> Result<Vec<String>> {
    if let Some(raw) = &opts.roots {
        let roots = parse_roots(raw).map_err(|e| Error::usage(format!("--roots: {e}")))?;
        warn_about_no_roots(ui, kit.harness, &roots);
        return Ok(roots);
    }
    let sep = path_separator();
    let default = kit
        .prior
        .as_ref()
        .map(|m| m.roots.join(&sep.to_string()))
        .unwrap_or_default();
    loop {
        let raw = if ui.interactive() {
            ui.ask_text(
                &format!("Workspace roots for dispatch and palette, separated by '{sep}' (none: no roots)"),
                &default,
            )
        } else {
            default.clone()
        };
        let roots = match parse_roots(&raw) {
            Ok(r) => r,
            Err(e) if ui.interactive() => {
                ui.warn(&e);
                continue;
            }
            Err(e) => return Err(Error::usage(format!("workspace roots: {e}"))),
        };
        if roots.is_empty() && kit.harness == Harness::Kimi && ui.interactive() {
            warn_about_no_roots(ui, kit.harness, &roots);
            if !ui.ask_yes_no("Continue without workspace roots?", false) {
                continue;
            }
        } else {
            warn_about_no_roots(ui, kit.harness, &roots);
        }
        for r in &roots {
            if !Path::new(r).exists() && ui.interactive() {
                ui.warn(&format!("{r} does not exist yet"));
            }
        }
        return Ok(roots);
    }
}

fn warn_about_no_roots(ui: &mut Ui, harness: Harness, roots: &[String]) {
    if roots.is_empty() && harness == Harness::Kimi && !ui.interactive() {
        ui.warn("no workspace roots: Kimi starts plugin servers outside any project, so dispatch and palette will reject every project");
    } else if roots.is_empty() && harness == Harness::Kimi {
        ui.warn("Kimi starts plugin servers outside any project: without roots dispatch and palette will reject every project");
    }
}

fn ask_prefs_file(
    ui: &mut Ui,
    opts: &Options,
    st: &PrefsState,
    vctx: &schema::ValidationCtx,
) -> Result<prefs::Input> {
    let explicit = opts.set.get(&st.name).cloned().unwrap_or_default();
    let mut input = prefs::Input {
        migrate: true,
        values: explicit.clone(),
    };
    let current = prefs::current_values(
        &st.name,
        st.existing.as_deref(),
        &st.template,
        st.state,
        vctx,
    );
    let unset = prefs::treated_as_unset(
        &st.name,
        st.existing.as_deref(),
        &st.template,
        st.state,
        vctx,
    );
    let file_label = st
        .dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    if st.state == State::Legacy {
        if ui.interactive() {
            ui.line(&format!("  {file_label} was written by an earlier release. Migrating keeps your values and your Notes."));
            for setting in schema::for_file(&st.name) {
                if let Some(v) = current.get(setting.key).filter(|v| !v.is_empty()) {
                    ui.detail(&format!("{} = {v}", setting.key));
                }
            }
        }
        input.migrate = ui.ask_yes_no("Migrate it? The old file is saved as a .bak copy", true);
        if !input.migrate {
            return Ok(input);
        }
    }

    let reconfigure = match st.state {
        State::Missing => true,
        State::Current | State::Legacy => {
            ui.interactive()
                && ui.ask_yes_no(
                    &format!("Change the values in {file_label}? (otherwise it stays as it is)"),
                    false,
                )
        }
    };
    if !reconfigure {
        return Ok(input);
    }

    let mut backend = explicit
        .get("backend")
        .or_else(|| current.get("backend"))
        .cloned()
        .unwrap_or_default();
    let mut explained_kimi = false;
    for setting in schema::for_file(&st.name) {
        if explicit.contains_key(setting.key) {
            continue;
        }
        if st.name == "subagent"
            && vctx.harness == Harness::Kimi
            && matches!(setting.key, "model" | "effort")
            && vctx.kimi_models.as_ref().is_none_or(Vec::is_empty)
        {
            if !explained_kimi && ui.interactive() {
                ui.line("  Kimi Code has no [models] aliases in config.toml, so the secondary model cannot be set here; skipping.");
                explained_kimi = true;
            }
            continue;
        }
        if !schema::applies(setting, vctx, &backend) {
            continue;
        }
        let default = current.get(setting.key).cloned().unwrap_or_default();
        let validate = |v: &str| schema::validate(setting, vctx, v);
        let options = schema::menu(setting, vctx)
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                let d = schema::describe(setting, &v).to_string();
                (v, d)
            })
            .collect();
        let answer = ui.ask(&Question {
            prompt: setting.label.to_string(),
            help: None,
            options,
            default: default.clone(),
            allow_custom: schema::allows_custom(setting, vctx),
            allow_clear: schema::allows_blank(setting),
            validate: &validate,
        });
        if st.name == "aside" && setting.key == "backend" {
            backend = answer.clone();
        }
        // A value shown in place of one the file holds is written even when accepted as shown.
        if answer != default || unset.contains(&setting.key) {
            input.values.insert(setting.key.to_string(), answer);
        }
    }
    Ok(input)
}

fn ask_custom_rules(env: &Env, ui: &mut Ui, kit: &Kit, opts: &Options) -> Result<CustomChoice> {
    if let Some(p) = &opts.custom_rules {
        return Ok(
            if p.as_os_str().is_empty() || p.to_string_lossy().eq_ignore_ascii_case("none") {
                CustomChoice::Clear
            } else {
                CustomChoice::Folder(p.clone())
            },
        );
    }
    if let Some(v) = env.get("CUSTOM_RULES_DIR") {
        return Ok(CustomChoice::Folder(PathBuf::from(v)));
    }
    let prior = kit.prior.as_ref().and_then(|m| m.custom_rules_dir.clone());
    if !ui.interactive() {
        return Ok(match prior {
            Some(dir) if dir.is_dir() => CustomChoice::Folder(dir),
            Some(dir) => {
                ui.warn(&format!(
                    "the custom rules folder {} no longer exists; earlier copies stay",
                    dir.display()
                ));
                CustomChoice::Keep
            }
            None => CustomChoice::Keep,
        });
    }
    let shown = prior
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    loop {
        let answer = ui.ask_text(
            "Folder with your own rule files, *.md (Enter: none; 'none' stops using a folder)",
            &shown,
        );
        if answer.is_empty() {
            return Ok(CustomChoice::Keep);
        }
        if answer.eq_ignore_ascii_case("none") {
            return Ok(CustomChoice::Clear);
        }
        let path = PathBuf::from(&answer);
        if path.is_dir() {
            return Ok(CustomChoice::Folder(path));
        }
        ui.warn(&format!("{answer} is not a folder"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_split_by_the_os_separator_and_must_be_absolute() {
        let sep = path_separator();
        let (a, b) = if cfg!(windows) {
            ("C:\\w\\a", "D:\\w\\b")
        } else {
            ("/w/a", "/w/b")
        };
        let roots = parse_roots(&format!("{a}{sep}{b}")).unwrap();
        assert_eq!(roots, vec![a.to_string(), b.to_string()]);
        assert!(
            parse_roots("relative/dir")
                .unwrap_err()
                .contains("not an absolute path")
        );
        assert!(parse_roots("").unwrap().is_empty());
        assert!(parse_roots("none").unwrap().is_empty());
        assert!(parse_roots(&format!("{sep}{a}{sep}")).unwrap().len() == 1);
    }
}
