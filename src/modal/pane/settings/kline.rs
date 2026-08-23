use super::{Message, cfg_view_container, sync_all_button};
use crate::widget::study_configurator as study;
use crate::{split_column, tooltip};
use data::chart::kline::FootprintStudy;
use data::chart::{KlineChartKind, kline::ClusterKind};
use data::layout::pane::VisualConfig;
use iced::{
    Alignment, Element,
    widget::{
        checkbox, column, pick_list, row, slider, space, text, tooltip::Position as TooltipPosition,
    },
};

pub fn kline_cfg_view<'a>(
    study_config: &'a study::Configurator<FootprintStudy>,
    cfg: data::chart::kline::Config,
    kind: &'a KlineChartKind,
    basis: data::chart::Basis,
) -> Element<'a, Message> {
    let display_readout_section = {
        let data_labels_checkbox = tooltip(
            checkbox(cfg.data_labels_always_visible)
                .label("Keep latest label visible")
                .on_toggle(move |value| Message::VisualConfigChanged {
                    config: VisualConfig::Kline(data::chart::kline::Config {
                        data_labels_always_visible: value,
                        ..cfg
                    }),
                    sync_all: false,
                }),
            Some("Show the latest datapoint label even when not hovering"),
            TooltipPosition::Top,
        );
        column![
            text("Data labels").size(crate::style::text_size::SECTION),
            data_labels_checkbox,
        ]
        .spacing(8)
    };

    let content = match kind {
        KlineChartKind::Candles => {
            split_column![
                display_readout_section,
                row![
                    space::horizontal(),
                    sync_all_button( VisualConfig::Kline(cfg))
                ],
                ; spacing = 12, align_x = Alignment::Start
            ]
        }
        KlineChartKind::Footprint {
            clusters,
            scaling,
            studies,
        } => {
            let cluster_picklist =
                pick_list(ClusterKind::ALL, Some(clusters), move |new_cluster_kind| {
                    Message::ClusterKindSelected(new_cluster_kind)
                });

            let footprint_summary_checkbox = tooltip(
                checkbox(cfg.show_footprint_summary)
                    .label("Show footprint summary")
                    .on_toggle(move |value| Message::VisualConfigChanged {
                        config: VisualConfig::Kline(data::chart::kline::Config {
                            show_footprint_summary: value,
                            ..cfg
                        }),
                        sync_all: false,
                    }),
                Some("Show per-bar volume and delta below footprint candles"),
                TooltipPosition::Top,
            );

            let scaling = {
                let picklist = pick_list(
                    data::chart::kline::ClusterScaling::ALL,
                    Some(scaling),
                    Message::ClusterScalingSelected,
                );

                if let data::chart::kline::ClusterScaling::Hybrid { weight } = scaling {
                    let hybrid_slider = slider(0.0..=1.0, *weight, move |new_weight| {
                        Message::ClusterScalingSelected(
                            data::chart::kline::ClusterScaling::Hybrid { weight: new_weight },
                        )
                    })
                    .step(0.05);

                    column![
                        picklist,
                        hybrid_slider,
                        text("Blend visible-range and per-candle scaling"),
                    ]
                    .spacing(8)
                } else {
                    column![picklist].spacing(8)
                }
            };

            let available_studies: Vec<_> = data::chart::kline::FootprintStudy::ALL.to_vec();

            let active_studies: Vec<_> = studies
                .iter()
                .copied()
                .filter(|study| {
                    available_studies
                        .iter()
                        .any(|available| available.is_same_type(study))
                })
                .collect();

            let study_cfg = study_config
                .view_available(&active_studies, basis, None, None, available_studies, &[])
                .map(move |msg| Message::Study(study::StudyMessage::Footprint(msg)));

            let mut content = split_column![
                display_readout_section,
                column![text("Footprint summary").size(crate::style::text_size::SECTION), footprint_summary_checkbox].spacing(8),
                column![text("Cluster type").size(crate::style::text_size::SECTION), cluster_picklist].spacing(8),
                ; spacing = 12, align_x = Alignment::Start
            ];

            content = content.push(
                column![
                    text("Cluster scaling").size(crate::style::text_size::SECTION),
                    scaling
                ]
                .spacing(8),
            );

            content = content
                .push(
                    column![
                        text("Studies").size(crate::style::text_size::SECTION),
                        study_cfg
                    ]
                    .spacing(8),
                )
                .push(row![
                    space::horizontal(),
                    sync_all_button(VisualConfig::Kline(cfg))
                ]);

            content
        }
    };

    cfg_view_container(400, content)
}
