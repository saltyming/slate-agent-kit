//! The settings of the five prefs files: headings, allowed values and validation.
//!
//! Owns the single table that maps `--set <file>.<key>` keys to the Markdown
//! headings the templates must carry, and the rules for which values each
//! setting accepts. It does not read or write files; `doc` does that, and the
//! wizard asks the questions this table describes.
//!
//! Main entry points: [`SETTINGS`], [`find`], [`for_file`], [`validate`],
//! [`describe`] and [`ValidationCtx`].

use crate::env::Harness;

/// Which values a setting accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Exactly one of the listed values.
    Enum(&'static [&'static str]),
    /// One of the listed values, or blank.
    EnumOrBlank(&'static [&'static str]),
    /// One of the listed values, or free single-line text.
    EnumOrText(&'static [&'static str]),
    /// Free single-line text, possibly blank.
    Text,
    /// A comma-separated list, possibly blank.
    List,
    /// The subagent default model; validated per harness.
    SubagentModel,
    /// The subagent default effort; valid values depend on the harness.
    SubagentEffort,
}

/// One setting of a prefs file.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Prefs file name without the `-prefs` suffix.
    pub file: &'static str,
    /// Key inside the file, such as `level` or `codex.model`.
    pub key: &'static str,
    /// The `## <heading>` that carries the value line.
    pub heading: &'static str,
    /// Accepted values.
    pub kind: Kind,
    /// Backend the setting belongs to, for per-backend aside settings.
    pub backend: Option<&'static str>,
    /// Question text for the wizard.
    pub label: &'static str,
}

/// The action levels.
pub const LEVELS: &[&str] = &["on-request", "suggest", "auto"];
/// Reasoning-effort values for consultation and dispatch backends.
pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Aside backends.
pub const ASIDE_BACKENDS: &[&str] = &["codex", "claude"];
/// Dispatch backends.
pub const DISPATCH_BACKENDS: &[&str] = &["codex", "opencode", "claude"];
/// Codex subagent efforts.
pub const CODEX_SUBAGENT_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
/// Kimi subagent efforts.
pub const KIMI_SUBAGENT_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// Claude model aliases accepted for the subagent model.
pub const CLAUDE_MODEL_ALIASES: &[&str] = &["sonnet", "opus", "haiku", "fable", "inherit"];

const fn s(
    file: &'static str,
    key: &'static str,
    heading: &'static str,
    kind: Kind,
    backend: Option<&'static str>,
    label: &'static str,
) -> Setting {
    Setting {
        file,
        key,
        heading,
        kind,
        backend,
        label,
    }
}

/// Every setting, in the order the wizard asks them.
pub const SETTINGS: &[Setting] = &[
    s(
        "aside",
        "level",
        "Level",
        Kind::Enum(LEVELS),
        None,
        "How freely the agent consults another model family",
    ),
    s(
        "aside",
        "backend",
        "Backend",
        Kind::Enum(ASIDE_BACKENDS),
        None,
        "Consultation backend",
    ),
    s(
        "aside",
        "codex.model",
        "Codex model",
        Kind::Text,
        Some("codex"),
        "codex model (blank: the CLI default)",
    ),
    s(
        "aside",
        "codex.effort",
        "Codex reasoning effort",
        Kind::EnumOrBlank(EFFORTS),
        Some("codex"),
        "codex reasoning effort",
    ),
    s(
        "aside",
        "codex.fallback",
        "Codex model fallback",
        Kind::List,
        Some("codex"),
        "codex fallback models, comma-separated (blank: none)",
    ),
    s(
        "aside",
        "claude.model",
        "Claude model",
        Kind::Text,
        Some("claude"),
        "claude model (blank: the CLI default)",
    ),
    s(
        "aside",
        "claude.effort",
        "Claude reasoning effort",
        Kind::EnumOrBlank(EFFORTS),
        Some("claude"),
        "claude reasoning effort",
    ),
    s(
        "aside",
        "claude.fallback",
        "Claude model fallback",
        Kind::List,
        Some("claude"),
        "claude fallback models, comma-separated (blank: none)",
    ),
    s(
        "dispatch",
        "level",
        "Level",
        Kind::Enum(LEVELS),
        None,
        "How freely the agent hands execution steps to a backend",
    ),
    s(
        "dispatch",
        "backend",
        "Backend",
        Kind::Enum(DISPATCH_BACKENDS),
        None,
        "Dispatch backend",
    ),
    s(
        "dispatch",
        "model",
        "Model",
        Kind::Text,
        None,
        "Model (blank: the backend default)",
    ),
    s(
        "dispatch",
        "effort",
        "Reasoning effort",
        Kind::EnumOrBlank(EFFORTS),
        None,
        "Reasoning effort",
    ),
    s(
        "dispatch",
        "fallback",
        "Model fallback",
        Kind::List,
        None,
        "Fallback models, comma-separated (blank: none)",
    ),
    s(
        "subagent",
        "level",
        "Level",
        Kind::Enum(LEVELS),
        None,
        "How freely the agent delegates to subagents",
    ),
    s(
        "subagent",
        "model",
        "Default model",
        Kind::SubagentModel,
        None,
        "Default subagent model (blank: the harness default)",
    ),
    s(
        "subagent",
        "effort",
        "Reasoning effort",
        Kind::SubagentEffort,
        None,
        "Default subagent reasoning effort",
    ),
    s(
        "git",
        "signing",
        "Commit signing",
        Kind::Enum(&["default", "no-gpg-sign", "unset"]),
        None,
        "Commit signing",
    ),
    s(
        "git",
        "attribution",
        "Model attribution",
        Kind::Enum(&["on", "off", "unset"]),
        None,
        "Model attribution in commits and PRs",
    ),
    s(
        "git",
        "commit-format",
        "Commit message format",
        Kind::EnumOrText(&["conventional", "repository", "unset"]),
        None,
        "Commit message format",
    ),
    s(
        "git",
        "pr-body",
        "PR body format",
        Kind::EnumOrText(&["summary-test-plan", "repository", "unset"]),
        None,
        "Pull request body format",
    ),
    s(
        "git",
        "branch-naming",
        "Branch naming",
        Kind::EnumOrText(&["descriptive", "repository", "unset"]),
        None,
        "Branch naming",
    ),
    s(
        "comment",
        "headers",
        "File headers",
        Kind::EnumOrText(&["repository", "structured"]),
        None,
        "File headers",
    ),
    s(
        "comment",
        "language",
        "Comment language",
        Kind::EnumOrText(&["repository", "english", "korean"]),
        None,
        "Comment language",
    ),
    s(
        "comment",
        "doc-comments",
        "Doc comments",
        Kind::EnumOrText(&["repository", "public-api"]),
        None,
        "Doc comments",
    ),
];

