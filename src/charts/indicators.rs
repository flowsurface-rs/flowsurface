pub mod open_interest;
pub mod volume;

use std::{
    any::Any,
    fmt::{self, Debug, Display},
};

use iced::{
    Event, Point, Rectangle, Renderer, Size, Theme, mouse,
    theme::palette::Extended,
    widget::canvas::{self, Cache, Frame, Geometry},
};
use serde::{Deserialize, Serialize};

use super::{abbr_large_numbers, round_to_tick, scales::linear};
use crate::{
    charts::scales::{AxisLabel, Label, calc_label_rect},
    data_providers::MarketType,
};

use super::{Interaction, Message};

pub trait Indicator: PartialEq + Display + Debug + 'static {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self]
    where
        Self: Sized;

    fn get_enabled(
        indicators: &[Self],
        market_type: Option<MarketType>,
    ) -> impl Iterator<Item = &Self>
    where
        Self: Sized,
    {
        Self::get_available(market_type)
            .iter()
            .filter(move |indicator| indicators.contains(indicator))
    }
    fn as_any(&self) -> &dyn Any;
}

/// Candlestick chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum CandlestickIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for CandlestickIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CandlestickIndicator {
    const ALL: [CandlestickIndicator; 2] = [
        CandlestickIndicator::Volume,
        CandlestickIndicator::OpenInterest,
    ];
    const SPOT: [CandlestickIndicator; 1] = [CandlestickIndicator::Volume];
    const PERPS: [CandlestickIndicator; 2] = [
        CandlestickIndicator::Volume,
        CandlestickIndicator::OpenInterest,
    ];
}

impl Display for CandlestickIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CandlestickIndicator::Volume => write!(f, "Volume"),
            CandlestickIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}

/// Heatmap chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum HeatmapIndicator {
    Volume,
}

impl Indicator for HeatmapIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl HeatmapIndicator {
    const ALL: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
    const SPOT: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
    const PERPS: [HeatmapIndicator; 1] = [HeatmapIndicator::Volume];
}

impl Display for HeatmapIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            HeatmapIndicator::Volume => write!(f, "Volume"),
        }
    }
}

/// Footprint chart indicators
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize, Eq, Hash)]
pub enum FootprintIndicator {
    Volume,
    OpenInterest,
}

impl Indicator for FootprintIndicator {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] {
        match market_type {
            Some(MarketType::Spot) => &Self::SPOT,
            Some(MarketType::LinearPerps) => &Self::PERPS,
            _ => &Self::ALL,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl FootprintIndicator {
    const ALL: [FootprintIndicator; 2] =
        [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
    const SPOT: [FootprintIndicator; 1] = [FootprintIndicator::Volume];
    const PERPS: [FootprintIndicator; 2] =
        [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
}

impl Display for FootprintIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FootprintIndicator::Volume => write!(f, "Volume"),
            FootprintIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}

fn draw_borders(frame: &mut Frame, bounds: Rectangle, palette: &Extended) {
    frame.fill_rectangle(
        Point::new(0.0, 0.0),
        Size::new(1.0, bounds.height),
        if palette.is_dark {
            palette.background.weak.color.scale_alpha(0.4)
        } else {
            palette.background.strong.color.scale_alpha(0.4)
        },
    );
}

pub struct IndicatorLabel<'a> {
    pub label_cache: &'a Cache,
    pub crosshair: bool,
    pub max: f32,
    pub min: f32,
    pub chart_bounds: Rectangle,
}

impl canvas::Program<Message> for IndicatorLabel<'_> {
    type State = Interaction;

    fn update(
        &self,
        _state: &mut Self::State,
        _event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = theme.extended_palette();

        let (highest, lowest) = (self.max, self.min);
        let range = highest - lowest;

        let text_size = 12.0;

        let labels = self.label_cache.draw(renderer, bounds.size(), |frame| {
            draw_borders(frame, bounds, palette);

            let mut all_labels = linear::generate_labels(
                bounds,
                self.min,
                self.max,
                text_size,
                palette.background.base.text,
                10.0,
                None,
            );

            if self.crosshair {
                let common_bounds = Rectangle {
                    x: self.chart_bounds.x,
                    y: bounds.y,
                    width: self.chart_bounds.width,
                    height: bounds.height,
                };

                if let Some(crosshair_pos) = cursor.position_in(common_bounds) {
                    let rounded_value = round_to_tick(
                        lowest + (range * (bounds.height - crosshair_pos.y) / bounds.height),
                        10.0,
                    );

                    let label = Label {
                        content: abbr_large_numbers(rounded_value),
                        background_color: Some(palette.secondary.base.color),
                        text_color: palette.secondary.base.text,
                        text_size,
                    };

                    let y_position =
                        bounds.height - ((rounded_value - lowest) / range * bounds.height);

                    all_labels.push(AxisLabel::Y(
                        calc_label_rect(y_position, 1, text_size, bounds),
                        label,
                        None,
                    ));
                }
            }

            AxisLabel::filter_and_draw(&all_labels, frame);
        });

        vec![labels]
    }

    fn mouse_interaction(
        &self,
        interaction: &Interaction,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match interaction {
            Interaction::Zoomin { .. } => mouse::Interaction::ResizingVertically,
            Interaction::Panning { .. } => mouse::Interaction::None,
            Interaction::None if cursor.is_over(bounds) => mouse::Interaction::ResizingVertically,
            _ => mouse::Interaction::default(),
        }
    }
}
