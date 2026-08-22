use super::{Message, cfg_view_container, sync_all_button};
use crate::widget::study_configurator as study;
use crate::{
    split_column, style,
    widget::{classic_slider_row, labeled_slider},
};
use data::chart::heatmap::HeatmapStudy;
use data::chart::heatmap::{self, CoalesceKind};
use data::layout::pane::VisualConfig;
use data::util::format_with_commas;
use iced::{
    Alignment, Element,
    widget::{checkbox, column, container, radio, row, slider, space, text},
};

pub fn heatmap_cfg_view<'a>(
    cfg: heatmap::Config,
    study_config: &'a study::Configurator<HeatmapStudy>,
    studies: &'a [HeatmapStudy],
    basis: data::chart::Basis,
) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;
        labeled_slider(
            "Trade",
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
        labeled_slider(
            "Order",
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
            text("Circle radius scaling"),
            slider(10..=200, radius_scale, move |value| {
                Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: Some(value),
                        ..cfg
                    }),
                    sync_all: false,
                }
            })
            .step(10)
            .into(),
            Some(text(format!("{}%", radius_scale)).size(crate::style::text_size::EMPHASIS)),
        )
    });

    let coalescer_cfg: Option<Element<_>> = if let Some(coalescing) = cfg.coalescing {
        let threshold_pct = coalescing.threshold();

        let coalescer_kinds = {
            let average = radio(
                "Average",
                CoalesceKind::Average(threshold_pct),
                Some(coalescing),
                move |value| Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        coalescing: Some(value),
                        ..cfg
                    }),
                    sync_all: false,
                },
            )
            .spacing(4);

            let first = radio(
                "First",
                CoalesceKind::First(threshold_pct),
                Some(coalescing),
                move |value| Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        coalescing: Some(value),
                        ..cfg
                    }),
                    sync_all: false,
                },
            )
            .spacing(4);

            let max = radio(
                "Max",
                CoalesceKind::Max(threshold_pct),
                Some(coalescing),
                move |value| Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        coalescing: Some(value),
                        ..cfg
                    }),
                    sync_all: false,
                },
            )
            .spacing(4);

            row![
                text("Merge method: "),
                row![average, first, max].spacing(12)
            ]
            .spacing(12)
        };

        let threshold_slider = classic_slider_row(
            text("Size similarity"),
            slider(0.05..=0.8, threshold_pct, move |value| {
                Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        coalescing: Some(coalescing.with_threshold(value)),
                        ..cfg
                    }),
                    sync_all: false,
                }
            })
            .step(0.05)
            .into(),
            Some(
                text(format!("{:.0}%", threshold_pct * 100.0))
                    .size(crate::style::text_size::EMPHASIS),
            ),
        );

        Some(
            container(column![coalescer_kinds, threshold_slider].spacing(8))
                .style(style::modal_container)
                .padding(8)
                .into(),
        )
    } else {
        None
    };

    let size_filters_column = column![
        text("Size filters").size(crate::style::text_size::SECTION),
        column![trade_size_slider, order_size_slider].spacing(8),
    ]
    .spacing(8);

    let noise_filters_column = {
        let merge_checkbox = checkbox(cfg.coalescing.is_some())
            .label("Merge orders if sizes are similar")
            .on_toggle(move |value| Message::VisualConfigChanged {
                config: VisualConfig::Heatmap(heatmap::Config {
                    coalescing: if value {
                        Some(CoalesceKind::Average(0.15))
                    } else {
                        None
                    },
                    ..cfg
                }),
                sync_all: false,
            });

        let mut col = column![
            text("Noise filters").size(crate::style::text_size::SECTION),
            merge_checkbox
        ]
        .spacing(8);
        if let Some(c) = coalescer_cfg {
            col = col.push(c);
        }
        col
    };

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

        let mut col = column![
            text("Trade visualization").size(crate::style::text_size::SECTION),
            dyn_checkbox
        ]
        .spacing(8);
        if let Some(slider) = circle_scaling_slider {
            col = col.push(slider);
        }
        col
    };

    let study_cfg = study_config
        .view(studies, basis)
        .map(move |msg| Message::Study(study::StudyMessage::Heatmap(msg)));

    let content = split_column![
        size_filters_column,
        noise_filters_column,
        trade_viz_column,
        column![text("Studies").size(crate::style::text_size::SECTION), study_cfg].spacing(8),
        row![
            space::horizontal(),
            sync_all_button( VisualConfig::Heatmap(cfg))
        ]
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(400, content)
}

pub fn heatmap_shader_cfg_view<'a>(
    cfg: heatmap::Config,
    study_config: &'a study::Configurator<HeatmapStudy>,
    studies: &'a [HeatmapStudy],
    basis: data::chart::Basis,
) -> Element<'a, Message> {
    let trade_size_slider = {
        let filter = cfg.trade_size_filter;
        labeled_slider(
            "Trade",
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
        labeled_slider(
            "Order",
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
            text("Circle radius scaling"),
            slider(10..=200, radius_scale, move |value| {
                Message::VisualConfigChanged {
                    config: VisualConfig::Heatmap(heatmap::Config {
                        trade_size_scale: Some(value),
                        ..cfg
                    }),
                    sync_all: false,
                }
            })
            .step(10)
            .into(),
            Some(text(format!("{}%", radius_scale)).size(crate::style::text_size::EMPHASIS)),
        )
    });

    let size_filters_column = column![
        text("Size filters").size(crate::style::text_size::SECTION),
        column![trade_size_slider, order_size_slider].spacing(8),
    ]
    .spacing(8);

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

        let mut col = column![
            text("Trade visualization").size(crate::style::text_size::SECTION),
            dyn_checkbox
        ]
        .spacing(8);
        if let Some(slider) = circle_scaling_slider {
            col = col.push(slider);
        }
        col
    };

    let study_cfg = study_config
        .view(studies, basis)
        .map(move |msg| Message::Study(study::StudyMessage::Heatmap(msg)));

    let content = split_column![
        size_filters_column,
        trade_viz_column,
        column![text("Studies").size(crate::style::text_size::SECTION), study_cfg].spacing(8),
        row![
            space::horizontal(),
            sync_all_button( VisualConfig::Heatmap(cfg))
        ]
        ; spacing = 12, align_x = Alignment::Start
    ];

    cfg_view_container(400, content)
}
