use super::super::interaction::LayerContext;
use super::super::painting::PaintInputs;
use super::LayerPaintPhase;
use crate::widget::chart::kline::footprint::{self, PixelRows};
use crate::widget::chart::kline::painting::PixelGeometry;
use crate::widget::chart::kline::scene::Scene;
use crate::widget::resolve_theme_color;
use data::chart::Basis;
use data::chart::kline::footprint::ClusterKind;
use data::chart::kline::poc::{NPoc, PocFillAt};
use data::chart::kline::{
    KlineStudy, ProfileDistribution, StudyDataSource, VolumeProfileColorConfig,
    VolumeProfileColorSlot, VolumeProfileSchedule,
};
use data::config::theme::ACCENT_COUNT;
use exchange::unit::{PriceStep, Qty};
use exchange::{Kline, TickerInfo, UnixMs};
use iced::theme::palette::Extended;
use iced::{Point, Size};
use std::collections::BTreeMap;
use std::{cell::RefCell, sync::Arc};

pub(crate) enum ProfileBarSource<'a> {
    Empty,
    External {
        bars: &'a BTreeMap<UnixMs, Kline>,
        revision: u64,
    },
    Chart {
        retained: (UnixMs, UnixMs),
        revision: u64,
    },
}

impl ProfileBarSource<'_> {
    fn collect(&self, series: &[data::chart::kline::series::KlineSeries]) -> Vec<Kline> {
        match self {
            Self::Empty => Vec::new(),
            Self::External { bars, .. } => bars.values().copied().collect(),
            Self::Chart { retained, .. } => series
                .first()
                .into_iter()
                .flat_map(data::chart::kline::series::KlineSeries::points)
                .filter(|point| point.kline.time >= retained.0 && point.kline.time < retained.1)
                .map(|point| *point.kline)
                .collect(),
        }
    }

    fn collect_visible(
        &self,
        series: &data::chart::kline::series::KlineSeries,
        earliest: UnixMs,
        latest: UnixMs,
    ) -> Vec<Kline> {
        if earliest >= latest {
            return Vec::new();
        }
        match self {
            Self::Empty => Vec::new(),
            Self::External { bars, .. } => {
                bars.range(earliest..latest).map(|(_, bar)| *bar).collect()
            }
            Self::Chart { retained, .. } => {
                let start = earliest.max(retained.0);
                let end = latest.min(retained.1);
                if start >= end {
                    return Vec::new();
                }
                let last = end.saturating_sub(1);
                let indices = series.time_index_range(start, last);
                let mut bars = Vec::with_capacity(indices.len());
                series.for_each_point_in_range(indices, Some((start, last)), |point| {
                    bars.push(*point.kline);
                });
                bars
            }
        }
    }

    fn key(&self) -> ProfileSourceKey {
        match self {
            Self::Empty => ProfileSourceKey::Empty,
            Self::External { revision, .. } => ProfileSourceKey::External(*revision),
            Self::Chart { retained, revision } => ProfileSourceKey::Chart {
                retained: *retained,
                revision: *revision,
            },
        }
    }
}

#[derive(PartialEq, Eq)]
enum ProfileSourceKey {
    Empty,
    External(u64),
    Chart {
        retained: (UnixMs, UnixMs),
        revision: u64,
    },
}

#[derive(PartialEq)]
struct VisibleProfileKey {
    source: ProfileSourceKey,
    ticker: TickerInfo,
    basis: Basis,
    earliest: UnixMs,
    latest: UnixMs,
    tick_size: PriceStep,
}

struct VisibleProfile {
    key: VisibleProfileKey,
    distribution: Option<Arc<ProfileDistribution>>,
}

#[derive(Default)]
pub(crate) struct VolumeProfileState {
    visible: RefCell<Option<VisibleProfile>>,
}

impl VolumeProfileState {
    fn resolve(
        &self,
        key: VisibleProfileKey,
        calculate: impl FnOnce() -> Option<ProfileDistribution>,
    ) -> Option<Arc<ProfileDistribution>> {
        let mut cached = self.visible.borrow_mut();
        if cached.as_ref().is_none_or(|profile| profile.key != key) {
            *cached = Some(VisibleProfile {
                key,
                distribution: calculate().map(Arc::new),
            });
        }
        cached.as_ref()?.distribution.clone()
    }
}

