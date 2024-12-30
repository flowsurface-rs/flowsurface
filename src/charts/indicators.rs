pub mod volume;
pub mod open_interest;

use std::{any::Any, fmt::{self, Debug, Display}};

use iced::{mouse, theme::palette::Extended, widget::canvas::{self, Cache, Frame, Geometry}, Event, Point, Rectangle, Renderer, Size, Theme};
use serde::{Deserialize, Serialize};

use crate::{
    charts::{calc_value_step, abbr_large_numbers, round_to_tick, AxisLabel, Label}, 
    data_providers::MarketType
};

use super::{Interaction, Message};

pub trait Indicator: PartialEq + Display + ToString + Debug + 'static  {
    fn get_available(market_type: Option<MarketType>) -> &'static [Self] where Self: Sized;
    
    fn get_enabled(indicators: &[Self], market_type: Option<MarketType>) -> impl Iterator<Item = &Self> 
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
    const ALL: [CandlestickIndicator; 2] = [CandlestickIndicator::Volume, CandlestickIndicator::OpenInterest];
    const SPOT: [CandlestickIndicator; 1] = [CandlestickIndicator::Volume];
    const PERPS: [CandlestickIndicator; 2] = [CandlestickIndicator::Volume, CandlestickIndicator::OpenInterest];
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
    const ALL: [FootprintIndicator; 2] = [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
    const SPOT: [FootprintIndicator; 1] = [FootprintIndicator::Volume];
    const PERPS: [FootprintIndicator; 2] = [FootprintIndicator::Volume, FootprintIndicator::OpenInterest];
}

impl Display for FootprintIndicator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FootprintIndicator::Volume => write!(f, "Volume"),
            FootprintIndicator::OpenInterest => write!(f, "Open Interest"),
        }
    }
}

fn draw_borders(
    frame: &mut Frame,
    bounds: Rectangle,
    palette: &Extended,
) {
    frame.fill_rectangle(
        Point::new(0.0, 0.0),
        Size::new(bounds.width, 1.0),
        if palette.is_dark {
            palette.background.weak.color.scale_alpha(0.2)
        } else {
            palette.background.strong.color.scale_alpha(0.2)
        },
    );

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
        _event: Event,
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
        
        let text_size = 12.0;

        let labels = self.label_cache.draw(renderer, bounds.size(), |frame| {
            draw_borders(frame, bounds, palette);

            let range = highest - lowest;
            let labels_can_fit = (bounds.height / (text_size * 2.0)) as i32;

            let mut all_labels = Vec::with_capacity((labels_can_fit + 2) as usize);

            let rect = |y_pos: f32, label_amt: i16| {
                let label_offset = text_size + (f32::from(label_amt) * (text_size / 2.0) + 2.0);

                Rectangle {
                    x: 6.0,
                    y: y_pos - label_offset / 2.0,
                    width: bounds.width - 8.0,
                    height: label_offset,
                }
            };

            // Regular value labels (priority 1)
            let (step, rounded_lowest) = calc_value_step(highest, lowest, labels_can_fit, 10.0);

            let mut value = rounded_lowest;

            while value <= highest {
                let label = Label {
                    content: abbr_large_numbers(value),
                    background_color: None,
                    marker_color: if palette.is_dark {
                        palette.background.weak.color.scale_alpha(0.6)
                    } else {
                        palette.background.strong.color.scale_alpha(0.6)
                    },
                    text_color: palette.background.base.text,
                    text_size,
                };

                all_labels.push(
                    AxisLabel::Y(
                        rect(
                            bounds.height - ((value - lowest) / range * bounds.height), 
                            1
                        ),
                        label, 
                        None,
                    )
                );

                value += step;
            }

            // Crosshair value (priority 3)
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
                        10.0
                    );

                    let label = Label {
                        content: abbr_large_numbers(rounded_value),
                        background_color: Some(palette.secondary.base.color),
                        marker_color: palette.background.strong.color,
                        text_color: palette.secondary.base.text,
                        text_size,
                    };

                    let y_position =
                        bounds.height - ((rounded_value - lowest) / range * bounds.height);

                    all_labels.push(
                        AxisLabel::Y(
                            rect(
                                y_position, 
                                1
                            ), 
                            label, 
                            None,
                        )
                    );
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