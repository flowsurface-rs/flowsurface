use std::collections::BTreeMap;

use serde::Deserializer;
use serde::de::Error as SerdeError;
use serde_json::Value;

use crate::{MinTicksize, Price};

#[derive(Clone, Copy)]
pub struct DeOrder {
    pub price: f32,
    pub qty: f32,
}

impl<'de> serde::Deserialize<'de> for DeOrder {
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

        Ok(DeOrder { price, qty })
    }
}

struct Order {
    price: Price,
    qty: f32,
}

pub struct DepthPayload {
    pub last_update_id: u64,
    pub time: u64,
    pub bids: Vec<DeOrder>,
    pub asks: Vec<DeOrder>,
}

pub enum DepthUpdate {
    Snapshot(DepthPayload),
    Diff(DepthPayload),
}

#[derive(Clone, Default)]
pub struct Depth {
    pub bids: BTreeMap<Price, f32>,
    pub asks: BTreeMap<Price, f32>,
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
    pub fn update(&mut self, diff: &DepthPayload, min_ticksize: MinTicksize) {
        Self::diff_price_levels(&mut self.bids, &diff.bids, min_ticksize);
        Self::diff_price_levels(&mut self.asks, &diff.asks, min_ticksize);
    }

    fn diff_price_levels(
        price_map: &mut BTreeMap<Price, f32>,
        orders: &[DeOrder],
        min_ticksize: MinTicksize,
    ) {
        orders.iter().for_each(|order| {
            let order = Order {
                price: Price::from_f32(order.price).round_to_min_tick(min_ticksize),
                qty: order.qty,
            };

            if order.qty == 0.0 {
                price_map.remove(&order.price);
            } else {
                price_map.insert(order.price, order.qty);
            }
        });
    }

    pub fn replace_all(&mut self, snapshot: &DepthPayload, min_ticksize: MinTicksize) {
        self.bids = snapshot
            .bids
            .iter()
            .map(|de_order| {
                (
                    Price::from_f32(de_order.price).round_to_min_tick(min_ticksize),
                    de_order.qty,
                )
            })
            .collect::<BTreeMap<Price, f32>>();
        self.asks = snapshot
            .asks
            .iter()
            .map(|de_order| {
                (
                    Price::from_f32(de_order.price).round_to_min_tick(min_ticksize),
                    de_order.qty,
                )
            })
            .collect::<BTreeMap<Price, f32>>();
    }

    pub fn mid_price(&self) -> Option<Price> {
        match (self.asks.first_key_value(), self.bids.last_key_value()) {
            (Some((ask_price, _)), Some((bid_price, _))) => Some((*ask_price + *bid_price) / 2),
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
    pub fn update(&mut self, new_depth: DepthUpdate, min_ticksize: MinTicksize) {
        match new_depth {
            DepthUpdate::Snapshot(snapshot) => {
                self.last_update_id = snapshot.last_update_id;
                self.time = snapshot.time;
                self.depth.replace_all(&snapshot, min_ticksize);
            }
            DepthUpdate::Diff(diff) => {
                self.last_update_id = diff.last_update_id;
                self.time = diff.time;
                self.depth.update(&diff, min_ticksize);
            }
        }
    }
}