/// Looks up a setting by `<file>.<key>`.
pub fn find(full_key: &str) -> Option<&'static Setting> {
    let (file, key) = full_key.split_once('.')?;
    SETTINGS.iter().find(|s| s.file == file && s.key == key)
}

/// The settings of one prefs file, in question order.
pub fn for_file(file: &str) -> impl Iterator<Item = &'static Setting> + '_ {
    SETTINGS.iter().filter(move |s| s.file == file)
}

/// Facts about the target that some validations need.
#[derive(Debug, Clone)]
pub struct ValidationCtx {
    /// Target harness.
    pub harness: Harness,
    /// Aliases defined under `[models]` of the Kimi configuration, when known.
    pub kimi_models: Option<Vec<String>>,
}

impl ValidationCtx {
    /// A context for `harness` with no Kimi model list.
    pub fn new(harness: Harness) -> Self {
        ValidationCtx {
            harness,
            kimi_models: None,
        }
    }
}

/// The values the wizard offers as a menu for `setting`, if it has a fixed set.
pub fn menu(setting: &Setting, ctx: &ValidationCtx) -> Option<Vec<String>> {
    let owned = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match setting.kind {
        Kind::Enum(v) | Kind::EnumOrBlank(v) | Kind::EnumOrText(v) => Some(owned(v)),
        Kind::SubagentEffort => match ctx.harness {
            Harness::Claude => None,
            Harness::Codex => Some(owned(CODEX_SUBAGENT_EFFORTS)),
            Harness::Kimi => Some(owned(KIMI_SUBAGENT_EFFORTS)),
        },
        Kind::SubagentModel => match ctx.harness {
            Harness::Claude => Some(owned(CLAUDE_MODEL_ALIASES)),
            Harness::Kimi => ctx.kimi_models.clone(),
            Harness::Codex => None,
        },
        _ => None,
    }
}

/// True when a blank answer is a valid value for `setting`.
pub fn allows_blank(setting: &Setting) -> bool {
    matches!(
        setting.kind,
        Kind::EnumOrBlank(_) | Kind::Text | Kind::List | Kind::SubagentModel | Kind::SubagentEffort
    )
}

/// True when the wizard accepts a typed value that is not in the menu.
pub fn allows_custom(setting: &Setting, ctx: &ValidationCtx) -> bool {
    match setting.kind {
        Kind::EnumOrText(_) | Kind::Text | Kind::List => true,
        Kind::SubagentModel => ctx.harness != Harness::Kimi,
        _ => false,
    }
}