pub(crate) struct VolumeProfileObject<'a> {
    studies: &'a [KlineStudy],
    profile_bars: ProfileBarSource<'a>,
}

impl<'a> VolumeProfileObject<'a> {
    pub(crate) fn new(studies: &'a [KlineStudy], profile_bars: ProfileBarSource<'a>) -> Self {
        Self {
            studies,
            profile_bars,
        }
    }

    pub(super) fn paint(
        &self,
        phase: LayerPaintPhase,
        context: LayerContext<'_>,
        state: &VolumeProfileState,
        frame: &mut PaintInputs,
        palette: &Extended,
        accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        if phase == LayerPaintPhase::BeforeIndicators {
            self.fill_volume_profile_overlay(frame, context, state, palette, accents, pixels);
        }
    }

    fn draw_profile_distribution(
        &self,
        frame: &mut PaintInputs,
        scene: &Scene,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        distribution: &ProfileDistribution,
        colors: VolumeProfileColorConfig,
        profile_edge: f32,
        profile_width: f32,
        bar_alpha: f32,
        opacity_scale: f32,
        line_start: f32,
        naked_line_end: f32,
        grow_right: bool,
        pixels: PixelGeometry,
    ) {
        let tick_size = distribution.price_step;
        let max_qty = distribution.max_qty.to_f64();
        if max_qty <= 0.0 {
            return;
        }

        let anchor = scene
            .primary
            .series_percent_anchors
            .first()
            .copied()
            .flatten();
        let min_row_height = pixels.logical_width_from_physical(1);
        let sell_color = resolve_theme_color(
            colors.color_for(VolumeProfileColorSlot::Sell),
            palette,
            generated_accents,
        )
        .scale_alpha(bar_alpha);
        let buy_color = resolve_theme_color(
            colors.color_for(VolumeProfileColorSlot::Buy),
            palette,
            generated_accents,
        )
        .scale_alpha(bar_alpha);

        let plot = *scene.primary_plot();
        let body_left = if grow_right {
            profile_edge
        } else {
            profile_edge - profile_width
        };
        if profile_width > 0.0
            && body_left < plot.x + plot.width
            && body_left + profile_width > plot.x
        {
            let profile_width = profile_width.max(min_row_height);
            let rows =
                footprint::coalesce_price_rows(scene, tick_size, anchor, pixels).then(|| {
                    let mut rows = PixelRows::new(*scene.primary_plot(), pixels);
                    for level in &distribution.levels {
                        rows.add(
                            scene.map_primary_plot_with_anchor_f32(
                                level.price.to_f32_lossy(),
                                anchor,
                            ),
                            level.positive_qty,
                            level.negative_qty,
                        );
                    }
                    rows
                });
            let max_qty = rows.as_ref().map_or(max_qty, |rows| {
                let coalesced_max = rows.max_qty(ClusterKind::VolumeProfile).to_f64();
                if coalesced_max > 0.0 {
                    coalesced_max
                } else {
                    max_qty
                }
            });
            let mut draw_row = |y: f32, row_height: f32, positive: Qty, negative: Qty| {
                let row_height = row_height.max(min_row_height);
                let row_top = y - row_height * 0.5;
                let row_bottom = row_top + row_height;
                if row_bottom < scene.primary_plot().y
                    || row_top > scene.primary_plot().y + scene.primary_plot().height
                {
                    return;
                }
                let total_qty = (positive + negative).to_f64();
                if total_qty <= 0.0 {
                    return;
                }

                let total_width = profile_width * (total_qty / max_qty).clamp(0.0, 1.0) as f32;
                if total_width <= 0.0 {
                    return;
                }

                let row_span = pixels.snapped_y_span(
                    row_top,
                    row_top + (row_height - min_row_height).max(min_row_height),
                );

                let left = if grow_right {
                    profile_edge
                } else {
                    profile_edge - total_width
                };
                let left_span = pixels.snapped_x_span(left, left + total_width);
                let negative_width =
                    left_span.size * (negative.to_f64() / total_qty).clamp(0.0, 1.0) as f32;
                let positive_width = left_span.size - negative_width;

                if negative_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left_span.start, row_span.start),
                        Size::new(negative_width, row_span.size),
                        sell_color,
                    );
                }
                if positive_width > 0.0 {
                    frame.fill_rectangle(
                        Point::new(left_span.start + negative_width, row_span.start),
                        Size::new(positive_width, row_span.size),
                        buy_color,
                    );
                }
            };
            if let Some(rows) = rows {
                for (y, positive, negative) in rows.rows() {
                    draw_row(y, rows.bucket_height, positive, negative);
                }
            } else {
                for level in &distribution.levels {
                    let y =
                        scene.map_primary_plot_with_anchor_f32(level.price.to_f32_lossy(), anchor);
                    draw_row(
                        y,
                        footprint::row_height(scene, level.price, tick_size, anchor),
                        level.positive_qty,
                        level.negative_qty,
                    );
                }
            }
        }

        let Some(poc) = distribution.poc else {
            return;
        };
        let (line_end, line_color) = match poc.status {
            NPoc::Naked if colors.show_npoc => (
                naked_line_end,
                resolve_theme_color(
                    colors.color_for(VolumeProfileColorSlot::NakedPoc),
                    palette,
                    generated_accents,
                )
                .scale_alpha((0.7 * opacity_scale).min(1.0)),
            ),
            NPoc::Filled {
                at: PocFillAt::Time(at),
            } if colors.show_poc => (
                scene
                    .x
                    .axis
                    .unit_from_time_f32(at)
                    .map(|unit| scene.map_x_plot_f32(unit))
                    .unwrap_or(naked_line_end),
                resolve_theme_color(
                    colors.color_for(VolumeProfileColorSlot::FilledPoc),
                    palette,
                    generated_accents,
                )
                .scale_alpha((0.85 * opacity_scale).min(1.0)),
            ),
            _ => return,
        };
        if !line_start.is_finite() || !line_end.is_finite() {
            return;
        }

        let poc_y = scene.map_primary_plot_with_anchor_f32(poc.price.to_f32_lossy(), anchor);
        if !poc_y.is_finite() {
            return;
        }
        let Some(line) = pixels.positive_x_span(line_start, line_end) else {
            return;
        };
        let raw_line_height = footprint::row_height(scene, poc.price, tick_size, anchor)
            .abs()
            .max(min_row_height)
            .min(1.0_f32.max(min_row_height));
        let line_span =
            pixels.snapped_y_span(poc_y - raw_line_height * 0.5, poc_y + raw_line_height * 0.5);

        frame.fill_rectangle(
            Point::new(line.start, line_span.start),
            Size::new(line.size, line_span.size),
            line_color,
        );
    }

    fn fill_volume_profile_overlay(
        &self,
        frame: &mut PaintInputs,
        context: LayerContext<'_>,
        state: &VolumeProfileState,
        palette: &Extended,
        generated_accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        let scene = context.scene;
        let Some(base_series) = context.coordinates.series.first() else {
            return;
        };

        let Some(earliest) = scene.x.axis.time_from_unit(scene.x.min_unit) else {
            return;
        };
        let Some(latest) = scene.x.axis.time_from_unit(scene.x.max_unit) else {
            return;
        };

        let tick_size = base_series.tick_size();
        let visible_profile = self.studies.iter().find_map(|study| {
            if !study.visible() {
                return None;
            }

            if study.volume_profile_schedule()? != VolumeProfileSchedule::VisibleRange {
                return None;
            }

            let colors = study.volume_profile_colors()?;
            let width_ratio = study.volume_profile_width_ratio()?;
            let opacity_ratio = study.volume_profile_opacity_ratio()?;
            let opacity_scale = study.volume_profile_opacity_scale()?;
            let profile = match study.source() {
                StudyDataSource::Kline => state.resolve(
                    VisibleProfileKey {
                        source: self.profile_bars.key(),
                        ticker: base_series.ticker_info,
                        basis: context.coordinates.basis,
                        earliest,
                        latest,
                        tick_size,
                    },
                    || {
                        let bars = self
                            .profile_bars
                            .collect_visible(base_series, earliest, latest);
                        data::chart::kline::kline_volume_profile(&bars, earliest, latest, tick_size)
                    },
                ),
                StudyDataSource::Trades => base_series
                    .trade_price_distribution(earliest, latest, tick_size)
                    .map(Arc::new),
            };
            profile.map(|profile| (profile, colors, width_ratio, opacity_ratio, opacity_scale))
        });
        let recurring_intervals = self
            .studies
            .iter()
            .filter(|study| study.visible())
            .filter_map(|study| {
                let VolumeProfileSchedule::Recurring { interval } =
                    study.volume_profile_schedule()?
                else {
                    return None;
                };
                if !study.is_interval_available(context.coordinates.basis) {
                    return None;
                }
                Some((
                    study.source(),
                    interval,
                    study.volume_profile_colors()?,
                    study.volume_profile_width_ratio()?,
                    study.volume_profile_opacity_ratio()?,
                    study.volume_profile_opacity_scale()?,
                    tick_size,
                ))
            });
        let mut recurring_bars = None;
        let profiles = recurring_intervals
            .flat_map(
                |(
                    source,
                    interval,
                    colors,
                    width_ratio,
                    opacity_ratio,
                    opacity_scale,
                    tick_size,
                )| {
                    let profiles = match source {
                        StudyDataSource::Kline => {
                            let profile_bars = recurring_bars.get_or_insert_with(|| {
                                self.profile_bars.collect(context.coordinates.series)
                            });
                            data::chart::kline::recurring_kline_volume_profiles(
                                profile_bars,
                                interval,
                                earliest,
                                latest,
                                tick_size,
                            )
                        }
                        StudyDataSource::Trades => base_series.recurring_trade_price_distributions(
                            interval, tick_size, earliest, latest,
                        ),
                    };
                    profiles.into_iter().map(move |profile| {
                        (profile, colors, width_ratio, opacity_ratio, opacity_scale)
                    })
                },
            )
            .collect::<Vec<_>>();
        if visible_profile.is_none() && profiles.is_empty() {
            return;
        }

        let primary_plot = *scene.primary_plot();
        let min_profile_width = pixels.logical_width_from_physical(1);
        frame.with_clip(primary_plot, |frame| {
            if let Some((profile, colors, width_ratio, opacity_ratio, opacity_scale)) =
                visible_profile.as_ref()
            {
                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    profile,
                    *colors,
                    primary_plot.x + primary_plot.width,
                    primary_plot.width * width_ratio,
                    *opacity_ratio,
                    *opacity_scale,
                    primary_plot.x,
                    primary_plot.x + primary_plot.width,
                    false,
                    pixels,
                );
            }

            for (anchored_profile, colors, width_ratio, opacity_ratio, opacity_scale) in profiles {
                let Some(start_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.start)
                else {
                    continue;
                };
                let Some(end_unit) = scene.x.axis.unit_from_time_f32(anchored_profile.end) else {
                    continue;
                };

                let segment_left = scene.map_x_plot_f32(start_unit);
                let segment_right = scene.map_x_plot_f32(end_unit);
                if !width_ratio.is_finite() || width_ratio <= 0.0 {
                    continue;
                }
                let segment_width = (segment_right - segment_left).abs();
                if !segment_width.is_finite() || segment_width <= 0.0 {
                    continue;
                }
                let profile_width = (segment_width * width_ratio).max(min_profile_width);

                self.draw_profile_distribution(
                    frame,
                    scene,
                    palette,
                    generated_accents,
                    &anchored_profile.distribution,
                    colors,
                    segment_left,
                    profile_width,
                    opacity_ratio,
                    opacity_scale,
                    segment_left,
                    primary_plot.x + primary_plot.width,
                    true,
                    pixels,
                );
            }
        });
    }
}
