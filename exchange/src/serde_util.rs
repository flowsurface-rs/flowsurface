use serde::{Deserialize, Deserializer, de::Error as DeError};
use serde_json::Value;
use std::str::FromStr;

pub(crate) fn de_string_to_number<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<T>().map_err(D::Error::custom)
}

pub(crate) fn value_as_f32(value: &Value) -> Option<f32> {
    value_as_f64(value).map(|v| v as f32)
}

pub(crate) fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

pub(crate) fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::String(s) => s.parse::<u64>().ok(),
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok())),
        _ => None,
    }
}

pub(crate) fn de_number_like_or_object<'de, D, T>(
    deserializer: D,
    expected_name: &'static str,
    from_f64: impl Fn(f64) -> T,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;

    match value {
        Value::Object(_) => serde_json::from_value::<T>(value).map_err(D::Error::custom),
        Value::String(s) => {
            let number = s.parse::<f64>().map_err(D::Error::custom)?;
            Ok(from_f64(number))
        }
        Value::Number(n) => {
            let number = n
                .as_f64()
                .ok_or_else(|| D::Error::custom(format!("expected numeric {expected_name}")))?;
            Ok(from_f64(number))
        }
        _ => Err(D::Error::custom(format!(
            "expected {expected_name} as string or number"
        ))),
    }
}
