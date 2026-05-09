#![allow(dead_code)]

use super::YUnit;
use exchange::TickerInfo;
use exchange::unit::{Power10, Price, Qty};
use std::cmp::Ordering;
use std::collections::BTreeSet;

pub const DEFAULT_MIN_PANEL_RATIO: f32 = 0.08;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PanelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRole {
    Primary,
    Auxiliary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Candle,
    Bar(BarMode),
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerDataKind {
    Ohlc,
    Scalar,
    Histogram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistogramMode {
    Plain,
    SignedOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarMode {
    Regular,
    Histogram(HistogramMode),
}

pub const fn default_bar_mode_for_data_kind(data_kind: LayerDataKind) -> BarMode {
    match data_kind {
        LayerDataKind::Histogram => BarMode::Histogram(HistogramMode::Plain),
        LayerDataKind::Ohlc | LayerDataKind::Scalar => BarMode::Regular,
    }
}

pub const fn default_mark_for_data_kind(data_kind: LayerDataKind) -> MarkKind {
    match data_kind {
        LayerDataKind::Ohlc => MarkKind::Candle,
        LayerDataKind::Scalar => MarkKind::Line,
        LayerDataKind::Histogram => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
    }
}

const fn supports_mark(data_kind: LayerDataKind, mark: MarkKind) -> bool {
    match data_kind {
        LayerDataKind::Ohlc => matches!(mark, MarkKind::Line | MarkKind::Candle),
        LayerDataKind::Scalar => matches!(mark, MarkKind::Line | MarkKind::Bar(_)),
        LayerDataKind::Histogram => matches!(mark, MarkKind::Line | MarkKind::Bar(_)),
    }
}

const fn supports_bar_mode(
    data_kind: LayerDataKind,
    mode: BarMode,
    signed_overlay_input: bool,
) -> bool {
    match mode {
        BarMode::Regular => true,
        BarMode::Histogram(HistogramMode::Plain) => matches!(data_kind, LayerDataKind::Histogram),
        BarMode::Histogram(HistogramMode::SignedOverlay) => {
            matches!(data_kind, LayerDataKind::Histogram) && signed_overlay_input
        }
    }
}

pub fn resolve_mark_for_data_kind(
    mark: MarkKind,
    data_kind: LayerDataKind,
    signed_overlay_input: bool,
) -> MarkKind {
    let mut resolved = if supports_mark(data_kind, mark) {
        mark
    } else {
        default_mark_for_data_kind(data_kind)
    };

    if let MarkKind::Bar(mode) = resolved
        && !supports_bar_mode(data_kind, mode, signed_overlay_input)
    {
        resolved = MarkKind::Bar(default_bar_mode_for_data_kind(data_kind));
    }

    resolved
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisBinding {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DataSourceId {
    Primary,
    Symbol(&'static str),
    Synthetic(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelScaleMode {
    Absolute,
    FitVisible,
    FitVisibleIncludeZero,
    Logarithmic,
    PercentFromBase,
}

impl PanelScaleMode {
    pub(super) fn uses_percent_base(self) -> bool {
        matches!(self, Self::PercentFromBase)
    }

    pub(super) fn uses_series_percent_anchors(self, series_count: usize) -> bool {
        self.uses_percent_base() && series_count > 1
    }

    pub(super) fn primary_autoscale_enabled(self, user_autoscale_enabled: bool) -> bool {
        user_autoscale_enabled || matches!(self, Self::PercentFromBase)
    }

    pub(super) fn allows_primary_manual_pan(self, user_autoscale_enabled: bool) -> bool {
        !self.primary_autoscale_enabled(user_autoscale_enabled)
    }

    pub(super) fn allows_primary_manual_zoom(self) -> bool {
        !matches!(self, Self::PercentFromBase)
    }

    pub(super) fn display_anchor(
        self,
        explicit_anchor: Option<f32>,
        default_anchor: Option<f32>,
    ) -> Option<f32> {
        if self.uses_percent_base() {
            explicit_anchor
        } else {
            default_anchor
        }
    }

    pub(super) fn format_percent_label(
        self,
        display_value: f32,
        display_step: f32,
    ) -> Option<String> {
        if !self.uses_percent_base() {
            return None;
        }

        let precision = Self::percent_label_precision(display_step);
        Some(format!("{display_value:.precision$}%"))
    }

    pub(super) fn percent_display_step(
        self,
        value_step: Option<f32>,
        anchor: Option<f32>,
        fallback: f32,
    ) -> Option<f32> {
        if !self.uses_percent_base() {
            return None;
        }

        Some(
            value_step
                .and_then(|step| {
                    anchor
                        .filter(|anchor| anchor.abs() > f32::EPSILON)
                        .map(|anchor| (step / anchor.abs()) * 100.0)
                })
                .unwrap_or(fallback)
                .abs()
                .max(1e-6),
        )
    }

    pub(super) fn log_axis_value_and_step(
        self,
        display_value: f32,
        display_step: f32,
        anchor: Option<f32>,
        log_scale_enabled: bool,
    ) -> Option<(f32, f32)> {
        if !matches!(self, Self::Logarithmic) || !log_scale_enabled {
            return None;
        }

        let value = self.display_to_value(display_value, anchor, true);
        let next_value =
            self.display_to_value(display_value + display_step.abs().max(1e-3), anchor, true);
        let value_step = (next_value - value).abs().max(1e-6);

        Some((value, value_step))
    }

    pub(super) fn uses_display_transform(self, log_scale_enabled: bool) -> bool {
        matches!(self, Self::PercentFromBase)
            || (log_scale_enabled && matches!(self, Self::Logarithmic))
    }

    pub(super) fn value_to_display(
        self,
        value: f32,
        anchor: Option<f32>,
        log_scale_enabled: bool,
    ) -> f32 {
        if log_scale_enabled && matches!(self, Self::Logarithmic) {
            value.max(f32::MIN_POSITIVE).log10()
        } else {
            match (self, anchor) {
                (Self::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    ((value / base) - 1.0) * 100.0
                }
                _ => value,
            }
        }
    }

    pub(super) fn display_to_value(
        self,
        display_value: f32,
        anchor: Option<f32>,
        log_scale_enabled: bool,
    ) -> f32 {
        if log_scale_enabled && matches!(self, Self::Logarithmic) {
            10_f32.powf(display_value).max(f32::MIN_POSITIVE)
        } else {
            match (self, anchor) {
                (Self::PercentFromBase, Some(base)) if base.abs() > f32::EPSILON => {
                    base * (1.0 + display_value / 100.0)
                }
                _ => display_value,
            }
        }
    }

    pub(super) fn percent_label_precision(display_step: f32) -> usize {
        let step = display_step.abs().max(1e-6);
        if step >= 1.0 {
            1
        } else if step >= 0.1 {
            2
        } else if step >= 0.01 {
            3
        } else {
            4
        }
    }

    fn fit_visible_bounds(min_unit: i64, max_unit: i64) -> (YUnit, YUnit) {
        let span = max_unit.abs_diff(min_unit);
        let pad = if span == 0 {
            max_unit.unsigned_abs().max(1).saturating_add(49) / 50
        } else {
            span.saturating_mul(5).saturating_add(99) / 100
        }
        .max(1);
        let pad = i64::try_from(pad).unwrap_or(i64::MAX);

        let padded_min = min_unit.saturating_sub(pad);
        let padded_max = max_unit.saturating_add(pad);
        (YUnit(padded_min), YUnit(padded_max))
    }

    pub(super) fn indicator_domain_units(self, min_unit: i64, max_unit: i64) -> (YUnit, YUnit) {
        match self {
            Self::FitVisible => Self::fit_visible_bounds(min_unit, max_unit),
            Self::FitVisibleIncludeZero => {
                Self::fit_visible_bounds(min_unit.min(0), max_unit.max(0))
            }
            _ => (YUnit(0), YUnit(max_unit.max(1))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelValueId {
    Volume,
    BollingerBands,
    Rsi,
    OpenInterest,
    CumulativeVolumeDelta,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PanelValuePrecision {
    BaseTickerMinTick,
    BaseTickerMinQty,
    FixedPower10(Power10<-8, 8>),
    FixedStep(f32),
}

impl PanelValuePrecision {
    const MIN_STEP: f32 = 1e-8;
    const DEFAULT_FALLBACK_STEP: f32 = 1e-4;

    pub(super) fn quantization_step(self, base_ticker: Option<TickerInfo>) -> Option<f32> {
        let step = match self {
            Self::BaseTickerMinTick => base_ticker.map(|ticker| ticker.min_ticksize.as_f32()),
            Self::BaseTickerMinQty => base_ticker.map(|ticker| ticker.min_qty.as_f32()),
            Self::FixedPower10(step) => Some(step.as_f32()),
            Self::FixedStep(step) => Some(step),
        }?;

        Some(step.abs().max(Self::MIN_STEP))
    }

    fn decimals_from_power10(power: i8) -> usize {
        if power < 0 { (-power) as usize } else { 0 }
    }

    pub(super) fn decimals_for_step(step: f32) -> usize {
        let step = step.abs();
        if !step.is_finite() || step <= 0.0 {
            return 4;
        }

        for decimals in 0..=8 {
            let scaled = (step as f64) * 10_f64.powi(decimals as i32);
            let nearest = scaled.round();
            let tolerance = (scaled.abs() * 1e-9).max(1e-12);

            if (scaled - nearest).abs() <= tolerance {
                return decimals;
            }
        }

        8
    }

    pub(super) fn decimals(self, base_ticker: Option<TickerInfo>) -> Option<usize> {
        match self {
            Self::BaseTickerMinTick => {
                base_ticker.map(|ticker| Self::decimals_from_power10(ticker.min_ticksize.power))
            }
            Self::BaseTickerMinQty => {
                base_ticker.map(|ticker| Self::decimals_from_power10(ticker.min_qty.power))
            }
            Self::FixedPower10(step) => Some(Self::decimals_from_power10(step.power)),
            Self::FixedStep(step) => Some(Self::decimals_for_step(step)),
        }
    }

    fn pow10_i64(exp: i32) -> Option<i64> {
        if exp < 0 {
            return None;
        }

        10_i64.checked_pow(exp as u32)
    }

    fn round_to_i64_saturating(value: f32) -> i64 {
        if !value.is_finite() {
            return 0;
        }

        let rounded = value.round();
        if rounded > i64::MAX as f32 {
            i64::MAX
        } else if rounded < i64::MIN as f32 {
            i64::MIN
        } else {
            rounded as i64
        }
    }

    pub(super) fn value_to_unit(
        self,
        value: f32,
        base_ticker: Option<TickerInfo>,
        fallback_step: f32,
    ) -> YUnit {
        if !value.is_finite() {
            return YUnit(0);
        }

        match self {
            Self::BaseTickerMinTick => {
                if let Some(ticker) = base_ticker {
                    let min_tick = ticker.min_ticksize;
                    let exp = 8 + i32::from(min_tick.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let price_units = Price::from_f32(value).round_to_min_tick(min_tick).units;
                        return YUnit(price_units.div_euclid(unit_size));
                    }
                }
            }
            Self::BaseTickerMinQty => {
                if let Some(ticker) = base_ticker {
                    let min_qty = ticker.min_qty;
                    let exp = Qty::QTY_SCALE + i32::from(min_qty.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let qty_units = Qty::from_f32(value).round_to_min_qty(min_qty).units;
                        return YUnit(qty_units.div_euclid(unit_size));
                    }
                }
            }
            Self::FixedPower10(step) => {
                let step = step.as_f32().abs().max(Self::MIN_STEP);
                return YUnit(Self::round_to_i64_saturating(value / step));
            }
            Self::FixedStep(step) => {
                let step = step.abs().max(Self::MIN_STEP);
                return YUnit(Self::round_to_i64_saturating(value / step));
            }
        }

        let step = self
            .quantization_step(base_ticker)
            .unwrap_or(fallback_step.abs().max(Self::MIN_STEP));
        YUnit(Self::round_to_i64_saturating(value / step))
    }

    pub(super) fn unit_to_value(
        self,
        y_unit: YUnit,
        base_ticker: Option<TickerInfo>,
        fallback_step: f32,
    ) -> f32 {
        match self {
            Self::BaseTickerMinTick => {
                if let Some(ticker) = base_ticker {
                    let min_tick = ticker.min_ticksize;
                    let exp = 8 + i32::from(min_tick.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let units = y_unit.0.saturating_mul(unit_size);
                        return Price::from_units(units).to_f32();
                    }
                }
            }
            Self::BaseTickerMinQty => {
                if let Some(ticker) = base_ticker {
                    let min_qty = ticker.min_qty;
                    let exp = Qty::QTY_SCALE + i32::from(min_qty.power);

                    if let Some(unit_size) = Self::pow10_i64(exp) {
                        let units = y_unit.0.saturating_mul(unit_size);
                        return f32::from(Qty::from_units(units));
                    }
                }
            }
            Self::FixedPower10(step) => {
                return (y_unit.0 as f32) * step.as_f32().abs().max(Self::MIN_STEP);
            }
            Self::FixedStep(step) => {
                return (y_unit.0 as f32) * step.abs().max(Self::MIN_STEP);
            }
        }

        let step = self
            .quantization_step(base_ticker)
            .unwrap_or(fallback_step.abs().max(Self::MIN_STEP));
        (y_unit.0 as f32) * step
    }

    pub(super) fn quantize_value(
        self,
        value: f32,
        base_ticker: Option<TickerInfo>,
        fallback_step: f32,
    ) -> f32 {
        let y_unit = self.value_to_unit(value, base_ticker, fallback_step);
        self.unit_to_value(y_unit, base_ticker, fallback_step)
    }

    pub(super) fn format_compact(
        self,
        value: f32,
        fallback_step: f32,
        max_decimals: Option<u8>,
        base_ticker: Option<TickerInfo>,
    ) -> String {
        let quantized = self.quantize_value(value, base_ticker, Self::DEFAULT_FALLBACK_STEP);
        let fallback = fallback_step.abs().max(1e-6);
        let step = self
            .quantization_step(base_ticker)
            .map(|panel_step| panel_step.max(fallback))
            .unwrap_or(fallback);

        let decimals = self
            .decimals(base_ticker)
            .unwrap_or_else(|| Self::decimals_for_step(step));
        let decimals = max_decimals
            .map(|max| decimals.min(max as usize))
            .unwrap_or(decimals);
        format!("{quantized:.decimals$}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelValueLabelMode {
    Compact,
    Commas,
    Abbreviated,
}

impl PanelValueLabelMode {
    pub(super) fn format_value<FQ, FC>(self, quantized_value: FQ, compact_value: FC) -> String
    where
        FQ: FnOnce() -> f32,
        FC: FnOnce() -> String,
    {
        match self {
            Self::Compact => compact_value(),
            Self::Commas => data::util::format_with_commas(quantized_value()),
            Self::Abbreviated => data::util::abbr_large_numbers(quantized_value()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelValueLabelPolicy {
    pub axis_mode: PanelValueLabelMode,
    pub header_mode: PanelValueLabelMode,
    pub max_decimals: Option<u8>,
}

impl Default for PanelValueLabelPolicy {
    fn default() -> Self {
        Self {
            axis_mode: PanelValueLabelMode::Compact,
            header_mode: PanelValueLabelMode::Commas,
            max_decimals: None,
        }
    }
}

impl PanelValueLabelPolicy {
    pub(super) fn format_axis_value<FQ, FC>(self, quantized_value: FQ, compact_value: FC) -> String
    where
        FQ: FnOnce() -> f32,
        FC: FnOnce() -> String,
    {
        self.axis_mode.format_value(quantized_value, compact_value)
    }

    pub(super) fn format_header_value<FQ, FC>(
        self,
        quantized_value: FQ,
        compact_value: FC,
    ) -> String
    where
        FQ: FnOnce() -> f32,
        FC: FnOnce() -> String,
    {
        self.header_mode
            .format_value(quantized_value, compact_value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelComparisonPolicy {
    pub force_percent_scale_on_multi_source: bool,
    pub force_line_for_non_base_sources: bool,
}

impl Default for PanelComparisonPolicy {
    fn default() -> Self {
        Self {
            force_percent_scale_on_multi_source: true,
            force_line_for_non_base_sources: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerSource {
    RawKline {
        source: DataSourceId,
    },
    RawIndicator {
        source: DataSourceId,
        value_id: PanelValueId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDataHint {
    ValueLike,
    HistogramLike,
}

#[derive(Debug, Clone)]
pub struct LayerStyle {
    pub line_width: f32,
    pub opacity: f32,
}

impl Default for LayerStyle {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayerSpec {
    pub id: LayerId,
    pub name: String,
    pub source: LayerSource,
    pub data_kind: LayerDataKind,
    pub mark: MarkKind,
    pub axis: AxisBinding,
    pub visible: bool,
    pub style: LayerStyle,
}

impl LayerSpec {
    pub fn is_histogram_like(&self) -> bool {
        matches!(self.axis, AxisBinding::Secondary)
            || matches!(self.data_kind, LayerDataKind::Histogram)
    }

    pub fn source_id(&self) -> Option<DataSourceId> {
        self.source.source_id()
    }
}

impl LayerSource {
    pub fn source_id(&self) -> Option<DataSourceId> {
        match self {
            Self::RawKline { source } => Some(*source),
            Self::RawIndicator { source, .. } => Some(*source),
        }
    }

    pub fn indicator_value_id(&self) -> Option<PanelValueId> {
        match self {
            Self::RawIndicator { value_id, .. } => Some(*value_id),
            Self::RawKline { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PanelSpec {
    pub id: PanelId,
    pub role: PanelRole,
    pub title: Option<String>,
    pub value_id: Option<PanelValueId>,
    pub value_precision: Option<PanelValuePrecision>,
    pub value_label_policy: PanelValueLabelPolicy,
    pub base_layer: Option<LayerId>,
    pub preferred_scale: PanelScaleMode,
    pub comparison_policy: PanelComparisonPolicy,
    pub layers: Vec<LayerSpec>,
}

impl PanelSpec {
    pub fn data_hint(&self) -> PanelDataHint {
        if self.layers.iter().any(LayerSpec::is_histogram_like) {
            PanelDataHint::HistogramLike
        } else {
            PanelDataHint::ValueLike
        }
    }

    pub fn source_count(&self) -> usize {
        self.layers
            .iter()
            .filter_map(LayerSpec::source_id)
            .collect::<BTreeSet<DataSourceId>>()
            .len()
    }

    pub fn uses_multi_source(&self) -> bool {
        self.source_count() > 1
    }

    pub fn effective_scale_mode(&self) -> PanelScaleMode {
        if self.comparison_policy.force_percent_scale_on_multi_source && self.uses_multi_source() {
            PanelScaleMode::PercentFromBase
        } else {
            self.preferred_scale
        }
    }

    pub fn set_base_layer(&mut self, layer_id: LayerId) -> bool {
        if self.layers.iter().any(|layer| layer.id == layer_id) {
            self.base_layer = Some(layer_id);
            self.enforce_comparison_mark_policy();
            true
        } else {
            false
        }
    }

    pub fn set_layer_mark(&mut self, layer_id: LayerId, mark: MarkKind) -> bool {
        let is_multi_source = self.uses_multi_source();
        let force_line = self.comparison_policy.force_line_for_non_base_sources
            && is_multi_source
            && self.base_layer != Some(layer_id);

        let Some(layer) = self.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        layer.mark = if force_line { MarkKind::Line } else { mark };
        true
    }

    pub fn enforce_comparison_mark_policy(&mut self) {
        if !(self.comparison_policy.force_line_for_non_base_sources && self.uses_multi_source()) {
            return;
        }

        for layer in &mut self.layers {
            if Some(layer.id) != self.base_layer {
                layer.mark = MarkKind::Line;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChartComposition {
    pub panels: Vec<PanelSpec>,
    /// Normalized boundaries in ascending order, one fewer than panel count.
    pub splits: Vec<f32>,
    next_panel_id: u32,
    next_layer_id: u32,
}

impl ChartComposition {
    pub fn prototype_kline() -> Self {
        let mut composition = Self {
            panels: Vec::new(),
            splits: Vec::new(),
            next_panel_id: 1,
            next_layer_id: 1,
        };

        let candle_layer = composition.new_layer(
            "Candles",
            LayerSource::RawKline {
                source: DataSourceId::Primary,
            },
            LayerDataKind::Ohlc,
            MarkKind::Candle,
            AxisBinding::Primary,
        );

        let main_panel_id = composition.new_panel_id();
        composition.panels.push(PanelSpec {
            id: main_panel_id,
            role: PanelRole::Primary,
            title: None,
            value_id: None,
            value_precision: Some(PanelValuePrecision::BaseTickerMinTick),
            value_label_policy: PanelValueLabelPolicy::default(),
            base_layer: Some(candle_layer.id),
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers: vec![candle_layer],
        });

        composition.ensure_split_count();
        composition.splits = composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        composition
    }

    pub fn panel_count(&self) -> usize {
        self.panels.len()
    }

    pub fn primary_panel_id(&self) -> Option<PanelId> {
        self.panels
            .iter()
            .find(|panel| matches!(panel.role, PanelRole::Primary))
            .map(|panel| panel.id)
    }

    pub fn split_count(&self) -> usize {
        self.panels.len().saturating_sub(1)
    }

    pub fn panel_data_hints(&self) -> Vec<PanelDataHint> {
        self.panels.iter().map(PanelSpec::data_hint).collect()
    }

    pub fn panel(&self, panel_id: PanelId) -> Option<&PanelSpec> {
        self.panels.iter().find(|panel| panel.id == panel_id)
    }

    pub fn panel_mut(&mut self, panel_id: PanelId) -> Option<&mut PanelSpec> {
        self.panels.iter_mut().find(|panel| panel.id == panel_id)
    }

    pub fn panel_effective_scale_mode(&self, panel_id: PanelId) -> Option<PanelScaleMode> {
        self.panel(panel_id).map(PanelSpec::effective_scale_mode)
    }

    pub fn panel_effective_data_kind(&self, panel_id: PanelId) -> Option<LayerDataKind> {
        let panel = self.panel(panel_id)?;

        let fallback = match panel.role {
            PanelRole::Primary => LayerDataKind::Ohlc,
            PanelRole::Auxiliary => LayerDataKind::Scalar,
        };

        panel
            .base_layer
            .and_then(|base| panel.layers.iter().find(|layer| layer.id == base))
            .or_else(|| panel.layers.first())
            .map(|layer| layer.data_kind)
            .or(Some(fallback))
    }

    pub fn panel_effective_mark_with_runtime(
        &self,
        panel_id: PanelId,
        signed_overlay_input: bool,
    ) -> Option<MarkKind> {
        let panel = self.panel(panel_id)?;

        let fallback = match panel.role {
            PanelRole::Primary => default_mark_for_data_kind(LayerDataKind::Ohlc),
            PanelRole::Auxiliary => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        };

        let base_layer_id = panel
            .base_layer
            .or_else(|| panel.layers.first().map(|layer| layer.id));

        self.resolved_panel_marks_with_runtime(panel_id, signed_overlay_input)
            .and_then(|resolved_marks| {
                base_layer_id
                    .and_then(|base| {
                        resolved_marks
                            .iter()
                            .find(|(layer_id, _)| *layer_id == base)
                            .map(|(_, mark)| *mark)
                    })
                    .or_else(|| resolved_marks.first().map(|(_, mark)| *mark))
            })
            .or(Some(fallback))
    }

    pub fn panel_value_id(&self, panel_id: PanelId) -> Option<PanelValueId> {
        self.panel(panel_id).and_then(|panel| panel.value_id)
    }

    pub fn panel_value_precision(&self, panel_id: PanelId) -> Option<PanelValuePrecision> {
        self.panel(panel_id).and_then(|panel| panel.value_precision)
    }

    pub fn panel_value_label_policy(&self, panel_id: PanelId) -> Option<PanelValueLabelPolicy> {
        self.panel(panel_id).map(|panel| panel.value_label_policy)
    }

    pub fn normalized_splits(&self, min_panel_ratio: f32) -> Vec<f32> {
        let panel_count = self.panel_count();
        let split_count = panel_count.saturating_sub(1);

        if split_count == 0 {
            return Vec::new();
        }

        let min_ratio = if panel_count == 0 {
            0.0
        } else {
            min_panel_ratio.clamp(0.0, 1.0 / panel_count as f32)
        };

        let mut splits = Vec::with_capacity(split_count);
        for index in 0..split_count {
            let fallback = (index + 1) as f32 / panel_count as f32;
            splits.push(self.splits.get(index).copied().unwrap_or(fallback));
        }

        for index in 0..split_count {
            let remaining_panels_after = panel_count.saturating_sub(index + 1);
            let lower = if index > 0 {
                splits[index - 1] + min_ratio
            } else {
                min_ratio
            };
            let upper = 1.0 - (remaining_panels_after as f32 * min_ratio);

            let (min_bound, max_bound) = if lower <= upper {
                (lower, upper)
            } else {
                (upper, lower)
            };

            splits[index] = splits[index].clamp(min_bound, max_bound);
        }

        splits
    }

    pub fn set_split(&mut self, split_index: usize, split: f32, min_panel_ratio: f32) -> bool {
        if split_index >= self.split_count() {
            return false;
        }

        self.ensure_split_count();
        let mut splits = self.normalized_splits(min_panel_ratio);

        let panel_count = self.panel_count();
        let min_ratio = if panel_count == 0 {
            0.0
        } else {
            min_panel_ratio.clamp(0.0, 1.0 / panel_count as f32)
        };

        let lower = if split_index > 0 {
            splits[split_index - 1] + min_ratio
        } else {
            min_ratio
        };

        let upper = if split_index + 1 < splits.len() {
            splits[split_index + 1] - min_ratio
        } else {
            1.0 - min_ratio
        };

        let (min_bound, max_bound) = if lower <= upper {
            (lower, upper)
        } else {
            (upper, lower)
        };

        let new_value = split.clamp(min_bound, max_bound);
        if let Some(target) = splits.get_mut(split_index) {
            *target = new_value;
        }

        self.splits = splits;
        true
    }

    pub fn add_aux_panel(
        &mut self,
        title: impl Into<String>,
        layers: Vec<LayerSpec>,
        value_precision: Option<PanelValuePrecision>,
        value_label_policy: PanelValueLabelPolicy,
    ) -> PanelId {
        let panel_id = self.new_panel_id();
        let base_layer = layers.first().map(|layer| layer.id);
        self.panels.push(PanelSpec {
            id: panel_id,
            role: PanelRole::Auxiliary,
            title: Some(title.into()),
            value_id: None,
            value_precision,
            value_label_policy,
            base_layer,
            preferred_scale: PanelScaleMode::Absolute,
            comparison_policy: PanelComparisonPolicy::default(),
            layers,
        });

        self.ensure_split_count();
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        panel_id
    }

    pub fn add_layer_to_panel(&mut self, panel_id: PanelId, layer: LayerSpec) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let layer_id = layer.id;
        panel.layers.push(layer);
        if panel.base_layer.is_none() {
            panel.base_layer = Some(layer_id);
        }
        panel.enforce_comparison_mark_policy();

        true
    }

    pub fn add_comparison_source_to_panel(
        &mut self,
        panel_id: PanelId,
        source: DataSourceId,
        name: impl Into<String>,
    ) -> Option<LayerId> {
        let data_hint = self.panel(panel_id)?.data_hint();

        let (data_kind, axis) = match data_hint {
            PanelDataHint::ValueLike => (LayerDataKind::Scalar, AxisBinding::Primary),
            PanelDataHint::HistogramLike => (LayerDataKind::Histogram, AxisBinding::Secondary),
        };

        let layer = self.new_layer(
            name,
            LayerSource::RawKline { source },
            data_kind,
            MarkKind::Line,
            axis,
        );

        let layer_id = layer.id;
        self.add_layer_to_panel(panel_id, layer).then_some(layer_id)
    }

    pub fn set_panel_base_layer(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.set_base_layer(layer_id)
    }

    pub fn set_panel_preferred_scale(&mut self, panel_id: PanelId, scale: PanelScaleMode) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.preferred_scale = scale;
        true
    }

    pub fn set_panel_value_id(
        &mut self,
        panel_id: PanelId,
        value_id: Option<PanelValueId>,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.value_id = value_id;
        true
    }

    pub fn set_panel_value_precision(
        &mut self,
        panel_id: PanelId,
        value_precision: Option<PanelValuePrecision>,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.value_precision = value_precision;
        true
    }

    pub fn set_panel_value_label_policy(
        &mut self,
        panel_id: PanelId,
        value_label_policy: PanelValueLabelPolicy,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.value_label_policy = value_label_policy;
        true
    }

    pub fn set_panel_comparison_policy(
        &mut self,
        panel_id: PanelId,
        comparison_policy: PanelComparisonPolicy,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.comparison_policy = comparison_policy;
        panel.enforce_comparison_mark_policy();
        true
    }

    pub fn set_panel_layer_mark(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        mark: MarkKind,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        panel.set_layer_mark(layer_id, mark)
    }

    pub fn set_panel_layer_data_kind(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        data_kind: LayerDataKind,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let Some(layer) = panel.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        layer.data_kind = data_kind;
        if matches!(layer.mark, MarkKind::Bar(_)) {
            layer.mark = MarkKind::Bar(default_bar_mode_for_data_kind(data_kind));
        }
        true
    }

    pub fn set_panel_layer_bar_mode(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        mode: BarMode,
    ) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let Some(layer) = panel.layers.iter_mut().find(|layer| layer.id == layer_id) else {
            return false;
        };

        layer.mark = MarkKind::Bar(mode);
        panel.enforce_comparison_mark_policy();
        true
    }

    pub fn set_panel_layer_histogram_mode(
        &mut self,
        panel_id: PanelId,
        layer_id: LayerId,
        mode: HistogramMode,
    ) -> bool {
        self.set_panel_layer_bar_mode(panel_id, layer_id, BarMode::Histogram(mode))
    }

    pub fn resolved_panel_marks_with_runtime(
        &self,
        panel_id: PanelId,
        signed_overlay_input: bool,
    ) -> Option<Vec<(LayerId, MarkKind)>> {
        let panel = self.panel(panel_id)?;
        let is_multi_source = panel.uses_multi_source();

        Some(
            panel
                .layers
                .iter()
                .map(|layer| {
                    let mut mark = layer.mark;

                    if panel.comparison_policy.force_line_for_non_base_sources
                        && is_multi_source
                        && panel.base_layer != Some(layer.id)
                    {
                        mark = MarkKind::Line;
                    }

                    (
                        layer.id,
                        resolve_mark_for_data_kind(mark, layer.data_kind, signed_overlay_input),
                    )
                })
                .collect(),
        )
    }

    pub fn remove_layer_from_panel(&mut self, panel_id: PanelId, layer_id: LayerId) -> bool {
        let Some(panel) = self.panel_mut(panel_id) else {
            return false;
        };

        let Some(index) = panel.layers.iter().position(|layer| layer.id == layer_id) else {
            return false;
        };

        panel.layers.remove(index);

        if panel.base_layer == Some(layer_id) {
            panel.base_layer = panel.layers.first().map(|layer| layer.id);
        }

        panel.enforce_comparison_mark_policy();
        true
    }

    pub fn resolved_panel_marks(&self, panel_id: PanelId) -> Option<Vec<(LayerId, MarkKind)>> {
        self.resolved_panel_marks_with_runtime(panel_id, false)
    }

    pub fn remove_panel(&mut self, panel_id: PanelId) -> bool {
        let Some(index) = self.panels.iter().position(|panel| panel.id == panel_id) else {
            return false;
        };

        if matches!(self.panels[index].role, PanelRole::Primary) {
            return false;
        }

        self.panels.remove(index);
        self.ensure_split_count();
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    pub fn move_panel(&mut self, from_index: usize, to_index: usize) -> bool {
        let len = self.panels.len();
        if from_index >= len || to_index >= len || from_index == to_index {
            return false;
        }

        self.ensure_split_count();
        let mut heights = self.normalized_panel_heights(DEFAULT_MIN_PANEL_RATIO);

        let panel = self.panels.remove(from_index);
        let panel_height = heights.remove(from_index);

        self.panels.insert(to_index, panel);
        heights.insert(to_index, panel_height);

        self.splits = Self::splits_from_heights(&heights);
        self.splits = self.normalized_splits(DEFAULT_MIN_PANEL_RATIO);
        true
    }

    pub fn new_layer(
        &mut self,
        name: impl Into<String>,
        source: LayerSource,
        data_kind: LayerDataKind,
        mark: MarkKind,
        axis: AxisBinding,
    ) -> LayerSpec {
        LayerSpec {
            id: self.new_layer_id(),
            name: name.into(),
            source,
            data_kind,
            mark,
            axis,
            visible: true,
            style: LayerStyle::default(),
        }
    }

    fn ensure_split_count(&mut self) {
        let target = self.split_count();
        match self.splits.len().cmp(&target) {
            Ordering::Equal => {}
            Ordering::Greater => self.splits.truncate(target),
            Ordering::Less => {
                for index in self.splits.len()..target {
                    let fallback = (index + 1) as f32 / (target + 1) as f32;
                    self.splits.push(fallback);
                }
            }
        }
    }

    fn normalized_panel_heights(&self, min_panel_ratio: f32) -> Vec<f32> {
        let panel_count = self.panel_count();
        if panel_count == 0 {
            return Vec::new();
        }

        let mut heights = Vec::with_capacity(panel_count);
        let mut previous = 0.0;

        for split in self.normalized_splits(min_panel_ratio) {
            let clamped = split.clamp(previous, 1.0);
            heights.push((clamped - previous).max(0.0));
            previous = clamped;
        }

        heights.push((1.0 - previous).max(0.0));

        let total: f32 = heights.iter().sum();
        if total > f32::EPSILON {
            for height in &mut heights {
                *height /= total;
            }
        }

        heights
    }

    fn splits_from_heights(heights: &[f32]) -> Vec<f32> {
        if heights.len() <= 1 {
            return Vec::new();
        }

        let total: f32 = heights.iter().copied().sum();
        if total <= f32::EPSILON {
            let count = heights.len();
            return (1..count)
                .map(|index| index as f32 / count as f32)
                .collect();
        }

        let mut splits = Vec::with_capacity(heights.len().saturating_sub(1));
        let mut acc = 0.0;
        for height in heights.iter().take(heights.len().saturating_sub(1)) {
            acc += *height / total;
            splits.push(acc.clamp(0.0, 1.0));
        }

        splits
    }

    fn new_panel_id(&mut self) -> PanelId {
        let id = PanelId(self.next_panel_id);
        self.next_panel_id = self.next_panel_id.wrapping_add(1);
        id
    }

    fn new_layer_id(&mut self) -> LayerId {
        let id = LayerId(self.next_layer_id);
        self.next_layer_id = self.next_layer_id.wrapping_add(1);
        id
    }
}
