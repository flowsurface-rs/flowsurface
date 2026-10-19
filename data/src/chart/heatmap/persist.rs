use super::{GroupedTrade, HeatmapDataPoint, HistoricalDepth, HistoryWindow, OrderRun};
use crate::aggr::time::TimeSeries;
use exchange::unit::price::{Price, PriceStep};
use exchange::unit::qty::Qty;
use exchange::{TickerInfo, UnixMs};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"FSHM";
const VERSION: u8 = 1;
const HISTORY_DIR: &str = "heatmap_history";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLoad {
    pub price_levels: usize,
    pub trade_buckets: usize,
}

pub fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// One file per ticker, tick size and aggregation interval, so a different
/// configuration never loads incompatible history.
pub fn history_path(ticker_info: &TickerInfo, step: PriceStep, aggr_ms: u64) -> PathBuf {
    let symbol: String = ticker_info
        .ticker
        .symbol_and_exchange_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();

    crate::data_path(Some(&format!(
        "{HISTORY_DIR}/{symbol}_{}_{aggr_ms}.bin",
        step.units
    )))
}

/// Writes to a temp file and renames, so a crash never leaves a truncated history.
pub fn write_history(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension("tmp");
    let result = std::fs::File::create(&tmp_path).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, path)
    });

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

impl HistoricalDepth {
    /// Serializes depth runs and trade buckets newer than `now - window`.
    pub fn encode_history(
        &self,
        trades: &TimeSeries<HeatmapDataPoint>,
        window: HistoryWindow,
        now: u64,
    ) -> Vec<u8> {
        let oldest = now.saturating_sub(window.to_milliseconds());

        let mut out = Vec::with_capacity(64 * 1024);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        put_u64(&mut out, self.aggr_time_ms());
        put_i64(&mut out, self.tick_size.units);

        let levels: Vec<(&Price, Vec<&OrderRun>)> = self
            .price_levels
            .iter()
            .map(|(price, runs)| {
                let kept = runs
                    .iter()
                    .filter(|run| run.until_time.as_u64() >= oldest)
                    .collect::<Vec<_>>();
                (price, kept)
            })
            .filter(|(_, runs)| !runs.is_empty())
            .collect();

        put_u32(&mut out, levels.len() as u32);
        for (price, runs) in levels {
            put_i64(&mut out, price.units);
            put_u32(&mut out, runs.len() as u32);
            for run in runs {
                put_u64(&mut out, run.start_time.as_u64());
                put_u64(&mut out, run.until_time.as_u64());
                put_i64(&mut out, run.qty.units);
                out.push(u8::from(run.is_bid));
            }
        }

        let buckets: Vec<_> = trades
            .datapoints
            .iter()
            .filter(|(time, _)| time.as_u64() >= oldest)
            .collect();

        put_u32(&mut out, buckets.len() as u32);
        for (time, point) in buckets {
            put_u64(&mut out, time.as_u64());
            put_i64(&mut out, point.buy_sell.0.units);
            put_i64(&mut out, point.buy_sell.1.units);
            put_u32(&mut out, point.grouped_trades.len() as u32);
            for trade in point.grouped_trades.iter() {
                out.push(u8::from(trade.is_sell));
                put_i64(&mut out, trade.price.units);
                put_i64(&mut out, trade.qty.units);
            }
        }

        out
    }

