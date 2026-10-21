use super::AXIS_TEXT_SIZE;
use crate::widget::chart;
use crate::widget::chart::kline::place_non_overlapping_labels;
use crate::{
    style,
    widget::chart::{
        Y_AXIS_LABEL_LEFT_INSET,
        heatmap::{Message, view::TimeView},
    },
};
use data::{
    chart::heatmap::indicators::{
        DepthCoverage, HeatmapIndicators, HeatmapPanel, IndicatorValues, SpreadUnit,
    },
    chart::kline::viewport::PanelYValueRange,
};
use exchange::{TickerInfo, UnixMs};
use iced::{Point, Rectangle, Renderer, Size, Theme, mouse, widget::canvas};

pub struct IndicatorCanvas<'a> {
    pub indicators: &'a HeatmapIndicators,
    pub kind: HeatmapPanel,
    pub time: TimeView<'a>,
    pub plot_width: f32,
    pub cursor_time: Option<UnixMs>,
    pub range: Option<PanelYValueRange>,
    pub ticker_info: TickerInfo,
}

impl canvas::Program<Message> for IndicatorCanvas<'_> {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Vec::new();
        }

        let palette = theme.extended_palette();
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let latest_time = UnixMs::new(
            (self.time.latest_bucket.max(0) as u64).saturating_mul(self.time.interval_ms),
        );
        let readout_time = self.cursor_time.unwrap_or(latest_time);
        let values =
            self.indicators
                .value_at(self.kind, self.ticker_info.min_ticksize, readout_time);

        let readouts = match (values, self.kind) {
            (Some(IndicatorValues::Spread(value)), _) => vec![("Spread", Some(value))],
            (Some(IndicatorValues::Depth { bid, ask }), _) => {
                vec![("Bid", Some(bid)), ("Ask", Some(ask))]
            }
            (None, HeatmapPanel::Spread) => vec![("Spread", None)],
            (None, HeatmapPanel::Depth) => vec![("Bid", None), ("Ask", None)],
        };
        let colors = chart::heatmap::panel_colors(self.kind, theme);

        let scale = self
            .kind
            .value_scale(self.indicators.config(), self.ticker_info);
        let label_policy = self.kind.value_label_policy();
        let top = 0.0;
        let bottom = bounds.height;

        let title = match self.kind {
            HeatmapPanel::Spread => {
                let unit = match self.indicators.config().spread_unit {
                    SpreadUnit::Bps => "bps",
                    SpreadUnit::Ticks => "ticks",
                };
                format!("Spread ({unit})")
            }
            HeatmapPanel::Depth => format!("Depth ({}%)", self.indicators.config().distance),
        };

        let y = chart::LEGEND_TOP_PAD + chart::LEGEND_ROW_HEIGHT * 0.5;
        let text_color = palette.background.base.text;
        let mut text = Vec::new();
        let mut title_x = chart::LEGEND_LEFT_PAD;

        if self.kind == HeatmapPanel::Depth
            && values.is_some()
            && self.indicators.depth_coverage_at(readout_time) == Some(DepthCoverage::FallsShort)
        {
            text.push(("!".to_owned(), title_x, palette.danger.base.color));
            title_x += chart::LEGEND_CHAR_WIDTH + chart::LEGEND_PADDING;
        }

        let mut end_x = title_x + title.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
        text.push((title, title_x, text_color));
        let mut x = end_x + chart::LEGEND_TITLE_GAP;

        for ((label, value), color) in readouts.into_iter().zip(colors) {
            if x >= self.plot_width - chart::LEGEND_PADDING {
                break;
            }

            if self.kind == HeatmapPanel::Depth {
                let label = label.to_owned();
                end_x = x + label.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
                text.push((label, x, color));
                x = end_x + chart::LEGEND_LABEL_VALUE_GAP;
            }

            if x >= self.plot_width - chart::LEGEND_PADDING {
                break;
            }

            let value = value
                .map(|value| {
                    label_policy.format_header_value(
                        || scale.quantize_value(value),
                        || scale.format_compact(value, 0.01, label_policy.max_decimals),
                    )
                })
                .unwrap_or_else(|| "n/a".to_owned());

            end_x = x + value.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
            text.push((value, x, color));
            x = end_x + chart::LEGEND_READOUT_GAP;
        }

        frame.with_clip(
            Rectangle::new(Point::ORIGIN, Size::new(self.plot_width, bounds.height)),
            |frame| {
                let background = chart::legend_row_background(
                    Rectangle {
                        x: 0.0,
                        y: chart::LEGEND_TOP_PAD,
                        width: self.plot_width,
                        height: chart::LEGEND_ROW_HEIGHT,
                    },
                    end_x,
                );

                frame.fill_rectangle(
                    background.position(),
                    background.size(),
                    chart::legend_background_color(palette),
                );

                for (content, x, color) in text {
                    frame.fill_text(canvas::Text {
                        content,
                        position: Point::new(x, y),
                        color,
                        size: chart::LEGEND_TEXT_SIZE.into(),
                        font: style::AZERET_MONO,
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });
                }
            },
        );

        let cursor_position = cursor
            .position_in(bounds)
            .filter(|position| position.x < self.plot_width);
        let cursor_value = cursor_position.zip(self.range).map(|(position, range)| {
            let ratio = f64::from((bottom - position.y) / (bottom - top));
            scale.quantize_value(range.min_value() + range.span() * ratio)
        });
        let cursor_y = cursor_value
            .zip(self.range)
            .map(|(value, range)| {
                (bottom - ((value - range.min_value()) / range.span()) as f32 * (bottom - top))
                    .clamp(top, bottom)
            })
            .or_else(|| cursor_position.map(|position| position.y));

        if let Some(y) = cursor_y {
            let y = (y.round() + 0.5).clamp(top, bottom);
            frame.with_clip(
                Rectangle::with_size(Size::new(self.plot_width, bounds.height)),
                |frame| {
                    frame.stroke(
                        &canvas::Path::line(Point::new(0.0, y), Point::new(self.plot_width, y)),
                        style::chart_crosshair(theme),
                    );
                },
            );
        }

        if let Some(range) = self.range {
            let badge_height = AXIS_TEXT_SIZE + 6.0;
            let badge_top = cursor_y.map(|y| {
                (y - badge_height * 0.5).clamp(0.0, (bounds.height - badge_height).max(0.0))
            });

            let labels = range.axis_labels(scale, label_policy, top, bounds.height, AXIS_TEXT_SIZE);
            let centers: Vec<_> = labels.iter().map(|(_, y)| *y).collect();
            let placements =
                place_non_overlapping_labels(&centers, top, bottom, AXIS_TEXT_SIZE, 2.0);
            let label_inset = (AXIS_TEXT_SIZE * 0.6).min(bounds.height * 0.5);

            for ((content, _), placement) in labels.into_iter().zip(placements) {
                let Some(y) = placement else { continue };
                let y = y.clamp(label_inset, bounds.height - label_inset);

                if badge_top.is_some_and(|badge_top| {
                    y + AXIS_TEXT_SIZE * 0.5 >= badge_top
                        && y - AXIS_TEXT_SIZE * 0.5 <= badge_top + badge_height
                }) {
                    continue;
                }

                frame.fill_text(canvas::Text {
                    content,
                    position: Point::new(self.plot_width + Y_AXIS_LABEL_LEFT_INSET, y),
                    color: palette.background.base.text,
                    size: AXIS_TEXT_SIZE.into(),
                    font: style::AZERET_MONO,
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }

            if let Some((value, badge_top)) = cursor_value.zip(badge_top) {
                frame.fill_rectangle(
                    Point::new(self.plot_width, badge_top),
                    Size::new((bounds.width - self.plot_width).max(0.0), badge_height),
                    palette.background.strong.color,
                );

                frame.fill_text(canvas::Text {
                    content: label_policy.format_axis_value(
                        || scale.quantize_value(value),
                        || scale.format_compact(value, 0.01, label_policy.max_decimals),
                    ),
                    position: Point::new(
                        self.plot_width + Y_AXIS_LABEL_LEFT_INSET,
                        badge_top + badge_height * 0.5,
                    ),
                    color: palette.background.strong.text,
                    size: AXIS_TEXT_SIZE.into(),
                    font: style::AZERET_MONO,
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }
        }

        vec![frame.into_geometry()]
    }
}
