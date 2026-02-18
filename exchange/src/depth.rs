use crate::{
    MinTicksize, Price,
    unit::qty::{Qty, QtyNormalization},
};

use serde::Deserializer;
use serde::de::Error as SerdeError;
use serde_json::Value;

use std::{collections::BTreeMap, sync::Arc};

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
    pub bids: BTreeMap<Price, Qty>,
    pub asks: BTreeMap<Price, Qty>,
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
    fn update_with_qty_norm(
        &mut self,
        diff: &DepthPayload,
        min_ticksize: MinTicksize,
        qty_norm: Option<QtyNormalization>,
    ) {
        Self::diff_price_levels(&mut self.bids, &diff.bids, min_ticksize, qty_norm);
        Self::diff_price_levels(&mut self.asks, &diff.asks, min_ticksize, qty_norm);
    }

    fn diff_price_levels(
        price_map: &mut BTreeMap<Price, Qty>,
        orders: &[DeOrder],
        min_ticksize: MinTicksize,
        qty_norm: Option<QtyNormalization>,
    ) {
        orders.iter().for_each(|order| {
            let normalized_qty = qty_norm
                .map(|normalizer| normalizer.normalize(order.qty, order.price))
                .unwrap_or(order.qty);

            let price = Price::from_f32(order.price).round_to_min_tick(min_ticksize);
            let qty = Qty::from_f32(normalized_qty);

            if qty.is_zero() {
                price_map.remove(&price);
            } else {
                price_map.insert(price, qty);
            }
        });
    }

    fn replace_all_with_qty_norm(
        &mut self,
        snapshot: &DepthPayload,
        min_ticksize: MinTicksize,
        qty_norm: Option<QtyNormalization>,
    ) {
        self.bids = snapshot
            .bids
            .iter()
            .map(|de_order| {
                let normalized_qty = qty_norm
                    .map(|normalizer| normalizer.normalize(de_order.qty, de_order.price))
                    .unwrap_or(de_order.qty);

                (
                    Price::from_f32(de_order.price).round_to_min_tick(min_ticksize),
                    Qty::from_f32(normalized_qty),
                )
            })
            .collect::<BTreeMap<Price, Qty>>();
        self.asks = snapshot
            .asks
            .iter()
            .map(|de_order| {
                let normalized_qty = qty_norm
                    .map(|normalizer| normalizer.normalize(de_order.qty, de_order.price))
                    .unwrap_or(de_order.qty);

                (
                    Price::from_f32(de_order.price).round_to_min_tick(min_ticksize),
                    Qty::from_f32(normalized_qty),
                )
            })
            .collect::<BTreeMap<Price, Qty>>();
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
    pub depth: Arc<Depth>,
}

impl LocalDepthCache {
    pub fn update(&mut self, new_depth: DepthUpdate, min_ticksize: MinTicksize) {
        self.update_with_qty_norm(new_depth, min_ticksize, None);
    }

    pub fn update_with_qty_norm(
        &mut self,
        new_depth: DepthUpdate,
        min_ticksize: MinTicksize,
        qty_norm: Option<QtyNormalization>,
    ) {
        match new_depth {
            DepthUpdate::Snapshot(snapshot) => {
                self.last_update_id = snapshot.last_update_id;
                self.time = snapshot.time;

                let depth = Arc::make_mut(&mut self.depth);
                depth.replace_all_with_qty_norm(&snapshot, min_ticksize, qty_norm);
            }
            DepthUpdate::Diff(diff) => {
                self.last_update_id = diff.last_update_id;
                self.time = diff.time;

                let depth = Arc::make_mut(&mut self.depth);
                depth.update_with_qty_norm(&diff, min_ticksize, qty_norm);
            }
        }
    }
}