/// Short explanation of a menu value, or an empty string.
pub fn describe(setting: &Setting, value: &str) -> &'static str {
    match (setting.key, value) {
        ("level", "on-request") => "only when you ask",
        ("level", "suggest") => "the agent proposes it in one line and waits",
        ("level", "auto") => {
            "the agent uses it when it judges it worthwhile and says so in one line"
        }
        ("signing", "default") => "leave git's own configuration alone",
        ("signing", "no-gpg-sign") => "pass --no-gpg-sign on commits",
        ("attribution", "on") => "keep the harness's co-author trailer and PR footer",
        ("attribution", "off") => "add no attribution",
        ("commit-format", "conventional") => "<type>(<area>): <subject>",
        ("commit-format", "repository") => "match the repository's own log",
        ("pr-body", "summary-test-plan") => "a Summary list and a Test plan checklist",
        ("pr-body", "repository") => "follow the repository's template",
        ("branch-naming", "descriptive") => "feat/..., fix/...",
        ("branch-naming", "repository") => "follow the repository",
        (_, "unset") => "ask when the agent first needs it",
        ("headers", "repository") => "only where the repository has them",
        ("headers", "structured") => "a structured header in every new file",
        ("language", "repository") => "the language of the surrounding comments",
        ("doc-comments", "repository") => "as much as the neighbouring code",
        ("doc-comments", "public-api") => "a doc comment on every public item",
        _ => "",
    }
}

fn one_of(value: &str, allowed: &[&str]) -> std::result::Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "`{value}` is not valid; allowed: {}",
            allowed.join(", ")
        ))
    }
}

/// Validates `value` for `setting`. Returns the value to write, or a message naming the allowed values.
pub fn validate(
    setting: &Setting,
    ctx: &ValidationCtx,
    value: &str,
) -> std::result::Result<String, String> {
    let value = value.trim();
    if value.contains(['\n', '\r']) {
        return Err("the value must be a single line".into());
    }
    if value.is_empty() && !allows_blank(setting) {
        return Err(match setting.kind {
            Kind::Enum(v) | Kind::EnumOrText(v) => {
                format!("a value is required; allowed: {}", v.join(", "))
            }
            _ => "a value is required".into(),
        });
    }
    match setting.kind {
        Kind::Enum(v) => one_of(value, v)?,
        Kind::EnumOrBlank(v) => {
            if !value.is_empty() {
                one_of(value, v)?;
            }
        }
        Kind::EnumOrText(_) | Kind::Text => {}
        Kind::List => {
            if !value.is_empty() && value.split(',').any(|p| p.trim().is_empty()) {
                return Err("the list has an empty entry".into());
            }
        }
        Kind::SubagentModel => validate_subagent_model(ctx, value)?,
        Kind::SubagentEffort => validate_subagent_effort(ctx, value)?,
    }
    Ok(value.to_string())
}

/// Validates the subagent default model for the target harness.
pub fn validate_subagent_model(
    ctx: &ValidationCtx,
    value: &str,
) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    match ctx.harness {
        Harness::Claude => {
            if CLAUDE_MODEL_ALIASES.contains(&value)
                || (value.starts_with("claude-") && value.len() > "claude-".len() && !value.contains(char::is_whitespace))
            {
                Ok(())
            } else {
                Err(format!(
                    "`{value}` is not a Claude Code model; use {} or an identifier starting with `claude-`",
                    CLAUDE_MODEL_ALIASES.join(", ")
                ))
            }
        }
        Harness::Codex => {
            if value.contains(char::is_whitespace) {
                Err("a Codex model name has no spaces".into())
            } else {
                Ok(())
            }
        }
        Harness::Kimi => match &ctx.kimi_models {
            Some(models) if models.iter().any(|m| m == value) => Ok(()),
            Some(models) if !models.is_empty() => Err(format!(
                "`{value}` is not an alias under [models] in the Kimi configuration; available: {}",
                models.join(", ")
            )),
            _ => Err("the Kimi configuration defines no [models] aliases, so a secondary model cannot be set".into()),
        },
    }
}

/// Validates the subagent default effort for the target harness.
pub fn validate_subagent_effort(
    ctx: &ValidationCtx,
    value: &str,
) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    match ctx.harness {
        Harness::Claude => Err("the subagent effort is not configurable in Claude Code".into()),
        Harness::Codex => one_of(value, CODEX_SUBAGENT_EFFORTS),
        Harness::Kimi => one_of(value, KIMI_SUBAGENT_EFFORTS),
    }
}

