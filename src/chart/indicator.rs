pub mod open_interest;
pub mod plot;
pub mod volume;

use std::{collections::BTreeMap, ops::RangeInclusive};

use iced::{
    Element, Event, Length, Rectangle, Renderer, Theme, mouse,
    widget::{
        Canvas,
        canvas::{self, Cache, Geometry},
        container, row, vertical_rule,
    },
};

use super::scale::linear;
use crate::chart::{
    Caches, TEXT_SIZE, ViewState,
    indicator::plot::{ChartCanvas, Plot, Series},
    scale::{AxisLabel, LabelContent, calc_label_rect},
};
use data::util::{abbr_large_numbers, round_to_tick};

use super::{Interaction, Message};

pub type IndicatorMap<T> = BTreeMap<u64, T>;

/// Creates the indicator plot and its labels. Wraps it under `iced::Element`(row).
pub fn indicator_row<'a, P, S>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    plot: P,
    series: S,
    visible_range: RangeInclusive<u64>,
) -> Element<'a, Message>
where
    P: Plot<S> + 'a,
    S: Series + 'a,
{
    let (min, max) = plot
        .y_extents(&series, visible_range)
        .map(|(min, max)| plot.adjust_extents(min, max))
        .unwrap_or((0.0, 0.0));

    let canvas = Canvas::new(ChartCanvas::<P, S> {
        indicator_cache: &cache.main,
        crosshair_cache: &cache.crosshair,
        ctx: chart_state,
        plot,
        series,
        max_for_labels: max,
        min_for_labels: min,
    })
    .height(Length::Fill)
    .width(Length::Fill);

    let labels = Canvas::new(IndicatorLabel {
        label_cache: &cache.y_labels,
        max,
        min,
        chart_bounds: chart_state.bounds,
    })
    .height(Length::Fill)
    .width(chart_state.y_labels_width());

    row![
        canvas,
        vertical_rule(1).style(crate::style::split_ruler),
        container(labels),
    ]
    .into()
}

pub struct IndicatorLabel<'a> {
    pub label_cache: &'a Cache,
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

        let tick_size = data::util::guesstimate_ticks(range);

        let labels = self.label_cache.draw(renderer, bounds.size(), |frame| {
            let mut all_labels = linear::generate_labels(
                bounds,
                self.min,
                self.max,
                TEXT_SIZE,
                palette.background.base.text,
                None,
            );

            let common_bounds = Rectangle {
                x: self.chart_bounds.x,
                y: bounds.y,
                width: self.chart_bounds.width,
                height: bounds.height,
            };

            if let Some(crosshair_pos) = cursor.position_in(common_bounds) {
                let rounded_value = round_to_tick(
                    lowest + (range * (bounds.height - crosshair_pos.y) / bounds.height),
                    tick_size,
                );

                let label = LabelContent {
                    content: abbr_large_numbers(rounded_value),
                    background_color: Some(palette.secondary.base.color),
                    text_color: palette.secondary.base.text,
                    text_size: TEXT_SIZE,
                };

                let y_position = bounds.height - ((rounded_value - lowest) / range * bounds.height);

                all_labels.push(AxisLabel::Y {
                    bounds: calc_label_rect(y_position, 1, TEXT_SIZE, bounds),
                    value_label: label,
                    timer_label: None,
                });
            }

            AxisLabel::filter_and_draw(&all_labels, frame);
        });

        vec![labels]
    }

    fn mouse_interaction(
        &self,
        _interaction: &Interaction,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}
