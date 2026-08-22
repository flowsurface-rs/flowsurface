use super::{Message, cfg_view_container, sync_all_button};
use crate::{split_column, style, tooltip, widget::classic_slider_row};
use data::layout::pane::VisualConfig;
use data::panel::ladder;
use iced::{
    Alignment, Element,
    widget::{
        button, checkbox, column, row, slider, space, text, tooltip::Position as TooltipPosition,
    },
};
use std::time::Duration;

pub fn ladder_cfg_view<'a>(cfg: ladder::Config) -> Element<'a, Message> {
    let display_options = {
        let spread = checkbox(cfg.show_spread)
            .label("Show Spread")
            .on_toggle(move |value| Message::VisualConfigChanged {
                config: VisualConfig::Ladder(ladder::Config {
                    show_spread: value,
                    ..cfg
                }),
                sync_all: false,
            });

        let chase_tracker = checkbox(cfg.show_chase_tracker)
            .label("Show Chase Tracker")
            .on_toggle(move |value| Message::VisualConfigChanged {
                config: VisualConfig::Ladder(ladder::Config {
                    show_chase_tracker: value,
                    ..cfg
                }),
                sync_all: false,
            });

        column![
            text("Display Options").size(crate::style::text_size::SECTION),
            column![
                spread,
                row![
                    chase_tracker,
                    tooltip(
                        button("i").style(style::button::info),
                        Some("Highlights consecutive best-price moves and fades when momentum stalls.\nCalculated using raw ungrouped data."),
                        TooltipPosition::Top,
                    )
                ]
                .align_y(Alignment::Center)
                .spacing(4)
            ]
            .spacing(4)
        ]
        .spacing(8)
    };

    let retention_slider = {
        let retention_minutes = (cfg.trade_retention.as_secs_f32() / 60.0).max(1.0);

        let slider_ui = slider(1.0..=60.0, retention_minutes, move |new_minutes| {
            let mins = new_minutes.round().max(1.0) as u64;
            Message::VisualConfigChanged {
                config: VisualConfig::Ladder(ladder::Config {
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
            Some(
                text(format!("≈ {} min", retention_minutes.round() as u64))
                    .size(crate::style::text_size::EMPHASIS),
            ),
        )
    };

    let history_column = column![
        text("History").size(crate::style::text_size::SECTION),
        retention_slider
    ]
    .spacing(8);

    let content = split_column![
        display_options,
        history_column,
        row![
            space::horizontal(),
            sync_all_button( VisualConfig::Ladder(cfg))
        ],
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(320, content)
}
