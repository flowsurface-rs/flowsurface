use serde::{Deserialize, Deserializer, de::Error as DeError};
use serde_json::{Number, Value};
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

fn number_as_f32(number: &Number) -> Option<f32> {
    number
        .as_i64()
        .map(|v| v as f32)
        .or_else(|| number.as_u64().map(|v| v as f32))
        .or_else(|| number.to_string().parse::<f32>().ok())
}

pub(crate) fn value_as_f32(value: &Value) -> Option<f32> {
    match value {
        Value::String(s) => s.parse::<f32>().ok(),
        Value::Number(n) => number_as_f32(n),
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
    from_f32: impl Fn(f32) -> T,
) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;

    match value {
        Value::Object(_) => serde_json::from_value::<T>(value).map_err(D::Error::custom),
        Value::String(s) => {
            let number = s.parse::<f32>().map_err(D::Error::custom)?;
            Ok(from_f32(number))
        }
        Value::Number(n) => {
            let number = number_as_f32(&n)
                .ok_or_else(|| D::Error::custom(format!("expected numeric {expected_name}")))?;
            Ok(from_f32(number))
        }
        _ => Err(D::Error::custom(format!(
            "expected {expected_name} as string or number"
        ))),
    }
}
