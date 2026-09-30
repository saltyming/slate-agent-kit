//! [`ConfigDoc`] implementation over a `toml_edit` document.
//!
//! Owns key-path navigation of TOML tables (including inline tables), the
//! conversion between TOML values and [`Json`], and format-preserving writes:
//! comments, ordering and the decoration of untouched keys survive, and a
//! replaced value keeps its trailing comment. It does not choose keys.
//!
//! Main entry points: [`TomlDoc::parse`], [`TomlDoc::render`] and the
//! [`ConfigDoc`] methods.

use super::ConfigDoc;
use crate::error::{Error, Result};
use crate::json::Json;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// A parsed TOML configuration file.
#[derive(Debug, Clone)]
pub struct TomlDoc {
    doc: DocumentMut,
}

impl TomlDoc {
    /// Parses TOML text. An empty text is an empty document.
    pub fn parse(text: &str) -> Result<TomlDoc> {
        text.parse::<DocumentMut>()
            .map(|doc| TomlDoc { doc })
            .map_err(|e| Error::config(format!("invalid TOML: {e}")))
    }

    /// The document text, with all untouched formatting intact.
    pub fn render(&self) -> String {
        self.doc.to_string()
    }

    /// The root table.
    pub fn root(&self) -> &Table {
        self.doc.as_table()
    }

    /// The keys of the table at `path`, in file order; empty when it is missing.
    pub fn keys_at(&self, path: &[&str]) -> Vec<String> {
        let mut cur: &dyn TableLike = self.doc.as_table();
        for seg in path {
            match cur.get(seg).and_then(Item::as_table_like) {
                Some(t) => cur = t,
                None => return Vec::new(),
            }
        }
        cur.iter().map(|(k, _)| k.to_string()).collect()
    }

    /// Returns the item type at `path` as a short word, for refusal messages.
    pub fn kind_at(&self, path: &[&str]) -> Option<&'static str> {
        let mut cur: &Item = self.doc.as_item();
        for seg in path {
            cur = cur.as_table_like()?.get(seg)?;
        }
        Some(match cur {
            Item::None => "none",
            Item::Value(Value::String(_)) => "string",
            Item::Value(Value::Integer(_)) => "integer",
            Item::Value(Value::Float(_)) => "float",
            Item::Value(Value::Boolean(_)) => "boolean",
            Item::Value(Value::Datetime(_)) => "datetime",
            Item::Value(Value::Array(_)) | Item::ArrayOfTables(_) => "array",
            Item::Value(Value::InlineTable(_)) | Item::Table(_) => "table",
        })
    }
}

fn value_to_json(v: &Value) -> Json {
    match v {
        Value::String(s) => Json::String(s.value().clone()),
        Value::Integer(i) => Json::Number(i.value().to_string()),
        Value::Float(f) => Json::Number(f.value().to_string()),
        Value::Boolean(b) => Json::Bool(*b.value()),
        Value::Datetime(d) => Json::String(d.value().to_string()),
        Value::Array(a) => Json::Array(a.iter().map(value_to_json).collect()),
        Value::InlineTable(t) => table_like_to_json(t),
    }
}

fn table_like_to_json(t: &dyn TableLike) -> Json {
    Json::Object(
        t.iter()
            .map(|(k, v)| (k.to_string(), item_to_json(v)))
            .collect(),
    )
}

fn item_to_json(item: &Item) -> Json {
    match item {
        Item::None => Json::Null,
        Item::Value(v) => value_to_json(v),
        Item::Table(t) => table_like_to_json(t),
        Item::ArrayOfTables(a) => Json::Array(a.iter().map(|t| table_like_to_json(t)).collect()),
    }
}

fn json_to_value(j: &Json) -> Result<Value> {
    Ok(match j {
        Json::Null => return Err(Error::config("TOML cannot store a null value")),
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => match n.parse::<i64>() {
            Ok(i) => Value::from(i),
            Err(_) => Value::from(
                n.parse::<f64>()
                    .map_err(|_| Error::config(format!("`{n}` is not a TOML number")))?,
            ),
        },
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(a) => {
            let mut arr = Array::new();
            for v in a {
                arr.push(json_to_value(v)?);
            }
            Value::Array(arr)
        }
        Json::Object(m) => {
            let mut t = InlineTable::new();
            for (k, v) in m {
                t.insert(k, json_to_value(v)?);
            }
            Value::InlineTable(t)
        }
    })
}

