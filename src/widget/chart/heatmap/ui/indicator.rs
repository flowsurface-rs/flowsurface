use super::AXIS_TEXT_SIZE;
use crate::widget::chart;
use crate::{
    style,
    widget::chart::{
        Y_AXIS_LABEL_LEFT_INSET,
        heatmap::{Message, view::TimeView},
    },
};
use data::{
    chart::heatmap::indicators::{HeatmapIndicator, HeatmapIndicators, SpreadUnit},
    chart::kline::viewport::PanelYValueRange,
    chart::ticks::{
        Y_LABEL_DENSITY,
        y::{TickValue, YAxisScale},
        y_labels_that_fit,
    },
    util::abbr_large_numbers,
};
use exchange::{UnixMs, unit::MinTicksize};
use iced::{Point, Rectangle, Renderer, Size, Theme, mouse, widget::canvas};

pub struct IndicatorCanvas<'a> {
    pub indicators: &'a HeatmapIndicators,
    pub kind: HeatmapIndicator,
    pub time: TimeView<'a>,
    pub plot_width: f32,
    pub cursor_time: Option<UnixMs>,
    pub range: Option<PanelYValueRange>,
    pub min_ticksize: MinTicksize,
}

impl IndicatorCanvas<'_> {
    fn format_value(&self, value: f64) -> String {
        match self.kind {
            HeatmapIndicator::Spread => {
                let formatted = format!("{value:.3}");
                match self.indicators.config().spread_unit {
                    SpreadUnit::Bps => formatted,
                    SpreadUnit::Ticks => formatted
                        .trim_end_matches('0')
                        .trim_end_matches('.')
                        .to_owned(),
                }
            }
            HeatmapIndicator::Depth => abbr_large_numbers(value),
        }
    }
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
        let channels: Vec<_> = self
            .indicators
            .channels(self.kind, self.min_ticksize)
            .collect();
        let latest_time = UnixMs::new(
            (self.time.latest_bucket.max(0) as u64).saturating_mul(self.time.interval_ms),
        );
        let top = 0.0;
        let bottom = bounds.height;
        let title = match self.kind {
            HeatmapIndicator::Spread => {
                let unit = match self.indicators.config().spread_unit {
                    SpreadUnit::Bps => "bps",
                    SpreadUnit::Ticks => "ticks",
                };
                format!("Spread ({unit})")
            }
            HeatmapIndicator::Depth => format!(
                "Depth ({:.2}%)",
                self.indicators.config().distance.percent()
            ),
        };

        let y = chart::LEGEND_TOP_PAD + chart::LEGEND_ROW_HEIGHT * 0.5;
        let mut end_x =
            chart::LEGEND_LEFT_PAD + title.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
        let mut text = vec![(title, chart::LEGEND_LEFT_PAD)];
        let mut x = end_x + chart::LEGEND_TITLE_GAP;

        for channel in &channels {
            if x >= self.plot_width - chart::LEGEND_PADDING {
                break;
            }

            if self.kind == HeatmapIndicator::Depth {
                let label = channel.label().to_owned();
                end_x = x + label.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
                text.push((label, x));
                x = end_x + chart::LEGEND_LABEL_VALUE_GAP;
            }

            if x >= self.plot_width - chart::LEGEND_PADDING {
                break;
            }

            let readout_time = self.cursor_time.unwrap_or(latest_time);
            let value = (readout_time <= latest_time)
                .then(|| channel.value_at(readout_time))
                .flatten();
            let value = value
                .map(|value| self.format_value(value))
                .unwrap_or_else(|| "n/a".to_owned());
            end_x = x + value.chars().count() as f32 * chart::LEGEND_CHAR_WIDTH;
            text.push((value, x));
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

                for (content, x) in text {
                    frame.fill_text(canvas::Text {
                        content,
                        position: Point::new(x, y),
                        color: palette.background.base.text,
                        size: chart::LEGEND_TEXT_SIZE.into(),
                        font: style::AZERET_MONO,
                        align_x: iced::Alignment::Start.into(),
                        align_y: iced::Alignment::Center.into(),
                        ..Default::default()
                    });
                }
            },
        );
        let cursor_y = cursor
            .position_in(bounds)
            .filter(|position| position.x < self.plot_width)
            .map(|position| (position.y.round() + 0.5).clamp(top, bottom));
        if let Some(y) = cursor_y {
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
            let min = range.min_value();
            let max = range.max_value();
            let badge_height = AXIS_TEXT_SIZE + 6.0;
            let badge_top = cursor_y.map(|y| {
                (y - badge_height * 0.5).clamp(0.0, (bounds.height - badge_height).max(0.0))
            });
            let ticks = YAxisScale::Linear.ticks(
                min,
                max,
                y_labels_that_fit(bottom - top, AXIS_TEXT_SIZE, Y_LABEL_DENSITY).max(2) as i32,
            );
            let label_inset = (AXIS_TEXT_SIZE * 0.6).min(bounds.height * 0.5);
            for tick in ticks.ticks {
                let TickValue::Float(value) = tick.value else {
                    continue;
                };
                let y = (bottom - tick.ratio as f32 * (bottom - top))
                    .clamp(label_inset, bounds.height - label_inset);
                if badge_top.is_some_and(|badge_top| {
                    y + AXIS_TEXT_SIZE * 0.5 >= badge_top
                        && y - AXIS_TEXT_SIZE * 0.5 <= badge_top + badge_height
                }) {
                    continue;
                }

                frame.fill_text(canvas::Text {
                    content: self.format_value(value),
                    position: Point::new(self.plot_width + Y_AXIS_LABEL_LEFT_INSET, y),
                    color: palette.background.base.text,
                    size: AXIS_TEXT_SIZE.into(),
                    font: style::AZERET_MONO,
                    align_x: iced::Alignment::Start.into(),
                    align_y: iced::Alignment::Center.into(),
                    ..Default::default()
                });
            }
            if let Some((y, badge_top)) = cursor_y.zip(badge_top) {
                let value = min + range.span() * f64::from((bottom - y) / (bottom - top));
                frame.fill_rectangle(
                    Point::new(self.plot_width, badge_top),
                    Size::new((bounds.width - self.plot_width).max(0.0), badge_height),
                    palette.background.strong.color,
                );

                frame.fill_text(canvas::Text {
                    content: self.format_value(value),
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
