use super::{Message, sync_all_button};
use crate::theme::ThemeAccents;
use crate::widget::study_configurator as study;
use crate::widget::{
    classic_slider_row,
    inspector::{InspectorContent, InspectorSection},
};
use data::chart::heatmap;
use data::chart::heatmap::HeatmapStudy;
use data::layout::pane::VisualConfig;
use data::util::format_with_commas;
use iced::widget::{checkbox, column, row, space, text};

pub fn heatmap_shader_cfg_view<'a>(
    cfg: heatmap::Config,
    study_config: &'a study::Configurator<HeatmapStudy>,
    studies: &'a [HeatmapStudy],
    basis: data::chart::Basis,
    theme_accents: ThemeAccents,
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

        let mut col = column![dyn_checkbox].spacing(8);
        if let Some(slider) = circle_scaling_slider {
            col = col.push(slider);
        }
        col
    };

    let study_cfg = study_config
        .view(studies, basis, None, None, theme_accents)
        .map(move |msg| Message::Study(study::StudyMessage::Heatmap(msg)));

    let mut content = InspectorContent::new(vec![
        InspectorSection::titled("Size filter", size_filters),
        InspectorSection::titled("Trade visualization", trade_viz_column),
        InspectorSection::titled("Studies", study_cfg),
    ]);
    content.set_footer(row![
        space::horizontal(),
        sync_all_button(VisualConfig::Heatmap(cfg))
    ]);
    content
}