fn describe(path: &[&str], upto: usize) -> Error {
    Error::config(format!("`{}` is not a table", path[..=upto].join(".")))
}

impl ConfigDoc for TomlDoc {
    fn get(&self, path: &[&str]) -> Result<Option<Json>> {
        let mut cur: &dyn TableLike = self.doc.as_table();
        for (i, seg) in path.iter().enumerate() {
            let Some(item) = cur.get(seg) else {
                return Ok(None);
            };
            if i + 1 == path.len() {
                return Ok(Some(item_to_json(item)));
            }
            match item.as_table_like() {
                Some(t) => cur = t,
                None => return Err(describe(path, i)),
            }
        }
        Ok(None)
    }

    fn set(&mut self, path: &[&str], value: &Json) -> Result<usize> {
        let Some((leaf, parents)) = path.split_last() else {
            return Err(Error::config("empty key path"));
        };
        let new_value = json_to_value(value)?;
        let mut created = 0;
        let mut cur: &mut dyn TableLike = self.doc.as_table_mut();
        for (i, seg) in parents.iter().enumerate() {
            if cur.get(seg).is_none() {
                let mut t = Table::new();
                t.set_implicit(true);
                cur.insert(seg, Item::Table(t));
                created += 1;
            }
            cur = cur
                .get_mut(seg)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| describe(path, i))?;
        }
        let mut new_value = new_value;
        if let Some(Item::Value(old)) = cur.get(leaf) {
            *new_value.decor_mut() = old.decor().clone();
        }
        cur.insert(leaf, Item::Value(new_value));
        Ok(created)
    }

    fn append_string(&mut self, path: &[&str], item: &str) -> Result<usize> {
        let Some((leaf, parents)) = path.split_last() else {
            return Err(Error::config("empty key path"));
        };
        let exists = self.get(path)?.is_some();
        if !exists {
            return self.set(path, &Json::Array(vec![Json::str(item)]));
        }
        let mut cur: &mut dyn TableLike = self.doc.as_table_mut();
        for (i, seg) in parents.iter().enumerate() {
            cur = cur
                .get_mut(seg)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| describe(path, i))?;
        }
        let array = cur
            .get_mut(leaf)
            .and_then(Item::as_array_mut)
            .ok_or_else(|| Error::config(format!("`{}` is not an array", path.join("."))))?;
        // Give the new element the layout of the last one, so a multi-line array stays multi-line.
        let prefix = array
            .iter()
            .last()
            .and_then(|v| {
                v.decor()
                    .prefix()
                    .and_then(|p| p.as_str())
                    .map(str::to_string)
            })
            .filter(|p| p.contains('\n'));
        array.push(item);
        if let Some(prefix) = prefix
            && let Some(last) = array.get_mut(array.len() - 1)
        {
            last.decor_mut().set_prefix(prefix);
        }
        Ok(0)
    }

    fn remove_strings(&mut self, path: &[&str], items: &[String]) -> Result<()> {
        let Some((leaf, parents)) = path.split_last() else {
            return Err(Error::config("empty key path"));
        };
        if self.get(path)?.is_none() {
            return Ok(());
        }
        let mut cur: &mut dyn TableLike = self.doc.as_table_mut();
        for (i, seg) in parents.iter().enumerate() {
            cur = cur
                .get_mut(seg)
                .and_then(Item::as_table_like_mut)
                .ok_or_else(|| describe(path, i))?;
        }
        if let Some(array) = cur.get_mut(leaf).and_then(Item::as_array_mut) {
            let mut i = 0;
            while i < array.len() {
                let hit = array
                    .get(i)
                    .and_then(Value::as_str)
                    .is_some_and(|s| items.iter().any(|x| x == s));
                if hit {
                    array.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        Ok(())
    }

    fn remove(&mut self, path: &[&str], prune: usize) {
        fn go(table: &mut dyn TableLike, path: &[&str], prune: usize) {
            let Some((first, rest)) = path.split_first() else {
                return;
            };
            if rest.is_empty() {
                table.remove(first);
                return;
            }
            if let Some(child) = table.get_mut(first).and_then(Item::as_table_like_mut) {
                go(child, rest, prune);
                if rest.len() <= prune && child.is_empty() {
                    table.remove(first);
                }
            }
        }
        go(self.doc.as_table_mut(), path, prune);
    }
}
