use serde::{Deserialize, Serialize};

use ordered_float::OrderedFloat;
use std::collections::BTreeMap;

use super::de_string_to_f32;

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Order {
    #[serde(rename = "0", deserialize_with = "de_string_to_f32")]
    pub price: f32,
    #[serde(rename = "1", deserialize_with = "de_string_to_f32")]
    pub qty: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Depth {
    pub bids: BTreeMap<OrderedFloat<f32>, f32>,
    pub asks: BTreeMap<OrderedFloat<f32>, f32>,
}

#[derive(Debug, Clone, Default)]
pub struct TempLocalDepth {
    pub last_update_id: u64,
    pub time: u64,
    pub bids: Vec<Order>,
    pub asks: Vec<Order>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalDepthCache {
    pub last_update_id: u64,
    pub time: u64,
    pub bids: BTreeMap<OrderedFloat<f32>, f32>,
    pub asks: BTreeMap<OrderedFloat<f32>, f32>,
}

impl LocalDepthCache {
    pub fn new() -> Self {
        Self {
            last_update_id: 0,
            time: 0,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    pub fn fetched(&mut self, new_depth: &TempLocalDepth) {
        self.last_update_id = new_depth.last_update_id;
        self.time = new_depth.time;

        self.bids = new_depth
            .bids
            .iter()
            .map(|order| (OrderedFloat(order.price), order.qty))
            .collect();
        self.asks = new_depth
            .asks
            .iter()
            .map(|order| (OrderedFloat(order.price), order.qty))
            .collect();
    }

    pub fn update_depth_cache(&mut self, new_depth: &TempLocalDepth) {
        self.last_update_id = new_depth.last_update_id;
        self.time = new_depth.time;

        Self::update_price_levels(&mut self.bids, &new_depth.bids);
        Self::update_price_levels(&mut self.asks, &new_depth.asks);
    }

    fn update_price_levels(price_map: &mut BTreeMap<OrderedFloat<f32>, f32>, orders: &[Order]) {
        orders.iter().for_each(|order| {
            if order.qty == 0.0 {
                price_map.remove(&OrderedFloat(order.price));
            } else {
                price_map.insert(OrderedFloat(order.price), order.qty);
            }
        });
    }

    pub fn get_fetch_id(&self) -> u64 {
        self.last_update_id
    }

    pub fn get_depth(&self) -> Depth {
        Depth {
            bids: self.bids.clone(),
            asks: self.asks.clone(),
        }
    }
}
