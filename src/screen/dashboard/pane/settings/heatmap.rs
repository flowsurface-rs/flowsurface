use super::{HeatmapIndicatorMessage, Message, sync_all_button};
use crate::widget::{
    classic_slider_row,
    inspector::{InspectorContent, InspectorSection},
};
use crate::{style, tooltip};
use data::chart::heatmap;
use data::chart::heatmap::indicators::{
    DepthDistance, HeatmapIndicator, HeatmapIndicatorConfig, SpreadUnit,
};
use data::chart::heatmap::{CLEANUP_THRESHOLD, ProfileKind};
use data::layout::pane::VisualConfig;
use data::util::format_with_commas;
use exchange::adapter::Venue;
use iced::{
    Length,
    widget::{
        button, checkbox, column, container, radio, row, space, text, text_input,
        tooltip::Position as TooltipPosition,
    },
};

pub fn heatmap_shader_cfg_view<'a>(
    cfg: heatmap::Config,
    basis: data::chart::Basis,
    venue: Venue,
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

    let mut indicator_controls = column![].spacing(8);
    let input_width = Length::Fixed(80.0);

    for kind in HeatmapIndicator::ALL {
        let unsupported_reason = kind.unsupported_reason(venue);
        let enabled = indicators.is_enabled(kind) && unsupported_reason.is_none();
        let toggle = checkbox(enabled).label(kind.label()).on_toggle_maybe(
            unsupported_reason.is_none().then_some(move |enabled| {
                Message::HeatmapIndicator(HeatmapIndicatorMessage::Enabled(kind, enabled))
            }),
        );
        let mut checkbox_row = row![toggle]
            .spacing(4)
            .height(36)
            .align_y(iced::Alignment::Center)
            .padding([0, 8]);

        if kind == HeatmapIndicator::Depth && enabled {
            checkbox_row = checkbox_row
                .push(iced::widget::space::horizontal())
                .push(tooltip(
                    button("i").style(style::button::info),
                    Some("`!` in legend means book falls short of range.\nMissing levels may go unflagged."),
                    TooltipPosition::Left,
                ));
        }

        let mut controls = column![checkbox_row];
        if let Some(reason) = unsupported_reason {
            controls = controls.push(
                container(text(reason).size(style::text_size::SMALL))
                    .padding(iced::padding::bottom(8).horizontal(8)),
            );
        }
        if enabled {
            let settings: Option<iced::Element<'_, Message>> = match kind {
                HeatmapIndicator::Volume => None,
                HeatmapIndicator::VolumeProfile => indicators
                    .volume_profile
                    .map(|kind| volume_profile_controls(kind, basis)),

                HeatmapIndicator::Spread => Some(
                    column![
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
                ),

                HeatmapIndicator::Depth => Some(
                    container(
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
                                    Message::HeatmapIndicator(
                                        HeatmapIndicatorMessage::DistanceEdited(value),
                                    )
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
                ),
            };

            if let Some(settings) = settings {
                controls = controls.push(container(settings).padding(8));
            }
        }

        indicator_controls = indicator_controls.push(
            container(controls)
                .width(Length::Fill)
                .style(style::inspector_control),
        );
    }

    let mut content = InspectorContent::new(vec![
        InspectorSection::titled("Size filter", size_filters).with_modal_container(),
        InspectorSection::titled("Trade visualization", trade_viz_column).with_modal_container(),
        InspectorSection::titled("Indicators", indicator_controls).with_modal_container(),
    ]);
    content.set_footer(row![
        space::horizontal(),
        sync_all_button(VisualConfig::Heatmap(cfg))
    ]);
    content
}

fn volume_profile_controls<'a>(
    kind: ProfileKind,
    basis: data::chart::Basis,
) -> iced::Element<'a, Message> {
    let interval_ms = match basis {
        data::chart::Basis::Time(interval) => interval.to_milliseconds(),
        data::chart::Basis::Tick(_) => {
            return text("Heatmap studies are not supported for tick-based charts").into();
        }
    };

    let fixed_window = match kind {
        ProfileKind::FixedWindow(_) => kind,
        ProfileKind::VisibleRange => ProfileKind::FixedWindow(CLEANUP_THRESHOLD / 5),
    };
    let fixed_window_label = match kind {
        ProfileKind::FixedWindow(count) => format!(
            "Fixed window ({})",
            study_window_duration(count, interval_ms)
        ),
        ProfileKind::VisibleRange => "Fixed window".to_owned(),
    };

    let on_change = |kind| Message::HeatmapIndicator(HeatmapIndicatorMessage::VolumeProfile(kind));
    let mut controls = column![
        radio(
            "Visible range",
            ProfileKind::VisibleRange,
            Some(kind),
            on_change
        ),
        radio(fixed_window_label, fixed_window, Some(kind), on_change),
    ]
    .spacing(4);

    if let ProfileKind::FixedWindow(count) = kind {
        controls = controls.push(classic_slider_row(
            Some(text("Window")),
            (CLEANUP_THRESHOLD / 20) as f32..=CLEANUP_THRESHOLD as f32,
            count as f32,
            move |count| on_change(ProfileKind::FixedWindow(count as usize)),
            |value| format!("{} datapoints", *value as usize),
            Some(40.0),
        ));
    }

    controls.into()
}

fn study_window_duration(datapoint_count: usize, interval_ms: u64) -> String {
    let duration_secs = (datapoint_count as u64 * interval_ms) / 1000;
    if duration_secs < 60 {
        format!("{} seconds", duration_secs)
    } else {
        let minutes = duration_secs / 60;
        let seconds = duration_secs % 60;
        if seconds == 0 {
            format!("{} minutes", minutes)
        } else {
            format!("{}m {}s", minutes, seconds)
        }
    }
}
