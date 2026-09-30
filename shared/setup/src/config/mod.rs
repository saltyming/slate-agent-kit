//! Format-preserving edits of harness configuration keys, with undo records.
//!
//! Owns the [`ConfigDoc`] abstraction over TOML (`toml_edit`) and JSON
//! ([`crate::json`]) documents, the [`ConfigRecord`] that the manifest stores for
//! every edited key, and the generic set, list-add and restore operations. It
//! does not know which keys a harness needs; `harness::*` and `native` choose them.
//!
//! Main entry points: [`ConfigDoc`], [`set_value`], [`list_add`], [`restore`],
//! [`TomlDoc`] and [`JsonDoc`].

mod json_doc;
mod toml_doc;

pub use json_doc::JsonDoc;
pub use toml_doc::TomlDoc;

use crate::error::{Error, Result};
use crate::json::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A configuration document that can be read and edited by key path.
pub trait ConfigDoc {
    /// Returns the value at `path` as JSON, or `None` when a segment is missing.
    ///
    /// Fails when an intermediate segment exists but is not a table.
    fn get(&self, path: &[&str]) -> Result<Option<Json>>;

    /// Sets the value at `path`, creating missing tables. Returns how many
    /// parent tables of the leaf were created.
    fn set(&mut self, path: &[&str], value: &Json) -> Result<usize>;

    /// Removes the key at `path`, then removes up to `prune` emptied parent tables.
    fn remove(&mut self, path: &[&str], prune: usize);

    /// Appends a string to the array at `path`, keeping the array's formatting and comments.
    /// Creates the array (and missing tables) when the key does not exist.
    fn append_string(&mut self, path: &[&str], item: &str) -> Result<usize>;

    /// Removes the given strings from the array at `path`, keeping the rest of it untouched.
    fn remove_strings(&mut self, path: &[&str], items: &[String]) -> Result<()>;
}

/// One edited key, as stored in the manifest so uninstall can undo it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigRecord {
    /// The edited file.
    pub file: PathBuf,
    /// Key path inside the document.
    pub path: Vec<String>,
    /// JSON text of the value before the edit; absent when the key did not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// JSON text of the value the installer wrote.
    pub written: String,
    /// Number of parent tables the edit created.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub created_parents: usize,
    /// For list edits: the entries the installer added. Uninstall removes only these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What one edit changed, for the summary and for the manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Key path.
    pub path: Vec<String>,
    /// Value before the edit.
    pub old: Option<Json>,
    /// Value after the edit.
    pub new: Json,
    /// Parent tables created by the edit.
    pub created_parents: usize,
    /// Entries added, for list edits.
    pub added: Vec<String>,
}

impl Change {
    /// Builds the manifest record for this change in `file`.
    pub fn record(&self, file: &std::path::Path) -> ConfigRecord {
        ConfigRecord {
            file: file.to_path_buf(),
            path: self.path.clone(),
            previous: self.old.as_ref().map(Json::to_compact),
            written: self.new.to_compact(),
            created_parents: self.created_parents,
            added: self.added.clone(),
        }
    }

    /// The dotted key path, for messages.
    pub fn dotted(&self) -> String {
        self.path.join(".")
    }
}

fn refs(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

/// Sets `path` to `value` unless it already holds it. Returns the change, if any.
pub fn set_value(doc: &mut dyn ConfigDoc, path: &[&str], value: Json) -> Result<Option<Change>> {
    let old = doc.get(path)?;
    if old.as_ref() == Some(&value) {
        return Ok(None);
    }
    let created_parents = doc.set(path, &value)?;
    Ok(Some(Change {
        path: path.iter().map(|s| s.to_string()).collect(),
        old,
        new: value,
        created_parents,
        added: Vec::new(),
    }))
}

/// Adds each string in `items` to the array at `path` when missing.
///
/// Fails when the key exists and is not an array of strings.
pub fn list_add(
    doc: &mut dyn ConfigDoc,
    path: &[&str],
    items: &[String],
) -> Result<Option<Change>> {
    let old = doc.get(path)?;
    let mut list: Vec<Json> = match &old {
        None => Vec::new(),
        Some(Json::Array(a)) => a.clone(),
        Some(_) => {
            return Err(Error::config(format!(
                "`{}` is not an array, so entries cannot be added to it",
                path.join(".")
            )));
        }
    };
    let mut added = Vec::new();
    let mut created_parents = 0;
    for item in items {
        if !list.iter().any(|v| v.as_str() == Some(item.as_str())) {
            list.push(Json::str(item.clone()));
            added.push(item.clone());
            created_parents += doc.append_string(path, item)?;
        }
    }
    if added.is_empty() {
        return Ok(None);
    }
    let new = Json::Array(list);
    Ok(Some(Change {
        path: path.iter().map(|s| s.to_string()).collect(),
        old,
        new,
        created_parents,
        added,
    }))
}

/// Result of undoing one recorded edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Restored {
    /// The key holds its previous value again.
    Restored,
    /// The key was removed because it did not exist before the install.
    Removed,
    /// Nothing to undo: the key is already in its previous state.
    Nothing,
    /// The current value is not the one the installer wrote; the key was left alone.
    Changed(Option<String>),
}

