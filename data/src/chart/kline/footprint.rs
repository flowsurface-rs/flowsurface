use super::poc::{NPoc, PointOfControl};
use exchange::{
    Trade, UnixMs,
    unit::price::{Price, PriceStep},
    unit::qty::Qty,
};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct GroupedTrades {
    pub buy_qty: Qty,
    pub sell_qty: Qty,
    pub first_time: UnixMs,
    pub last_time: UnixMs,
    pub buy_count: usize,
    pub sell_count: usize,
}

impl GroupedTrades {
    fn new(trade: &Trade) -> Self {
        Self {
            buy_qty: if trade.is_sell {
                Qty::default()
            } else {
                trade.qty
            },
            sell_qty: if trade.is_sell {
                trade.qty
            } else {
                Qty::default()
            },
            first_time: trade.time,
            last_time: trade.time,
            buy_count: if trade.is_sell { 0 } else { 1 },
            sell_count: if trade.is_sell { 1 } else { 0 },
        }
    }

    fn add_trade(&mut self, trade: &Trade) {
        if trade.is_sell {
            self.sell_qty += trade.qty;
            self.sell_count += 1;
        } else {
            self.buy_qty += trade.qty;
            self.buy_count += 1;
        }
        self.first_time = self.first_time.min(trade.time);
        self.last_time = self.last_time.max(trade.time);
    }

    pub fn merge(&mut self, other: &Self) {
        self.buy_qty += other.buy_qty;
        self.sell_qty += other.sell_qty;
        self.buy_count += other.buy_count;
        self.sell_count += other.sell_count;
        self.first_time = self.first_time.min(other.first_time);
        self.last_time = self.last_time.max(other.last_time);
    }

    pub fn total_qty(&self) -> Qty {
        self.buy_qty + self.sell_qty
    }

    pub fn delta_qty(&self) -> Qty {
        self.buy_qty - self.sell_qty
    }

