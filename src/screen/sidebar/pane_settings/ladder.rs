use super::{Message, sync_all_button};
use crate::{
    style, tooltip,
    widget::{
        classic_slider_row,
        inspector::{InspectorContent, InspectorSection},
    },
};
use data::layout::pane::VisualConfig;
use data::panel::ladder;
use iced::{
    Alignment,
    widget::{button, checkbox, column, row, space, text, tooltip::Position as TooltipPosition},
};
use std::time::Duration;

pub fn ladder_cfg_view<'a>(cfg: ladder::Config) -> InspectorContent<'a, Message> {
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

    let retention_minutes = (cfg.trade_retention.as_secs_f32() / 60.0).max(1.0);
    let retention_slider = classic_slider_row(
        Some(text("Keep trades for")),
        1.0..=60.0,
        retention_minutes,
        move |new_minutes| {
            let mins = new_minutes.round().max(1.0) as u64;
            Message::VisualConfigChanged {
                config: VisualConfig::Ladder(ladder::Config {
                    trade_retention: Duration::from_secs(mins * 60),
                    ..cfg
                }),
                sync_all: false,
            }
        },
        |value| format!("≈ {} min", value.round() as u64),
        Some(1.0),
    );

    let mut content = InspectorContent::new(vec![
        InspectorSection::titled("Display Options", display_options).with_modal_container(),
        InspectorSection::titled("History", retention_slider),
    ]);
    content.set_footer(row![
        space::horizontal(),
        sync_all_button(VisualConfig::Ladder(cfg))
    ]);
    content
}