/// Undoes `record` in `doc`, but only while the key still holds what the installer wrote.
pub fn restore(doc: &mut dyn ConfigDoc, record: &ConfigRecord) -> Result<Restored> {
    let path = refs(&record.path);
    let current = doc.get(&path)?;
    if !record.added.is_empty() {
        let Some(Json::Array(list)) = current else {
            return Ok(Restored::Nothing);
        };
        let remaining = list
            .iter()
            .filter(|v| {
                !v.as_str()
                    .is_some_and(|s| record.added.iter().any(|a| a == s))
            })
            .count();
        if remaining == 0 && record.previous.is_none() {
            doc.remove(&path, record.created_parents);
            return Ok(Restored::Removed);
        }
        doc.remove_strings(&path, &record.added)?;
        return Ok(Restored::Restored);
    }
    let written = Json::parse(&record.written)?;
    match (&current, &record.previous) {
        (None, None) => Ok(Restored::Nothing),
        (Some(cur), _) if *cur == written => match &record.previous {
            Some(prev) => {
                doc.set(&path, &Json::parse(prev)?)?;
                Ok(Restored::Restored)
            }
            None => {
                doc.remove(&path, record.created_parents);
                Ok(Restored::Removed)
            }
        },
        (Some(cur), Some(prev)) if Json::parse(prev)? == *cur => Ok(Restored::Nothing),
        (cur, _) => Ok(Restored::Changed(cur.as_ref().map(Json::to_compact))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toml(text: &str) -> TomlDoc {
        TomlDoc::parse(text).unwrap()
    }

    #[test]
    fn set_records_previous_and_restores_only_when_unchanged() {
        let mut doc = toml("[agents]\ndefault_subagent_model = \"old\" # keep me\nother = 1\n");
        let change = set_value(
            &mut doc,
            &["agents", "default_subagent_model"],
            Json::str("new"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(change.old, Some(Json::str("old")));
        assert_eq!(
            doc.render(),
            "[agents]\ndefault_subagent_model = \"new\" # keep me\nother = 1\n"
        );
        let rec = change.record(std::path::Path::new("c.toml"));
        assert_eq!(restore(&mut doc, &rec).unwrap(), Restored::Restored);
        assert_eq!(
            doc.render(),
            "[agents]\ndefault_subagent_model = \"old\" # keep me\nother = 1\n"
        );

        // Changed by the user after the install: left alone.
        set_value(
            &mut doc,
            &["agents", "default_subagent_model"],
            Json::str("new"),
        )
        .unwrap();
        doc.set(&["agents", "default_subagent_model"], &Json::str("user"))
            .unwrap();
        assert_eq!(
            restore(&mut doc, &rec).unwrap(),
            Restored::Changed(Some("\"user\"".into()))
        );
        assert!(doc.render().contains("\"user\""));
    }

    #[test]
    fn restoring_a_created_key_prunes_created_tables_only() {
        let mut doc = toml("# top\n[keep]\na = 1\n");
        let change = set_value(
            &mut doc,
            &["features", "code_mode", "x"],
            Json::Number("1".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(change.created_parents, 2);
        let rec = change.record(std::path::Path::new("c.toml"));
        assert_eq!(restore(&mut doc, &rec).unwrap(), Restored::Removed);
        assert_eq!(doc.render(), "# top\n[keep]\na = 1\n");
    }

    #[test]
    fn list_add_keeps_existing_entries_and_is_idempotent() {
        let mut doc = toml("[features.code_mode]\nexcluded_tool_namespaces = [\"a\", \"b\"]\n");
        let items = vec!["mcp__aside".to_string()];
        let path = ["features", "code_mode", "excluded_tool_namespaces"];
        let change = list_add(&mut doc, &path, &items).unwrap().unwrap();
        assert_eq!(change.added, items);
        assert_eq!(
            doc.render(),
            "[features.code_mode]\nexcluded_tool_namespaces = [\"a\", \"b\", \"mcp__aside\"]\n"
        );
        assert!(list_add(&mut doc, &path, &items).unwrap().is_none());
        let rec = change.record(std::path::Path::new("c.toml"));
        assert_eq!(restore(&mut doc, &rec).unwrap(), Restored::Restored);
        assert_eq!(
            doc.render(),
            "[features.code_mode]\nexcluded_tool_namespaces = [\"a\", \"b\"]\n"
        );
    }

    #[test]
    fn list_add_rejects_non_arrays() {
        let mut doc = toml("[features.code_mode]\nexcluded_tool_namespaces = \"x\"\n");
        let err = list_add(
            &mut doc,
            &["features", "code_mode", "excluded_tool_namespaces"],
            &["n".to_string()],
        )
        .unwrap_err();
        assert!(err.to_string().contains("not an array"));
    }

    #[test]
    fn json_list_add_and_restore() {
        let mut doc = JsonDoc::parse(
            "{\n  \"permissions\": {\n    \"allow\": [\"Bash(ls)\"]\n  },\n  \"z\": 1\n}\n",
        )
        .unwrap();
        let items = vec!["mcp__palette__a".to_string(), "mcp__palette__b".to_string()];
        let change = list_add(&mut doc, &["permissions", "allow"], &items)
            .unwrap()
            .unwrap();
        assert_eq!(change.added.len(), 2);
        let rec = change.record(std::path::Path::new("settings.json"));
        // The user adds another entry afterwards; only ours are removed.
        let mut cur = doc.get(&["permissions", "allow"]).unwrap().unwrap();
        cur.as_array_mut().unwrap().push(Json::str("Bash(pwd)"));
        doc.set(&["permissions", "allow"], &cur).unwrap();
        assert_eq!(restore(&mut doc, &rec).unwrap(), Restored::Restored);
        assert_eq!(
            doc.get(&["permissions", "allow"])
                .unwrap()
                .unwrap()
                .to_compact(),
            "[\"Bash(ls)\",\"Bash(pwd)\"]"
        );
    }

    #[test]
    fn json_created_permissions_are_pruned() {
        let mut doc = JsonDoc::parse("{\"a\":1}").unwrap();
        let change = list_add(&mut doc, &["permissions", "allow"], &["x".to_string()])
            .unwrap()
            .unwrap();
        assert_eq!(change.created_parents, 1);
        let rec = change.record(std::path::Path::new("s.json"));
        assert_eq!(restore(&mut doc, &rec).unwrap(), Restored::Removed);
        assert_eq!(doc.render("  "), "{\n  \"a\": 1\n}\n");
    }
}

/// What a native setting should be after an install.
#[derive(Debug, Clone, PartialEq)]
pub enum Desired {
    /// The key holds this value.
    Set(Json),
    /// The key holds nothing the installer put there.
    Unset,
}

/// Brings `path` to `desired`, undoing an earlier installer write when the value is now unset.
///
/// `prior` is the manifest record of an earlier edit of the same key. Returns
/// the change made, if any, and whether an earlier edit was undone.
pub fn apply_desired(
    doc: &mut dyn ConfigDoc,
    path: &[&str],
    desired: &Desired,
    prior: Option<&ConfigRecord>,
) -> Result<(Option<Change>, Option<Restored>)> {
    match desired {
        Desired::Set(value) => Ok((set_value(doc, path, value.clone())?, None)),
        Desired::Unset => match prior {
            Some(rec) => {
                let outcome = restore(doc, rec)?;
                Ok((None, Some(outcome)))
            }
            None => Ok((None, None)),
        },
    }
}
