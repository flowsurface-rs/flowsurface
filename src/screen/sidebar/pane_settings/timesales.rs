use super::{Message, sync_all_button};
use crate::{
    style, tooltip,
    widget::{
        classic_slider_row,
        inspector::{InspectorContent, InspectorSection},
        labeled_slider,
    },
};
use data::layout::pane::VisualConfig;
use data::panel::timesales::{self, StackedBar, StackedBarRatio};
use data::util::format_with_commas;
use iced::{
    Alignment, Element, Length,
    widget::{
        button, checkbox, column, pick_list, radio, row, slider, space, text,
        tooltip::Position as TooltipPosition,
    },
};
use std::time::Duration;

pub fn timesales_cfg_view<'a>(cfg: timesales::Config) -> InspectorContent<'a, Message> {
    let filter = cfg.trade_size_filter;
    let trade_size_slider = labeled_slider(
        "Trade",
        0.0..=50000.0,
        filter,
        move |value| Message::VisualConfigChanged {
            config: VisualConfig::TimeAndSales(timesales::Config {
                trade_size_filter: value,
                ..cfg
            }),
            sync_all: false,
        },
        |value| format!(">${}", format_with_commas(*value as f64)),
        Some(500.0),
    );

    let retention_minutes = (cfg.trade_retention.as_secs_f32() / 60.0).max(1.0);
    let retention_slider = {
        let slider_ui = slider(1.0..=60.0, retention_minutes, move |new_minutes| {
            let mins = new_minutes.round().max(1.0) as u64;
            Message::VisualConfigChanged {
                config: VisualConfig::TimeAndSales(timesales::Config {
                    trade_retention: Duration::from_secs(mins * 60),
                    ..cfg
                }),
                sync_all: false,
            }
        })
        .step(1.0);

        classic_slider_row(
            text("Keep trades for"),
            slider_ui.into(),
            Some(crate::widget::text::emphasis(format!(
                "≈ {} min",
                retention_minutes.round() as u64
            ))),
        )
    };

    let history_title = row![
        crate::widget::text::section("History"),
        tooltip(
            button("i").style(style::button::info),
            Some("Affects the stacked bar, colors and how much you can scroll down"),
            TooltipPosition::Top,
        )
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let stacked_bar: Element<_> = {
        let is_shown = cfg.stacked_bar.is_some();

        let enable_checkbox = checkbox(is_shown).label("Show stacked bar").on_toggle({
            move |value| {
                let current_ratio = cfg.stacked_bar.map(|h| h.ratio()).unwrap_or_default();
                Message::VisualConfigChanged {
                    config: VisualConfig::TimeAndSales(timesales::Config {
                        stacked_bar: if value {
                            Some(StackedBar::Compact(current_ratio))
                        } else {
                            None
                        },
                        ..cfg
                    }),
                    sync_all: false,
                }
            }
        });

        let controls: Option<Element<_>> = cfg.stacked_bar.map(|hist| {
            let ratio = hist.ratio();
            let is_compact = matches!(hist, StackedBar::Compact(_));

            let compact = radio("Compact", true, Some(is_compact), {
                move |_v| Message::VisualConfigChanged {
                    config: VisualConfig::TimeAndSales(timesales::Config {
                        stacked_bar: Some(StackedBar::Compact(ratio)),
                        ..cfg
                    }),
                    sync_all: false,
                }
            })
            .spacing(4);

            let full = radio("Full", false, Some(is_compact), {
                move |_v| Message::VisualConfigChanged {
                    config: VisualConfig::TimeAndSales(timesales::Config {
                        stacked_bar: Some(StackedBar::Full(ratio)),
                        ..cfg
                    }),
                    sync_all: false,
                }
            })
            .spacing(4);

            let metric_picklist = pick_list(StackedBarRatio::ALL, Some(ratio), move |new_ratio| {
                let new_hist = Some(match cfg.stacked_bar {
                    Some(StackedBar::Full(_)) => StackedBar::Full(new_ratio),
                    _ => StackedBar::Compact(new_ratio),
                });
                Message::VisualConfigChanged {
                    config: VisualConfig::TimeAndSales(timesales::Config {
                        stacked_bar: new_hist,
                        ..cfg
                    }),
                    sync_all: false,
                }
            });

            column![
                crate::widget::text::body("Mode"),
                row![compact, full].spacing(12),
                crate::widget::text::body("Metric"),
                metric_picklist,
            ]
            .spacing(8)
            .into()
        });

        let mut inner = column![enable_checkbox].width(Length::Fill).spacing(8);

        if let Some(ctrls) = controls {
            inner = inner.push(ctrls);
        }

        inner.into()
    };

    let mut content = InspectorContent::new(vec![
        InspectorSection::titled("Size filter", trade_size_slider),
        InspectorSection::new(history_title, retention_slider),
        InspectorSection::titled("Stacked bar", stacked_bar),
    ]);
    content.set_footer(row![
        space::horizontal(),
        sync_all_button(VisualConfig::TimeAndSales(cfg))
    ]);
    content
}
