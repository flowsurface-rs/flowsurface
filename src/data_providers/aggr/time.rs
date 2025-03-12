use std::collections::{BTreeMap, HashMap};

use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

use crate::data_providers::{Kline, Trade};

use super::round_to_tick;

impl std::fmt::Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Timeframe::M1 => "1m",
                Timeframe::M3 => "3m",
                Timeframe::M5 => "5m",
                Timeframe::M15 => "15m",
                Timeframe::M30 => "30m",
                Timeframe::H1 => "1h",
                Timeframe::H2 => "2h",
                Timeframe::H4 => "4h",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Timeframe {
    M1,
    M3,
    M5,
    M15,
    M30,
    H1,
    H2,
    H4,
}

impl Timeframe {
    pub const ALL: [Timeframe; 8] = [
        Timeframe::M1,
        Timeframe::M3,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H2,
        Timeframe::H4,
    ];

    pub fn to_minutes(self) -> u16 {
        match self {
            Timeframe::M1 => 1,
            Timeframe::M3 => 3,
            Timeframe::M5 => 5,
            Timeframe::M15 => 15,
            Timeframe::M30 => 30,
            Timeframe::H1 => 60,
            Timeframe::H2 => 120,
            Timeframe::H4 => 240,
        }
    }

    pub fn to_milliseconds(self) -> u64 {
        u64::from(self.to_minutes()) * 60_000
    }
}

impl From<Timeframe> for f32 {
    fn from(timeframe: Timeframe) -> f32 {
        timeframe.to_milliseconds() as f32
    }
}

impl From<Timeframe> for u64 {
    fn from(timeframe: Timeframe) -> u64 {
        timeframe.to_milliseconds()
    }
}

impl From<u64> for Timeframe {
    fn from(milliseconds: u64) -> Timeframe {
        match milliseconds {
            60_000 => Timeframe::M1,
            180_000 => Timeframe::M3,
            300_000 => Timeframe::M5,
            900_000 => Timeframe::M15,
            1_800_000 => Timeframe::M30,
            3_600_000 => Timeframe::H1,
            7_200_000 => Timeframe::H2,
            14_400_000 => Timeframe::H4,
            _ => panic!("Invalid timeframe: {}", milliseconds),
        }
    }
}

type FootprintTrades = HashMap<OrderedFloat<f32>, (f32, f32)>;

pub struct DataPoint {
    pub kline: Kline,
    pub trades: FootprintTrades,
}

impl DataPoint {
    pub fn get_max_trade_qty(&self, highest: OrderedFloat<f32>, lowest: OrderedFloat<f32>) -> f32 {
        let mut max_qty: f32 = 0.0;
        for (price, (buy_qty, sell_qty)) in &self.trades {
            if price >= &lowest && price <= &highest {
                max_qty = max_qty.max(buy_qty.max(*sell_qty));
            }
        }
        max_qty
    }
}

pub struct TimeSeries {
    pub data_points: BTreeMap<u64, DataPoint>,
    next_buffer: Vec<Trade>,
    pub interval: Timeframe,
    pub tick_size: f32,
}

impl TimeSeries {
    pub fn new(
        interval: Timeframe,
        tick_size: f32,
        raw_trades: &[Trade],
        klines: &[Kline],
    ) -> Self {
        let mut timeseries = Self {
            data_points: BTreeMap::new(),
            next_buffer: Vec::new(),
            interval,
            tick_size,
        };

        for kline in klines {
            let data_point = DataPoint {
                kline: *kline,
                trades: HashMap::new(),
            };
            timeseries.data_points.insert(kline.time, data_point);
        }

        if !raw_trades.is_empty() {
            timeseries.insert_trades(raw_trades, None);
        }

        timeseries
    }

    pub fn get_base_price(&self) -> f32 {
        self.data_points
            .values()
            .last()
            .map(|dp| dp.kline.close)
            .unwrap_or(0.0)
    }

