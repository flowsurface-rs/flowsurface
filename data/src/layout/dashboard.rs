use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use super::{WindowSpec, pane::Pane};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Dashboard {
    #[serde(deserialize_with = "deserialize_pane", default)]
    pub pane: Pane,
    #[serde(deserialize_with = "deserialize_popouts", default)]
    pub popout: Vec<(Pane, WindowSpec)>,
}

fn deserialize_pane<'de, D>(deserializer: D) -> Result<Pane, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    deserialize_normalized_pane(normalize_pane(value)).map_err(serde::de::Error::custom)
}

fn deserialize_popouts<'de, D>(deserializer: D) -> Result<Vec<(Pane, WindowSpec)>, D::Error>
where
    D: Deserializer<'de>,
{
    let popouts = Vec::<(Value, WindowSpec)>::deserialize(deserializer)?;

    popouts
        .into_iter()
        .map(|(pane, window)| {
            deserialize_normalized_pane(normalize_pane(pane))
                .map(|pane| (pane, window))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

fn normalize_pane(mut value: Value) -> Value {
    if let Some(split) = value.get_mut("Split").and_then(Value::as_object_mut) {
        for child_name in ["a", "b"] {
            if let Some(child) = split.remove(child_name) {
                split.insert(child_name.to_string(), normalize_pane(child));
            }
        }
    }

    if serde_json::from_value::<Pane>(value.clone()).is_ok() {
        value
    } else {
        starter_pane(&value)
    }
}

fn deserialize_normalized_pane(value: Value) -> Result<Pane, serde_json::Error> {
    serde_json::from_value(value)
}

fn starter_pane(value: &Value) -> Value {
    let link_group = value
        .as_object()
        .and_then(|object| object.values().next())
        .and_then(Value::as_object)
        .and_then(|payload| payload.get("link_group"))
        .cloned()
        .unwrap_or(Value::Null);

    serde_json::json!({
        "Starter": {
            "link_group": link_group,
        }
    })
}
