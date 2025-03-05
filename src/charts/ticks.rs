use std::collections::HashMap;
use ordered_float::OrderedFloat;
use crate::data_providers::Trade;
use super::round_to_tick;

type FootprintTrades = HashMap<OrderedFloat<f32>, (f32, f32)>;

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

    pub fn get_volume(&self) -> (f32, f32) {
        (self.volume_buy, self.volume_sell)
    }
}

pub struct TickAggr {
    pub data_points: Vec<TickAccumulation>,
    next_buffer: Vec<Trade>,
    pub aggr_interval: u64,
    pub tick_size: f32,
}

impl TickAggr {
    pub fn new(aggr_interval: u64, tick_size: f32) -> Self {
        Self {
            data_points: Vec::new(),
            next_buffer: Vec::new(),
            aggr_interval,
            tick_size,
        }
    }

    pub fn set_aggr_interval(&mut self, interval: u64) {
        self.aggr_interval = interval;
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