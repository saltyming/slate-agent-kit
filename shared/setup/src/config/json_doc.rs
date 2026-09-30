//! [`ConfigDoc`] implementation over an order-preserving JSON document.
//!
//! Owns key-path navigation of [`Json`] objects and the file-level parse and
//! render (indentation and trailing newline of the original are kept). It does
//! not choose keys or write files.
//!
//! Main entry points: [`JsonDoc::parse`], [`JsonDoc::render`] and the
//! [`ConfigDoc`] methods.

use super::ConfigDoc;
use crate::error::{Error, Result};
use crate::json::{Json, detect_indent};

/// A parsed JSON configuration file.
#[derive(Debug, Clone)]
pub struct JsonDoc {
    /// The document root, always an object.
    pub root: Json,
    /// Indentation unit detected in the source text.
    pub indent: String,
}

impl JsonDoc {
    /// Parses `text`, which must be a JSON object. An empty file counts as `{}`.
    pub fn parse(text: &str) -> Result<JsonDoc> {
        if text.trim().is_empty() {
            return Ok(JsonDoc {
                root: Json::object(),
                indent: "  ".into(),
            });
        }
        let root = Json::parse(text)?;
        if !root.is_object() {
            return Err(Error::config("the top-level JSON value is not an object"));
        }
        Ok(JsonDoc {
            root,
            indent: detect_indent(text),
        })
    }

    /// Serialises with the given indentation unit and a trailing newline.
    pub fn render(&self, indent: &str) -> String {
        self.root.to_pretty(indent)
    }

    /// Serialises with the indentation the source used.
    pub fn render_original(&self) -> String {
        self.root.to_pretty(&self.indent)
    }
}

fn not_a_table(path: &[&str], upto: usize) -> Error {
    Error::config(format!("`{}` is not an object", path[..=upto].join(".")))
}

impl ConfigDoc for JsonDoc {
    fn get(&self, path: &[&str]) -> Result<Option<Json>> {
        let mut cur = &self.root;
        for (i, seg) in path.iter().enumerate() {
            match cur.get(seg) {
                Some(next) => cur = next,
                None => return Ok(None),
            }
            if i + 1 < path.len() && !cur.is_object() {
                return Err(not_a_table(path, i));
            }
        }
        Ok(Some(cur.clone()))
    }

    fn set(&mut self, path: &[&str], value: &Json) -> Result<usize> {
        let Some((leaf, parents)) = path.split_last() else {
            return Err(Error::config("empty key path"));
        };
        let mut created = 0;
        let mut cur = &mut self.root;
        for (i, seg) in parents.iter().enumerate() {
            if cur.get(seg).is_none() {
                cur.set(seg, Json::object());
                created += 1;
            }
            cur = match cur.get_mut(seg) {
                Some(next) if next.is_object() => next,
                _ => return Err(not_a_table(path, i)),
            };
        }
        cur.set(leaf, value.clone());
        Ok(created)
    }

    fn append_string(&mut self, path: &[&str], item: &str) -> Result<usize> {
        let Some((leaf, parents)) = path.split_last() else {
            return Err(Error::config("empty key path"));
        };
        match self.get(path)? {
            Some(Json::Array(mut list)) => {
                list.push(Json::str(item));
                self.set(path, &Json::Array(list))?;
                Ok(0)
            }
            Some(_) => Err(Error::config(format!(
                "`{}` is not an array",
                path.join(".")
            ))),
            None => {
                let _ = (leaf, parents);
                self.set(path, &Json::Array(vec![Json::str(item)]))
            }
        }
    }

    fn remove_strings(&mut self, path: &[&str], items: &[String]) -> Result<()> {
        if let Some(Json::Array(list)) = self.get(path)? {
            let kept: Vec<Json> = list
                .into_iter()
                .filter(|v| !v.as_str().is_some_and(|s| items.iter().any(|i| i == s)))
                .collect();
            self.set(path, &Json::Array(kept))?;
        }
        Ok(())
    }

    fn remove(&mut self, path: &[&str], prune: usize) {
        fn go(node: &mut Json, path: &[&str], prune: usize) {
            let Some((first, rest)) = path.split_first() else {
                return;
            };
            if rest.is_empty() {
                node.remove(first);
                return;
            }
            if let Some(child) = node.get_mut(first) {
                go(child, rest, prune);
                let depth_from_leaf = rest.len();
                if depth_from_leaf <= prune && child.is_empty_container() {
                    node.remove(first);
                }
            }
        }
        go(&mut self.root, path, prune);
    }
}