    /// Replaces this history and `trades` with the decoded contents of `bytes`.
    /// Nothing is modified when the data is corrupt or was saved with another
    /// tick size or aggregation interval.
    pub fn load_history(
        &mut self,
        trades: &mut TimeSeries<HeatmapDataPoint>,
        bytes: &[u8],
        window: HistoryWindow,
        now: u64,
    ) -> Result<HistoryLoad, String> {
        let oldest = now.saturating_sub(window.to_milliseconds());
        let mut reader = Reader::new(bytes);

        if reader.take(4)? != MAGIC {
            return Err("not a heatmap history file".into());
        }
        if reader.u8()? != VERSION {
            return Err("unsupported heatmap history version".into());
        }
        if reader.u64()? != self.aggr_time_ms() {
            return Err("aggregation interval differs".into());
        }
        if reader.i64()? != self.tick_size.units {
            return Err("tick size differs".into());
        }

        let level_count = reader.u32()? as usize;
        let mut price_levels = std::collections::BTreeMap::new();
        for _ in 0..level_count {
            let price = Price::from_units(reader.i64()?);
            let run_count = reader.u32()? as usize;
            // each run is 25 bytes; reject counts that cannot possibly fit
            if run_count > reader.remaining() / 25 {
                return Err("truncated heatmap history".into());
            }

            let mut runs = Vec::with_capacity(run_count);
            for _ in 0..run_count {
                let start_time = UnixMs::new(reader.u64()?);
                let until_time = UnixMs::new(reader.u64()?);
                let qty = Qty::from_units(reader.i64()?);
                let is_bid = reader.u8()? != 0;

                if until_time.as_u64() >= oldest {
                    runs.push(OrderRun {
                        start_time,
                        until_time,
                        qty,
                        is_bid,
                    });
                }
            }

            if !runs.is_empty() {
                price_levels.insert(price, runs);
            }
        }

        let bucket_count = reader.u32()? as usize;
        let mut datapoints = std::collections::BTreeMap::new();
        for _ in 0..bucket_count {
            let time = UnixMs::new(reader.u64()?);
            let buy = Qty::from_units(reader.i64()?);
            let sell = Qty::from_units(reader.i64()?);
            let trade_count = reader.u32()? as usize;
            if trade_count > reader.remaining() / 17 {
                return Err("truncated heatmap history".into());
            }

            let mut grouped = Vec::with_capacity(trade_count);
            for _ in 0..trade_count {
                grouped.push(GroupedTrade {
                    is_sell: reader.u8()? != 0,
                    price: Price::from_units(reader.i64()?),
                    qty: Qty::from_units(reader.i64()?),
                });
            }

            if time.as_u64() >= oldest {
                datapoints.insert(
                    time,
                    HeatmapDataPoint {
                        grouped_trades: grouped.into_boxed_slice(),
                        buy_sell: (buy, sell),
                    },
                );
            }
        }

        let loaded = HistoryLoad {
            price_levels: price_levels.len(),
            trade_buckets: datapoints.len(),
        };

        self.price_levels = price_levels;
        // The offline period must stay a gap, not be bridged by the first live snapshot.
        self.last_snapshot_time = None;
        trades.datapoints = datapoints;

        Ok(loaded)
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "truncated heatmap history".to_string())?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes(bytes.try_into().expect("8 bytes")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::Basis;
    use exchange::Timeframe;
    use exchange::depth::Depth;
    use exchange::unit::MinQtySize;
    use exchange::unit::price::PriceStep;
    use std::collections::BTreeMap;

    fn step() -> PriceStep {
        PriceStep { units: 100_000_000 }
    }

    fn new_history() -> (HistoricalDepth, TimeSeries<HeatmapDataPoint>) {
        let basis = Basis::Time(Timeframe::MS1000);
        (
            HistoricalDepth::new(MinQtySize::from(0.001f32), step(), basis),
            TimeSeries::<HeatmapDataPoint>::new(basis, step()),
        )
    }

    fn sample_depth(qty: i64) -> Depth {
        let mut bids = BTreeMap::new();
        let mut asks = BTreeMap::new();
        bids.insert(Price::from_units(100 * 100_000_000), Qty::from_units(qty));
        asks.insert(
            Price::from_units(101 * 100_000_000),
            Qty::from_units(qty * 2),
        );
        Depth { bids, asks }
    }

    fn populated() -> (HistoricalDepth, TimeSeries<HeatmapDataPoint>) {
        let (mut depth, mut trades) = new_history();
        for i in 0..5u64 {
            depth.insert_latest_depth(&sample_depth(10 + i as i64), UnixMs::new(10_000 + i * 1000));
        }
        trades.datapoints.insert(
            UnixMs::new(12_000),
            HeatmapDataPoint {
                grouped_trades: vec![GroupedTrade {
                    is_sell: true,
                    price: Price::from_units(100 * 100_000_000),
                    qty: Qty::from_units(5),
                }]
                .into_boxed_slice(),
                buy_sell: (Qty::from_units(1), Qty::from_units(5)),
            },
        );
        (depth, trades)
    }

    #[test]
    fn roundtrip_preserves_runs_and_trades() {
        let (depth, trades) = populated();
        let bytes = depth.encode_history(&trades, HistoryWindow::H1, 20_000);

        let (mut loaded, mut loaded_trades) = new_history();
        let stats = loaded
            .load_history(&mut loaded_trades, &bytes, HistoryWindow::H1, 20_000)
            .expect("load");

        assert_eq!(stats.price_levels, 2);
        assert_eq!(stats.trade_buckets, 1);
        assert_eq!(loaded.price_levels, depth.price_levels);
        assert_eq!(loaded.last_snapshot_time, None);
        let point = &loaded_trades.datapoints[&UnixMs::new(12_000)];
        assert_eq!(point.buy_sell, (Qty::from_units(1), Qty::from_units(5)));
        assert_eq!(point.grouped_trades.len(), 1);
    }

    #[test]
    fn data_older_than_window_is_dropped() {
        let (depth, trades) = populated();
        let bytes = depth.encode_history(&trades, HistoryWindow::H4, 20_000);

        let (mut loaded, mut loaded_trades) = new_history();
        let now = 20_000 + HistoryWindow::M15.to_milliseconds() + 1;
        let stats = loaded
            .load_history(&mut loaded_trades, &bytes, HistoryWindow::M15, now)
            .expect("load");

        assert_eq!(stats.price_levels, 0);
        assert_eq!(stats.trade_buckets, 0);
        assert!(loaded.is_empty());
    }

    #[test]
    fn mismatched_tick_size_is_rejected() {
        let (depth, trades) = populated();
        let bytes = depth.encode_history(&trades, HistoryWindow::H1, 20_000);

        let basis = Basis::Time(Timeframe::MS1000);
        let other = PriceStep { units: 200_000_000 };
        let mut loaded = HistoricalDepth::new(MinQtySize::from(0.001f32), other, basis);
        let mut loaded_trades = TimeSeries::<HeatmapDataPoint>::new(basis, other);

        assert!(
            loaded
                .load_history(&mut loaded_trades, &bytes, HistoryWindow::H1, 20_000)
                .is_err()
        );
        assert!(loaded.is_empty());
    }

    #[test]
    fn truncated_and_garbage_input_is_rejected() {
        let (depth, trades) = populated();
        let bytes = depth.encode_history(&trades, HistoryWindow::H1, 20_000);

        let (mut loaded, mut loaded_trades) = new_history();
        for cut in [0, 3, 10, bytes.len() / 2, bytes.len() - 1] {
            assert!(
                loaded
                    .load_history(&mut loaded_trades, &bytes[..cut], HistoryWindow::H1, 20_000)
                    .is_err(),
                "cut at {cut}"
            );
        }
        assert!(
            loaded
                .load_history(
                    &mut loaded_trades,
                    b"garbage-bytes",
                    HistoryWindow::H1,
                    20_000
                )
                .is_err()
        );
        assert!(loaded.is_empty());
    }

    #[test]
    fn config_saved_before_history_option_still_loads() {
        let old = r#"{"trade_size_filter":0.0,"order_size_filter":0.0,"trade_size_scale":100,"coalescing":{"Average":0.15}}"#;
        let config: crate::chart::heatmap::Config = serde_json::from_str(old).expect("old config");

        assert!(!config.save_history);
        assert_eq!(config.history_window, HistoryWindow::H1);
    }
}
