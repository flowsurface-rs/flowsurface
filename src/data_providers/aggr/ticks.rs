use std::collections::HashMap;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use crate::data_providers::Trade;

use super::round_to_tick;

type FootprintTrades = HashMap<OrderedFloat<f32>, (f32, f32)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickCount {
    T50,
    T100,
    T200,
    T500,
    T1000,
    T2000,
    T5000,
    T10000,
}

impl TickCount {
    pub const ALL: [TickCount; 8] = [
        TickCount::T50,
        TickCount::T100,
        TickCount::T200,
        TickCount::T500,
        TickCount::T1000,
        TickCount::T2000,
        TickCount::T5000,
        TickCount::T10000,
    ];
}

impl From<usize> for TickCount {
    fn from(value: usize) -> Self {
        match value {
            50 => TickCount::T50,
            100 => TickCount::T100,
            200 => TickCount::T200,
            500 => TickCount::T500,
            1000 => TickCount::T1000,
            2000 => TickCount::T2000,
            5000 => TickCount::T5000,
            10000 => TickCount::T10000,
            _ => panic!("Invalid tick count value"),
        }
    }
}

impl From<TickCount> for u64 {
    fn from(value: TickCount) -> Self {
        match value {
            TickCount::T50 => 50,
            TickCount::T100 => 100,
            TickCount::T200 => 200,
            TickCount::T500 => 500,
            TickCount::T1000 => 1000,
            TickCount::T2000 => 2000,
            TickCount::T5000 => 5000,
            TickCount::T10000 => 10000,
        }
    }
}

impl std::fmt::Display for TickCount {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TickCount::T50 => write!(f, "T50"),
            TickCount::T100 => write!(f, "T100"),
            TickCount::T200 => write!(f, "T200"),
            TickCount::T500 => write!(f, "T500"),
            TickCount::T1000 => write!(f, "T1000"),
            TickCount::T2000 => write!(f, "T2000"),
            TickCount::T5000 => write!(f, "T5000"),
            TickCount::T10000 => write!(f, "T10000"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TickAccumulation {
    pub tick_count: usize,
    pub open_price: f32,
    pub high_price: f32,
    pub low_price: f32,
    pub close_price: f32,
    pub volume_buy: f32,
    pub volume_sell: f32,
    pub trades: FootprintTrades,
    pub start_timestamp: u64,
}

impl TickAccumulation {
    pub fn get_max_trade_qty(
        &self, 
        highest: OrderedFloat<f32>, 
        lowest: OrderedFloat<f32>,
    ) -> f32 {
        let mut max_qty: f32 = 0.0;
        for (price, (buy_qty, sell_qty)) in &self.trades {
            if price >= &lowest && price <= &highest {
                max_qty = max_qty.max(buy_qty.max(*sell_qty));
            }
        }
        max_qty
    }
}

pub struct TickAggr {
    pub data_points: Vec<TickAccumulation>,
    next_buffer: Vec<Trade>,
    pub aggr_interval: u64,
    pub tick_size: f32,
}

impl TickAggr {
    pub fn new(aggr_interval: u64, tick_size: f32, all_raw_trades: &[Trade]) -> Self {
        if all_raw_trades.is_empty() {
            return Self {
                data_points: Vec::new(),
                next_buffer: Vec::new(),
                aggr_interval,
                tick_size,
            };
        } else {
            let mut tick_aggr = Self {
                data_points: Vec::new(),
                next_buffer: Vec::new(),
                aggr_interval,
                tick_size,
            };
            tick_aggr.insert_trades(all_raw_trades);
            tick_aggr
        }
    }

    pub fn change_tick_size(&mut self, tick_size: f32, all_raw_trades: &[Trade]) {
        self.tick_size = tick_size;
        
        self.data_points.clear();
        self.next_buffer.clear();
        
        if !all_raw_trades.is_empty() {
            self.insert_trades(all_raw_trades);
        }
    }
    
    pub fn get_latest_data_point(&self) -> Option<&TickAccumulation> {
        self.data_points.last()
    }

    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        if buffer.is_empty() && self.next_buffer.is_empty() {
            return;
        }
    
        // Prepare all trades to be processed (next_buffer first, then the new buffer)
        let mut all_trades = Vec::with_capacity(self.next_buffer.len() + buffer.len());
        all_trades.append(&mut self.next_buffer); // Move from next_buffer
        all_trades.extend_from_slice(buffer);     // Add the new buffer
    
        for trade in all_trades {
            if self.data_points.is_empty() || 
                self.data_points.last().unwrap().tick_count >= self.aggr_interval as usize {
                self.data_points.push(TickAccumulation {
                    tick_count: 1,
                    open_price: trade.price,
                    high_price: trade.price,
                    low_price: trade.price,
                    close_price: trade.price,
                    volume_buy: if trade.is_sell { 0.0 } else { trade.qty },
                    volume_sell: if trade.is_sell { trade.qty } else { 0.0 },
                    trades: {
                        let mut trades = HashMap::new();
                        let price_level = OrderedFloat(round_to_tick(trade.price, self.tick_size));
                        if trade.is_sell {
                            trades.insert(price_level, (0.0, trade.qty));
                        } else {
                            trades.insert(price_level, (trade.qty, 0.0));
                        }
                        trades
                    },
                    start_timestamp: trade.time,
                });
                continue;
            }
                
            if let Some(current_accumulation) = self.data_points.last_mut() {
                current_accumulation.tick_count += 1;
                
                current_accumulation.high_price = current_accumulation.high_price.max(trade.price);
                current_accumulation.low_price = current_accumulation.low_price.min(trade.price);
                
                current_accumulation.close_price = trade.price;

                if trade.is_sell {
                    current_accumulation.volume_sell += trade.qty;
                } else {
                    current_accumulation.volume_buy += trade.qty;
                }
                
                let price_level = OrderedFloat(round_to_tick(trade.price, self.tick_size));
                if let Some((buy_qty, sell_qty)) = current_accumulation.trades.get_mut(&price_level) {
                    if trade.is_sell {
                        *sell_qty += trade.qty;
                    } else {
                        *buy_qty += trade.qty;
                    }
                } else if trade.is_sell {
                    current_accumulation.trades.insert(price_level, (0.0, trade.qty));
                } else {
                    current_accumulation.trades.insert(price_level, (trade.qty, 0.0));
                }
            }
        }
    }
}