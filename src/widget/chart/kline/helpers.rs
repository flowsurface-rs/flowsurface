use super::composition::{
    self, BarMode, HistogramMode, LayerDataKind, MarkKind, PanelId, PanelScaleMode, PanelValueId,
    PanelValueLabelPolicy, PanelValuePrecision,
};
use super::{
    DEFAULT_MIN_PANEL_RATIO, DEFAULT_OVERLAY_CHANNELS, IndicatorData, KlinePanelKind,
    KlineSeriesLike, KlineWidget, OverlayChannelColorRole, OverlayChannelSource,
    OverlayChannelSpec, PanelYViewport,
};
use data::chart::kline_v2::IndicatorColorSlot;
use data::chart::kline_v2::SeriesColor;
use data::config::theme::{
    ThemeColor, ThemeColorRole, ThemeColorShade, darken, lighten, mix_color,
};

use iced::Point;
use iced::theme::palette::Extended;

impl<'a, S> KlineWidget<'a, S>
where
    S: KlineSeriesLike,
{
    pub(super) fn indicator_data_for_series(
        &self,
        series: &S,
        panel_value: Option<PanelValueId>,
        bar: &exchange::Kline,
    ) -> Option<IndicatorData> {
        series.indicator_data_for_panel_value_opt(panel_value, bar)
    }

    pub(super) fn panel_count(&self) -> usize {
        self.composition.panel_count().max(1)
    }

    pub(super) fn panel_index_for_id(&self, panel_id: PanelId) -> Option<usize> {
        self.composition
            .panels
            .iter()
            .position(|panel| panel.id == panel_id)
    }

    pub(super) fn panel_id(&self, panel_index: usize) -> Option<composition::PanelId> {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.id)
    }

    pub(super) fn panel_value_id(&self, panel_index: usize) -> Option<PanelValueId> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_id)
    }

    pub(super) fn panel_value_precision(&self, panel_index: usize) -> Option<PanelValuePrecision> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.value_precision)
    }

    pub(super) fn panel_value_label_policy(&self, panel_index: usize) -> PanelValueLabelPolicy {
        self.composition
            .panels
            .get(panel_index)
            .map(|panel| panel.value_label_policy)
            .unwrap_or_default()
    }

    pub(super) fn panel_y_viewport(&self, panel_id: PanelId) -> Option<PanelYViewport> {
        self.panel_y_viewports
            .iter()
            .find(|(id, _)| *id == panel_id)
            .map(|(_, viewport)| *viewport)
    }

    pub(super) fn panel_y_viewport_for_index(&self, panel_index: usize) -> Option<PanelYViewport> {
        self.panel_id(panel_index)
            .and_then(|panel_id| self.panel_y_viewport(panel_id))
    }

    pub(super) fn panel_uses_signed_overlay_input(&self, panel_index: usize) -> bool {
        let panel_value = self.panel_value_id(panel_index);

        let Some(base_series) = self.series.first() else {
            return false;
        };

        base_series.bars().iter().any(|bar| {
            self.indicator_data_for_series(base_series, panel_value, bar)
                .and_then(IndicatorData::signed_overlay)
                .is_some()
        })
    }

    pub(super) fn normalized_panel_splits(&self) -> Vec<f32> {
        if self.panel_count() <= 1 {
            Vec::new()
        } else {
            self.composition.normalized_splits(DEFAULT_MIN_PANEL_RATIO)
        }
    }

    pub(super) fn default_mark_for_panel(kind: KlinePanelKind) -> MarkKind {
        match kind {
            KlinePanelKind::PrimaryChart => MarkKind::Candle,
            KlinePanelKind::Indicator => MarkKind::Bar(BarMode::Histogram(HistogramMode::Plain)),
        }
    }

    pub(super) fn default_title_for_panel(kind: KlinePanelKind) -> Option<&'static str> {
        match kind {
            KlinePanelKind::PrimaryChart => None,
            KlinePanelKind::Indicator => Some("Indicator"),
        }
    }

    pub(super) fn resolved_panel_title(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> Option<&str> {
        self.composition
            .panels
            .get(panel_index)
            .and_then(|panel| panel.title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| Self::default_title_for_panel(panel_kind))
    }

    pub(super) fn resolved_panel_mark(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> MarkKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_mark_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_mark_with_runtime(
                panel_id,
                self.panel_uses_signed_overlay_input(panel_index),
            )
            .unwrap_or_else(|| Self::default_mark_for_panel(panel_kind))
    }

    pub(super) fn resolved_panel_scale_mode(&self, panel_index: usize) -> PanelScaleMode {
        let Some(panel) = self.composition.panels.get(panel_index) else {
            return PanelScaleMode::Absolute;
        };

        let mut scale = self
            .composition
            .panel_effective_scale_mode(panel.id)
            .unwrap_or(PanelScaleMode::Absolute);

        if matches!(panel.value_id, Some(PanelValueId::Volume))
            && matches!(scale, PanelScaleMode::Absolute)
        {
            scale = PanelScaleMode::FitVisibleIncludeZero;
        }

        scale
    }

    pub(super) fn default_data_kind_for_panel(kind: KlinePanelKind) -> LayerDataKind {
        match kind {
            KlinePanelKind::PrimaryChart => LayerDataKind::Ohlc,
            KlinePanelKind::Indicator => LayerDataKind::Scalar,
        }
    }

    pub(super) fn resolved_panel_data_kind(
        &self,
        panel_index: usize,
        panel_kind: KlinePanelKind,
    ) -> LayerDataKind {
        let Some(panel_id) = self.panel_id(panel_index) else {
            return Self::default_data_kind_for_panel(panel_kind);
        };

        self.composition
            .panel_effective_data_kind(panel_id)
            .unwrap_or_else(|| Self::default_data_kind_for_panel(panel_kind))
    }

    pub(super) fn series_line_color(&self, series: &S, palette: &Extended) -> iced::Color {
        match series.series_color() {
            SeriesColor::Base => palette.background.base.text,
            SeriesColor::Accent(accent) => {
                data::config::theme::generated_theme_accent(palette, accent)
            }
            SeriesColor::Custom(color) => color,
        }
    }

    pub(super) fn primary_overlay_entries(&self) -> Vec<(PanelValueId, bool)> {
        let Some(primary_panel_id) = self.composition.primary_panel_id() else {
            return Vec::new();
        };

        let Some(primary_panel) = self.composition.panel(primary_panel_id) else {
            return Vec::new();
        };

        primary_panel
            .layers
            .iter()
            .filter_map(|layer| {
                layer
                    .source
                    .indicator_value_id()
                    .map(|value_id| (value_id, layer.visible))
            })
            .collect()
    }

    pub(super) fn visible_primary_overlay_value_ids(&self) -> Vec<PanelValueId> {
        self.primary_overlay_entries()
            .into_iter()
            .filter_map(|(value_id, visible)| visible.then_some(value_id))
            .collect()
    }

    pub(super) fn overlay_channels_for_panel_value(
        &self,
        value_id: Option<PanelValueId>,
    ) -> &'static [OverlayChannelSpec] {
        if let Some(value_id) = value_id {
            let channels = value_id.overlay_channels();
            if !channels.is_empty() {
                return channels;
            }
        }

        &DEFAULT_OVERLAY_CHANNELS
    }

    pub(super) fn overlay_channel_value(
        &self,
        data: IndicatorData,
        channel: OverlayChannelSpec,
    ) -> Option<f32> {
        match channel.source {
            OverlayChannelSource::Primary => Some(data.value()),
            OverlayChannelSource::Field(key) => data.field(key),
            OverlayChannelSource::Reference(resolve) => Some(resolve(self.indicator_configs)),
        }
    }

    pub(super) fn overlay_channel_color(
        &self,
        value_id: Option<PanelValueId>,
        channel: OverlayChannelSpec,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) -> iced::Color {
        let configured = value_id.map(|indicator| {
            self.indicator_colors
                .config_for(indicator)
                .color_for(channel.color_slot)
        });
        let color = configured.unwrap_or_else(|| {
            ThemeColor::new(
                Self::theme_color_role(channel.color_role),
                ThemeColorShade::Base,
            )
        });
        Self::resolve_theme_color(color, palette, generated_accents).scale_alpha(
            match channel.color_role {
                OverlayChannelColorRole::Neutral => 0.72,
                _ => 0.78,
            },
        )
    }

    pub(super) fn indicator_color(
        &self,
        value_id: Option<PanelValueId>,
        slot: IndicatorColorSlot,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) -> iced::Color {
        let configured = value_id
            .map(|indicator| self.indicator_colors.config_for(indicator).color_for(slot))
            .unwrap_or_else(|| ThemeColor::new(ThemeColorRole::Neutral, ThemeColorShade::Base));
        Self::resolve_theme_color(configured, palette, generated_accents)
    }

    fn theme_color_role(role: OverlayChannelColorRole) -> ThemeColorRole {
        match role {
            OverlayChannelColorRole::Neutral => ThemeColorRole::Neutral,
            OverlayChannelColorRole::Success => ThemeColorRole::Success,
            OverlayChannelColorRole::Danger => ThemeColorRole::Danger,
            OverlayChannelColorRole::Primary => ThemeColorRole::Primary,
        }
    }

    pub(super) fn resolve_theme_color(
        color: ThemeColor,
        palette: &Extended,
        generated_accents: &[iced::Color; 6],
    ) -> iced::Color {
        let base = color.accent.map_or_else(
            || match color.role {
                ThemeColorRole::Neutral => palette.background.base.text,
                ThemeColorRole::Secondary => palette.secondary.strong.color,
                ThemeColorRole::Primary => palette.primary.base.color,
                ThemeColorRole::Success => palette.success.base.color,
                ThemeColorRole::Danger => palette.danger.base.color,
                ThemeColorRole::Warning => palette.warning.base.color,
            },
            |accent| generated_accents[accent.index()],
        );
        let background = palette.background.base.color;

        match color.shade {
            ThemeColorShade::Faint => mix_color(base, background, 0.3),
            ThemeColorShade::Soft => mix_color(base, background, 0.65),
            ThemeColorShade::Base => base,
            ThemeColorShade::Strong => {
                if palette.is_dark {
                    lighten(base, 0.1)
                } else {
                    darken(base, 0.1)
                }
            }
            ThemeColorShade::Vivid => {
                if palette.is_dark {
                    lighten(base, 0.18)
                } else {
                    darken(base, 0.18)
                }
            }
        }
    }

    pub(super) fn optimal_candlestick_width(bar_spacing: f32, pixel_ratio: f32) -> i32 {
        let from = 2.5_f32;
        let to = 4.0_f32;
        let coeff_special = 3.0_f32;

        if bar_spacing >= from && bar_spacing <= to {
            return (coeff_special * pixel_ratio).floor() as i32;
        }

        let reducing_coeff = 0.2_f32;
        let coeff = 1.0
            - (reducing_coeff * (bar_spacing.max(to) - to).atan()) / (std::f32::consts::PI * 0.5);

        let res = (bar_spacing * coeff * pixel_ratio).floor() as i32;
        let scaled_bar_spacing = (bar_spacing * pixel_ratio).floor() as i32;
        let optimal = res.min(scaled_bar_spacing);

        optimal.max(pixel_ratio.floor() as i32)
    }

    pub(super) fn candlestick_width(bar_spacing: f32, pixel_ratio: f32) -> i32 {
        let mut width = Self::optimal_candlestick_width(bar_spacing, pixel_ratio);
        if width >= 2 {
            let wick_width = pixel_ratio.floor() as i32;
            if (wick_width & 1) != (width & 1) {
                width -= 1;
            }
        }
        width
    }

    pub(super) fn resolved_pixel_ratio(&self) -> f32 {
        if self.pixel_ratio.is_finite() && self.pixel_ratio > 0.0 {
            self.pixel_ratio
        } else {
            1.0
        }
    }

    pub(super) fn physical_px_to_logical(px: i32, pixel_ratio: f32) -> f32 {
        px as f32 / pixel_ratio
    }

    pub(super) fn physical_px_to_logical_with_origin(
        px: i32,
        pixel_ratio: f32,
        origin_global: f32,
    ) -> f32 {
        Self::physical_px_to_logical(px, pixel_ratio) - origin_global
    }

    pub(super) fn snap_axis_to_physical_with_origin(
        value_local: f32,
        pixel_ratio: f32,
        origin_global: f32,
    ) -> i32 {
        ((value_local + origin_global) * pixel_ratio).round() as i32
    }

    pub(super) fn snap_plot_x_to_cell_with_origin(
        x_plot: f32,
        pixel_ratio: f32,
        origin_x_global: f32,
    ) -> f32 {
        let x_phys = Self::snap_axis_to_physical_with_origin(x_plot, pixel_ratio, origin_x_global);
        Self::physical_px_to_logical_with_origin(x_phys, pixel_ratio, origin_x_global)
    }

    pub(super) fn centered_left_for_width_with_origin(
        x_plot: f32,
        width_phys: i32,
        pixel_ratio: f32,
        origin_x_global: f32,
    ) -> f32 {
        let center_phys =
            Self::snap_axis_to_physical_with_origin(x_plot, pixel_ratio, origin_x_global);
        let left_phys = center_phys - (width_phys / 2);
        Self::physical_px_to_logical_with_origin(left_phys, pixel_ratio, origin_x_global)
    }

    pub(super) fn snapped_span_with_origin(
        start_local: f32,
        end_local: f32,
        pixel_ratio: f32,
        origin_y_global: f32,
    ) -> (f32, f32) {
        let top = start_local.min(end_local);
        let bottom = start_local.max(end_local);

        let top_phys = Self::snap_axis_to_physical_with_origin(top, pixel_ratio, origin_y_global);
        let mut bottom_phys =
            Self::snap_axis_to_physical_with_origin(bottom, pixel_ratio, origin_y_global);

        if bottom_phys <= top_phys {
            bottom_phys = top_phys + 1;
        }

        let top_snapped =
            Self::physical_px_to_logical_with_origin(top_phys, pixel_ratio, origin_y_global);
        let height_snapped = (bottom_phys - top_phys) as f32 / pixel_ratio;

        (top_snapped, height_snapped)
    }

    pub(super) fn quantized_stroke_width(width_logical: f32, pixel_ratio: f32) -> (f32, i32) {
        let stroke_width_phys = (width_logical.max(0.0) * pixel_ratio).round() as i32;
        let stroke_width_phys = stroke_width_phys.max(1);
        (
            Self::logical_width_from_physical(stroke_width_phys, pixel_ratio),
            stroke_width_phys,
        )
    }

    pub(super) fn snap_stroke_center_with_origin(
        value_local: f32,
        pixel_ratio: f32,
        origin_global: f32,
        stroke_width_phys: i32,
    ) -> f32 {
        let axis_phys = (value_local + origin_global) * pixel_ratio;
        let snapped_phys = if (stroke_width_phys & 1) == 0 {
            axis_phys.round()
        } else {
            (axis_phys - 0.5).round() + 0.5
        };

        (snapped_phys / pixel_ratio) - origin_global
    }

    pub(super) fn snap_plot_x_for_stroke_with_origin(
        x_plot: f32,
        pixel_ratio: f32,
        origin_x_global: f32,
        stroke_width_phys: i32,
    ) -> f32 {
        let x_cell = Self::snap_plot_x_to_cell_with_origin(x_plot, pixel_ratio, origin_x_global);

        Self::snap_stroke_center_with_origin(
            x_cell,
            pixel_ratio,
            origin_x_global,
            stroke_width_phys,
        )
    }

    pub(super) fn snap_point_for_stroke_with_origin(
        point: Point,
        pixel_ratio: f32,
        origin_global: Point,
        stroke_width_phys: i32,
    ) -> Point {
        Point::new(
            Self::snap_stroke_center_with_origin(
                point.x,
                pixel_ratio,
                origin_global.x,
                stroke_width_phys,
            ),
            Self::snap_stroke_center_with_origin(
                point.y,
                pixel_ratio,
                origin_global.y,
                stroke_width_phys,
            ),
        )
    }

    pub(super) fn snap_plot_x_to_cell(x_plot: f32, pixel_ratio: f32) -> f32 {
        let x_phys = (x_plot * pixel_ratio).round() as i32;
        Self::physical_px_to_logical(x_phys, pixel_ratio)
    }

    pub(super) fn logical_width_from_physical(width_phys: i32, pixel_ratio: f32) -> f32 {
        Self::physical_px_to_logical(width_phys.max(1), pixel_ratio)
    }
}
