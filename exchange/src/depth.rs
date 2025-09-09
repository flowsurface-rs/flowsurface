use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

use serde::Deserializer;
use serde::de::Error as SerdeError;
use serde_json::Value;

#[derive(Clone, Copy)]
pub struct Order {
    pub price: f32,
    pub qty: f32,
}

impl<'de> serde::Deserialize<'de> for Order {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // can be either an array like ["price","qty", ...] or an object with keys "0" and "1"
        let value = Value::deserialize(deserializer).map_err(SerdeError::custom)?;

        let parse_f = |val: &Value| -> Option<f32> {
            match val {
                Value::String(s) => s.parse::<f32>().ok(),
                Value::Number(n) => n.as_f64().map(|x| x as f32),
                _ => None,
            }
        };

        let price = match &value {
            Value::Array(arr) => arr.first().and_then(parse_f),
            Value::Object(map) => map.get("0").and_then(parse_f),
            _ => None,
        }
        .ok_or_else(|| SerdeError::custom("Order price not found or invalid"))?;

        let qty = match &value {
            Value::Array(arr) => arr.get(1).and_then(parse_f),
            Value::Object(map) => map.get("1").and_then(parse_f),
            _ => None,
        }
        .ok_or_else(|| SerdeError::custom("Order qty not found or invalid"))?;

        Ok(Order { price, qty })
    }
}

pub struct DepthPayload {
    pub last_update_id: u64,
    pub time: u64,
    pub bids: Vec<Order>,
    pub asks: Vec<Order>,
}

pub enum DepthUpdate {
    Snapshot(DepthPayload),
    Diff(DepthPayload),
}

#[derive(Clone, Default)]
pub struct Depth {
    pub bids: BTreeMap<OrderedFloat<f32>, f32>,
    pub asks: BTreeMap<OrderedFloat<f32>, f32>,
}

impl std::fmt::Debug for Depth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Depth")
            .field("bids", &self.bids.len())
            .field("asks", &self.asks.len())
            .finish()
    }
}

impl Depth {
    pub fn update(&mut self, diff: &DepthPayload) {
        Self::diff_price_levels(&mut self.bids, &diff.bids);
        Self::diff_price_levels(&mut self.asks, &diff.asks);
    }

    fn diff_price_levels(price_map: &mut BTreeMap<OrderedFloat<f32>, f32>, orders: &[Order]) {
        orders.iter().for_each(|order| {
            if order.qty == 0.0 {
                price_map.remove(&OrderedFloat(order.price));
            } else {
                price_map.insert(OrderedFloat(order.price), order.qty);
            }
        });
    }

    pub fn replace_all(&mut self, snapshot: &DepthPayload) {
        self.bids = snapshot
            .bids
            .iter()
            .map(|order| (OrderedFloat(order.price), order.qty))
            .collect();
        self.asks = snapshot
            .asks
            .iter()
            .map(|order| (OrderedFloat(order.price), order.qty))
            .collect();
    }

    pub fn mid_price(&self) -> Option<f32> {
        match (self.asks.first_key_value(), self.bids.last_key_value()) {
            (Some((ask_price, _)), Some((bid_price, _))) => {
                Some((ask_price.into_inner() + bid_price.into_inner()) / 2.0)
            }
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct LocalDepthCache {
    pub last_update_id: u64,
    pub time: u64,
    pub depth: Depth,
}

impl LocalDepthCache {
    pub fn update(&mut self, new_depth: DepthUpdate) {
        match new_depth {
            DepthUpdate::Snapshot(snapshot) => {
                self.last_update_id = snapshot.last_update_id;
                self.time = snapshot.time;
                self.depth.replace_all(&snapshot);
            }
            DepthUpdate::Diff(diff) => {
                self.last_update_id = diff.last_update_id;
                self.time = diff.time;
                self.depth.update(&diff);
            }
        }
    }
}