/// True when the wizard should ask `setting`, given the answers so far.
///
/// `backend` is the aside backend chosen in this run (or already in the file).
pub fn applies(setting: &Setting, ctx: &ValidationCtx, aside_backend: &str) -> bool {
    if let Some(b) = setting.backend {
        return b == aside_backend;
    }
    if setting.kind == Kind::SubagentEffort {
        return ctx.harness != Harness::Claude;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(h: Harness) -> ValidationCtx {
        ValidationCtx::new(h)
    }

    #[test]
    fn keys_are_unique_and_findable() {
        let mut seen = std::collections::BTreeSet::new();
        for s in SETTINGS {
            assert!(
                seen.insert(format!("{}.{}", s.file, s.key)),
                "duplicate {}.{}",
                s.file,
                s.key
            );
            assert!(find(&format!("{}.{}", s.file, s.key)).is_some());
        }
        assert!(find("aside.nope").is_none());
        assert!(find("nofile").is_none());
        assert_eq!(for_file("dispatch").count(), 5);
    }

    #[test]
    fn levels_reject_unknown_values_with_the_allowed_list() {
        let level = find("aside.level").unwrap();
        assert_eq!(
            validate(level, &ctx(Harness::Claude), " auto ").unwrap(),
            "auto"
        );
        let err = validate(level, &ctx(Harness::Claude), "always").unwrap_err();
        assert!(err.contains("on-request, suggest, auto"), "{err}");
        assert!(validate(level, &ctx(Harness::Claude), "").is_err());
    }

    #[test]
    fn effort_and_list_rules() {
        let effort = find("aside.codex.effort").unwrap();
        assert_eq!(validate(effort, &ctx(Harness::Codex), "").unwrap(), "");
        assert!(validate(effort, &ctx(Harness::Codex), "ultra").is_err());
        let fb = find("aside.codex.fallback").unwrap();
        assert!(validate(fb, &ctx(Harness::Codex), "a,,b").is_err());
        assert_eq!(validate(fb, &ctx(Harness::Codex), "a, b").unwrap(), "a, b");
        assert!(validate(fb, &ctx(Harness::Codex), "a\nb").is_err());
    }

    #[test]
    fn git_and_comment_accept_free_text_but_not_blank() {
        let f = find("git.commit-format").unwrap();
        assert_eq!(
            validate(f, &ctx(Harness::Claude), "<type>: <subject>").unwrap(),
            "<type>: <subject>"
        );
        assert!(validate(f, &ctx(Harness::Claude), "").is_err());
        let sign = find("git.signing").unwrap();
        assert!(validate(sign, &ctx(Harness::Claude), "maybe").is_err());
        assert!(
            validate(
                find("comment.language").unwrap(),
                &ctx(Harness::Claude),
                "korean"
            )
            .is_ok()
        );
    }

    #[test]
    fn subagent_model_per_harness() {
        let m = find("subagent.model").unwrap();
        for ok in [
            "sonnet",
            "opus",
            "haiku",
            "fable",
            "inherit",
            "claude-opus-5-5",
            "",
        ] {
            assert!(validate(m, &ctx(Harness::Claude), ok).is_ok(), "{ok}");
        }
        for bad in ["gpt-6", "claude-", "Sonnet"] {
            assert!(validate(m, &ctx(Harness::Claude), bad).is_err(), "{bad}");
        }
        assert!(validate(m, &ctx(Harness::Codex), "gpt-6-astra").is_ok());
        assert!(validate(m, &ctx(Harness::Codex), "two words").is_err());
    }

    #[test]
    fn kimi_model_must_be_an_alias() {
        let m = find("subagent.model").unwrap();
        let mut c = ctx(Harness::Kimi);
        assert!(validate(m, &c, "k3").unwrap_err().contains("no [models]"));
        c.kimi_models = Some(vec!["k3".into(), "k2".into()]);
        assert!(validate(m, &c, "k3").is_ok());
        let err = validate(m, &c, "primary").unwrap_err();
        assert!(err.contains("k3, k2"), "{err}");
        assert_eq!(validate(m, &c, "").unwrap(), "");
    }

    #[test]
    fn subagent_effort_per_harness() {
        let e = find("subagent.effort").unwrap();
        assert!(validate(e, &ctx(Harness::Claude), "high").is_err());
        assert!(validate(e, &ctx(Harness::Claude), "").is_ok());
        assert!(validate(e, &ctx(Harness::Codex), "ultra").is_ok());
        assert!(validate(e, &ctx(Harness::Kimi), "ultra").is_err());
        assert!(validate(e, &ctx(Harness::Kimi), "max").is_ok());
    }

    #[test]
    fn applicability_follows_backend_and_harness() {
        let c = find("aside.claude.model").unwrap();
        assert!(applies(c, &ctx(Harness::Codex), "claude"));
        assert!(!applies(c, &ctx(Harness::Codex), "codex"));
        let e = find("subagent.effort").unwrap();
        assert!(!applies(e, &ctx(Harness::Claude), "codex"));
        assert!(applies(e, &ctx(Harness::Kimi), "codex"));
    }
}