    pub fn get_latest_timestamp(&self) -> Option<u64> {
        self.data_points.keys().last().copied()
    }

    pub fn get_latest_kline(&self) -> Option<&Kline> {
        self.data_points.values().last().map(|dp| &dp.kline)
    }

    pub fn get_price_scale(&self, lookback: usize) -> (f32, f32) {
        let mut scale_high = 0.0f32;
        let mut scale_low = f32::MAX;

        self.data_points
            .iter()
            .rev()
            .take(lookback)
            .for_each(|(_, data_point)| {
                scale_high = scale_high.max(data_point.kline.high);
                scale_low = scale_low.min(data_point.kline.low);
            });

        (scale_high, scale_low)
    }

    pub fn get_volume_data(&self) -> BTreeMap<u64, (f32, f32)> {
        let mut volume_data = BTreeMap::new();
        for (time, data_point) in &self.data_points {
            volume_data.insert(*time, data_point.kline.volume);
        }
        volume_data
    }

    pub fn get_kline_timerange(&self) -> (u64, u64) {
        let earliest = self.data_points.keys().next().copied().unwrap_or(0);
        let latest = self.data_points.keys().last().copied().unwrap_or(0);

        (earliest, latest)
    }

    pub fn change_tick_size(&mut self, tick_size: f32, all_raw_trades: &[Trade]) {
        self.tick_size = tick_size;

        self.clear_trades();
        self.next_buffer.clear();

        if !all_raw_trades.is_empty() {
            self.insert_trades(all_raw_trades, None);
        }
    }

    pub fn insert_klines(&mut self, klines: &[Kline]) {
        if klines.is_empty() {
            return;
        }

        for kline in klines {
            let entry = self
                .data_points
                .entry(kline.time)
                .or_insert_with(|| DataPoint {
                    kline: *kline,
                    trades: HashMap::new(),
                });

            entry.kline = *kline;
        }
    }

    pub fn insert_trades(&mut self, buffer: &[Trade], update_t: Option<u64>) {
        if buffer.is_empty() && self.next_buffer.is_empty() {
            return;
        }

        let aggregate_time = self.interval.to_milliseconds();
        let tick_size = self.tick_size;

        let rounded_update_t = update_t.map(|t| (t / aggregate_time) * aggregate_time);

        for trade in buffer {
            let rounded_time =
                rounded_update_t.unwrap_or((trade.time / aggregate_time) * aggregate_time);
            let price_level = OrderedFloat(round_to_tick(trade.price, tick_size));

            let entry = self
                .data_points
                .entry(rounded_time)
                .or_insert_with(|| DataPoint {
                    kline: Kline::default(),
                    trades: HashMap::new(),
                });

            if let Some((buy_qty, sell_qty)) = entry.trades.get_mut(&price_level) {
                if trade.is_sell {
                    *sell_qty += trade.qty;
                } else {
                    *buy_qty += trade.qty;
                }
            } else if trade.is_sell {
                entry.trades.insert(price_level, (0.0, trade.qty));
            } else {
                entry.trades.insert(price_level, (trade.qty, 0.0));
            }
        }
    }

    pub fn clear_trades(&mut self) {
        for data_point in self.data_points.values_mut() {
            data_point.trades.clear();
        }
    }

    pub fn check_integrity(&self, earliest: u64, latest: u64, interval: u64) -> Option<Vec<u64>> {
        let mut time = earliest;
        let mut missing_count = 0;

        while time < latest {
            if !self.data_points.contains_key(&time) {
                missing_count += 1;
                break;
            }
            time += interval;
        }

        if missing_count > 0 {
            let mut missing_keys = Vec::with_capacity(((latest - earliest) / interval) as usize);
            let mut time = earliest;

            while time < latest {
                if !self.data_points.contains_key(&time) {
                    missing_keys.push(time);
                }
                time += interval;
            }

            log::warn!(
                "Integrity check failed: missing {} klines",
                missing_keys.len()
            );
            return Some(missing_keys);
        }

        None
    }
}
