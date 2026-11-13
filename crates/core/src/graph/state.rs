//! Typed state <-> the library's channel map.

use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// The typed state of a graph: a `serde` struct whose fields are the channels. It is saved with
/// every checkpoint, so give a field added later `#[serde(default)]` to keep threads saved by an
/// older version loadable.
pub trait GraphState: Serialize + DeserializeOwned + Clone + Send + Sync + 'static {}
impl<T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static> GraphState for T {}

/// The state as a JSON object (one entry per field).
pub(crate) fn to_map<S: GraphState>(state: &S) -> Result<Map<String, Value>> {
    // Errors name the kind only: serde's messages can quote the offending value.
    match serde_json::to_value(state) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => bail!("graph state must serialize to a JSON object (use a struct)"),
        Err(e) => bail!("graph state cannot be serialized ({:?})", e.classify()),
    }
}

/// The typed state from the library's channel map.
pub(crate) fn from_map<S: GraphState>(map: &adk_graph::State) -> Result<S> {
    let object: Map<String, Value> = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    serde_json::from_value(Value::Object(object))
        .map_err(|e| anyhow::anyhow!("graph state does not fit its type ({:?})", e.classify()))
}

/// Updates that turn `before` into `after`: the changed fields, with the new tail only for
/// `appends` (list channels, which the library extends).
pub(crate) fn diff(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    appends: &[&str],
) -> Result<adk_graph::State> {
    let mut updates = adk_graph::State::new();
    for (key, new) in after {
        let old = before.get(key);
        if old == Some(new) {
            continue;
        }
        if appends.contains(&key.as_str()) {
            let (old, new) = (old.and_then(Value::as_array).map_or(&[][..], |a| a.as_slice()), new.as_array());
            match new {
                Some(new) if new.starts_with(old) => {
                    updates.insert(key.clone(), Value::Array(new[old.len()..].to_vec()));
                }
                _ => bail!("append field `{key}` may only grow (nodes push, never edit or remove)"),
            }
        } else {
            updates.insert(key.clone(), new.clone());
        }
    }
    // A field that serializes only when set disappears from `after` when cleared.
    for key in before.keys().filter(|k| !after.contains_key(*k)) {
        if appends.contains(&key.as_str()) {
            bail!("append field `{key}` may only grow (nodes push, never edit or remove)");
        }
        updates.insert(key.clone(), Value::Null);
    }
    Ok(updates)
}