    pub fn max_cluster_qty(&self, cluster_kind: ClusterKind) -> Qty {
        match cluster_kind {
            ClusterKind::BidAsk | ClusterKind::Table => self.buy_qty.max(self.sell_qty),
            ClusterKind::DeltaProfile => self.buy_qty.abs_diff(self.sell_qty),
            ClusterKind::VolumeProfile => self.total_qty(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KlineTrades {
    pub trades: FxHashMap<Price, GroupedTrades>,
    exact_trades: FxHashMap<Price, GroupedTrades>,
    pub poc: Option<PointOfControl>,
}

/// Aggregated directional trade quantities and counts for one footprint.
///
/// Keeping this summary alongside the grouped trades makes the footprint the
/// single source of truth for indicator inputs as well as rendering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TradeSummary {
    pub buy_count: u64,
    pub sell_count: u64,
    pub buy_qty: Qty,
    pub sell_qty: Qty,
}

impl KlineTrades {
    pub fn new() -> Self {
        Self {
            trades: FxHashMap::default(),
            exact_trades: FxHashMap::default(),
            poc: None,
        }
    }

    pub fn first_trade_t(&self) -> Option<UnixMs> {
        self.trades.values().map(|group| group.first_time).min()
    }

    pub fn last_trade_t(&self) -> Option<UnixMs> {
        self.trades.values().map(|group| group.last_time).max()
    }

    pub fn trade_summary(&self) -> Option<TradeSummary> {
        if self.trades.is_empty() {
            return None;
        }

        let mut summary = TradeSummary::default();
        for group in self.trades.values() {
            summary.buy_count = summary.buy_count.saturating_add(group.buy_count as u64);
            summary.sell_count = summary.sell_count.saturating_add(group.sell_count as u64);
            summary.buy_qty += group.buy_qty;
            summary.sell_qty += group.sell_qty;
        }

        Some(summary)
    }

    /// Add trade to the bin at the step multiple computed with side-based rounding.
    /// Intended for order-book ladder/quotes; Floor for sells, ceil for buys.
    /// Introduces side bias at bin edges and should not be used for OHLC/footprint aggregation
    pub fn add_trade_to_side_bin(&mut self, trade: &Trade, step: PriceStep) {
        let price = trade.price.round_to_side_step(trade.is_sell, step);

        self.trades
            .entry(price)
            .and_modify(|group| group.add_trade(trade))
            .or_insert_with(|| GroupedTrades::new(trade));
    }

    /// Add trade to the bin at the nearest step multiple (side-agnostic).
    /// Ties (exactly half a step) round up to the higher multiple.
    /// Intended for footprint/OHLC trade aggregation
    pub fn add_trade_to_nearest_bin(&mut self, trade: &Trade, step: PriceStep) {
        self.exact_trades
            .entry(trade.price)
            .and_modify(|group| group.add_trade(trade))
            .or_insert_with(|| GroupedTrades::new(trade));
        let price = trade.price.round_to_step(step);

        self.trades
            .entry(price)
            .and_modify(|group| group.add_trade(trade))
            .or_insert_with(|| GroupedTrades::new(trade));
    }

    /// Rebins retained exact-price trades without replaying individual trades.
    pub fn regroup(&mut self, step: PriceStep) {
        self.trades.clear();
        self.poc = None;
        for (&price, group) in &self.exact_trades {
            self.trades
                .entry(price.round_to_step(step))
                .and_modify(|current| current.merge(group))
                .or_insert_with(|| group.clone());
        }
        self.calculate_poc();
    }

    /// Max of some extracted qty across price levels within [`lowest`, `highest`].
    fn max_qty_by<F>(&self, highest: Price, lowest: Price, f: F) -> Qty
    where
        F: Fn(&GroupedTrades) -> Qty,
    {
        let mut max_qty = Qty::default();
        for (price, group) in &self.trades {
            if *price >= lowest && *price <= highest {
                max_qty = max_qty.max(f(group));
            }
        }
        max_qty
    }

    /// Max cluster qty considering only price levels within [`lowest`, `highest`].
    /// Used by the aggregate visible-range computation where the viewport may
    /// show only a subset of each bar's full price range (y-pan/zoom).
    pub fn max_cluster_qty(&self, cluster_kind: ClusterKind, highest: Price, lowest: Price) -> Qty {
        self.max_qty_by(highest, lowest, |group| group.max_cluster_qty(cluster_kind))
    }

    /// Max cluster qty across all price levels in this bar (unfiltered).
    /// Used for per-bar individual scaling, the full bar should contribute.
    pub fn max_cluster_qty_all(&self, cluster_kind: ClusterKind) -> Qty {
        self.trades
            .values()
            .map(|group| group.max_cluster_qty(cluster_kind))
            .max()
            .unwrap_or_default()
    }

    /// Selects the highest-volume row and resets its fill status.
    /// For equal positive volumes, chooses the highest price.
    pub fn calculate_poc(&mut self) {
        if self.trades.is_empty() {
            return;
        }

        let mut max_volume = Qty::ZERO;
        let mut poc_price = Price::from_f32(0.0);

        for (price, group) in &self.trades {
            let total_volume = group.total_qty();
            if total_volume > max_volume
                || (total_volume == max_volume && !total_volume.is_zero() && *price > poc_price)
            {
                max_volume = total_volume;
                poc_price = *price;
            }
        }

        self.poc = Some(PointOfControl {
            price: poc_price,
            volume: max_volume,
            status: NPoc::default(),
        });
    }

    pub fn set_poc_status(&mut self, status: NPoc) {
        if let Some(poc) = &mut self.poc {
            poc.status = status;
        }
    }

    pub fn poc_price(&self) -> Option<Price> {
        self.poc.map(|poc| poc.price)
    }

    pub fn clear(&mut self) {
        self.trades.clear();
        self.exact_trades.clear();
        self.poc = None;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum ClusterKind {
    #[default]
    BidAsk,
    VolumeProfile,
    DeltaProfile,
    Table,
}

impl ClusterKind {
    pub const ALL: [ClusterKind; 4] = [
        ClusterKind::BidAsk,
        ClusterKind::VolumeProfile,
        ClusterKind::DeltaProfile,
        ClusterKind::Table,
    ];
}

impl std::fmt::Display for ClusterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterKind::BidAsk => write!(f, "Bid/Ask"),
            ClusterKind::VolumeProfile => write!(f, "Volume Profile"),
            ClusterKind::DeltaProfile => write!(f, "Delta Profile"),
            ClusterKind::Table => write!(f, "Table"),
        }
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub enum ClusterScaling {
    #[default]
    /// Scale based on the maximum quantity in the visible range.
    VisibleRange,
    /// Blend global VisibleRange and per-cluster Individual using a weight in [0.0, 1.0].
    /// weight = fraction of global contribution (1.0 == all-global, 0.0 == all-individual).
    Hybrid { weight: f32 },
    /// Scale based only on the maximum quantity inside the datapoint (per-candle).
    Datapoint,
}

impl ClusterScaling {
    pub const ALL: [ClusterScaling; 3] = [
        ClusterScaling::VisibleRange,
        ClusterScaling::Hybrid { weight: 0.2 },
        ClusterScaling::Datapoint,
    ];

    /// Blend the global visible-range max qty with the per-candle individual max qty
    /// according to the scaling strategy.
    pub fn effective_qty(self, visible_max: f64, individual_max: Qty) -> f64 {
        match self {
            ClusterScaling::VisibleRange => Qty::scale_or_one(visible_max),
            ClusterScaling::Datapoint => individual_max.to_scale_or_one(),
            ClusterScaling::Hybrid { weight } => {
                let w = weight.clamp(0.0, 1.0) as f64;
                Qty::scale_or_one(visible_max * w + individual_max.to_f64() * (1.0 - w))
            }
        }
    }
}

impl std::fmt::Display for ClusterScaling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClusterScaling::VisibleRange => write!(f, "Visible Range"),
            ClusterScaling::Hybrid { weight } => write!(f, "Hybrid (weight: {:.2})", weight),
            ClusterScaling::Datapoint => write!(f, "Per-candle"),
        }
    }
}

impl std::cmp::Eq for ClusterScaling {}
