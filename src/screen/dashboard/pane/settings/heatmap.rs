use super::{HeatmapIndicatorMessage, Message, sync_all_button};
use crate::style;
use crate::theme::ThemeAccents;
use crate::widget::study_configurator as study;
use crate::widget::{
    classic_slider_row,
    inspector::{InspectorContent, InspectorSection},
};
use data::chart::heatmap;
use data::chart::heatmap::HeatmapStudy;
use data::chart::heatmap::indicators::{
    DepthDistance, HeatmapIndicator, HeatmapIndicatorConfig, SpreadUnit,
};
use data::layout::pane::VisualConfig;
use data::util::format_with_commas;
use iced::{
    Length,
    widget::{checkbox, column, container, radio, row, space, text, text_input},
};

pub fn heatmap_shader_cfg_view<'a>(
    cfg: heatmap::Config,
    studies: &'a [HeatmapStudy],
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
    indicators: &'a HeatmapIndicatorConfig,
    distance_input: &'a str,
) -> InspectorContent<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;
        classic_slider_row(
            Some(text("Trade")),
            0.0..=50000.0,
            filter,
            move |value| Message::VisualConfigChanged {
                config: VisualConfig::Heatmap(heatmap::Config {
                    trade_size_filter: value,
                    ..cfg
                }),
                sync_all: false,
            },
            |value| format!(">${}", format_with_commas(*value as f64)),
            Some(500.0),
        )
    };

    let order_size_slider = {
        let filter = cfg.order_size_filter;
        classic_slider_row(
            Some(text("Order")),
            0.0..=500_000.0,
            filter,
            move |value| Message::VisualConfigChanged {
                config: VisualConfig::Heatmap(heatmap::Config {
                    order_size_filter: value,
                    ..cfg
                }),
                sync_all: false,
            },
            |value| format!(">${}", format_with_commas(*value as f64)),
            Some(5000.0),
        )
    };

    let circle_scaling_slider = cfg.trade_size_scale.map(|radius_scale| {
        classic_slider_row(
            Some(crate::widget::text::body("Circle radius scaling")),
            10..=200,
            radius_scale,
            move |value| Message::VisualConfigChanged {
                config: VisualConfig::Heatmap(heatmap::Config {
                    trade_size_scale: Some(value),
                    ..cfg
                }),
                sync_all: false,
            },
            |value| format!("{}%", value),
            Some(10),
        )
    });

    let size_filters = column![trade_size_slider, order_size_slider].spacing(8);

    let trade_viz_column = {
        let dyn_checkbox = checkbox(cfg.trade_size_scale.is_some())
            .label("Dynamic circle radius")
            .on_toggle(move |value| Message::VisualConfigChanged {
                config: VisualConfig::Heatmap(heatmap::Config {
                    trade_size_scale: if value { Some(100) } else { None },
                    ..cfg
                }),
                sync_all: false,
            });

        let checkbox_row = row![dyn_checkbox]
            .height(36)
            .align_y(iced::Alignment::Center)
            .padding([0, 8]);
        let mut col = column![checkbox_row];
        if let Some(slider) = circle_scaling_slider {
            col = col.push(container(slider).padding(8));
        }
        container(col)
            .width(Length::Fill)
            .style(style::inspector_control)
    };

    let study_cfg = study::view(studies, basis, theme_accents)
        .map(move |msg| Message::Study(study::StudyMessage::Heatmap(msg)));

    let mut indicator_controls = column![].spacing(8);
    let input_width = Length::Fixed(80.0);

    for kind in HeatmapIndicator::ALL {
        let plot = indicators.plot(kind);
        let checkbox_row =
            row![
                checkbox(plot.enabled)
                    .label(kind.label())
                    .on_toggle(move |enabled| {
                        Message::HeatmapIndicator(HeatmapIndicatorMessage::Enabled(kind, enabled))
                    }),
            ]
            .height(36)
            .align_y(iced::Alignment::Center)
            .padding([0, 8]);

        let mut controls = column![checkbox_row];
        if plot.enabled {
            let settings: iced::Element<'_, Message> = match kind {
                HeatmapIndicator::Spread => column![
                    radio(
                        "Bps",
                        SpreadUnit::Bps,
                        Some(indicators.spread_unit),
                        |unit| {
                            Message::HeatmapIndicator(HeatmapIndicatorMessage::SpreadUnit(unit))
                        }
                    ),
                    radio(
                        "Ticks",
                        SpreadUnit::Ticks,
                        Some(indicators.spread_unit),
                        |unit| {
                            Message::HeatmapIndicator(HeatmapIndicatorMessage::SpreadUnit(unit))
                        }
                    ),
                ]
                .spacing(4)
                .into(),
                HeatmapIndicator::Depth => container(
                    row![
                        text("Distance (%)"),
                        space::horizontal(),
                        text_input("0.01-10", distance_input)
                            .width(input_width)
                            .style(move |theme, status| {
                                style::validated_text_input(
                                    theme,
                                    status,
                                    DepthDistance::from_percent(distance_input).is_some(),
                                )
                            })
                            .on_input(|value| {
                                Message::HeatmapIndicator(HeatmapIndicatorMessage::DistanceEdited(
                                    value,
                                ))
                            })
                            .on_submit_maybe(DepthDistance::from_percent(distance_input).map(
                                |_| {
                                    Message::HeatmapIndicator(
                                        HeatmapIndicatorMessage::DistanceSubmitted,
                                    )
                                }
                            )),
                    ]
                    .align_y(iced::Alignment::Center)
                    .spacing(8),
                )
                .width(Length::Fill)
                .padding(8)
                .style(style::inspector_slider_container)
                .into(),
            };
            controls = controls.push(container(settings).padding(8));
        }
        indicator_controls = indicator_controls.push(
            container(controls)
                .width(Length::Fill)
                .style(style::inspector_control),
        );
    }

    let mut content = InspectorContent::new(vec![
        InspectorSection::titled("Indicators", indicator_controls).with_modal_container(),
        InspectorSection::titled("Size filter", size_filters).with_modal_container(),
        InspectorSection::titled("Trade visualization", trade_viz_column).with_modal_container(),
        InspectorSection::titled("Studies", study_cfg).with_modal_container(),
    ]);
    content.set_footer(row![
        space::horizontal(),
        sync_all_button(VisualConfig::Heatmap(cfg))
    ]);
    content
}
