use super::plot::PlotTooltip;
use crate::chart::{
    Caches, Message, ViewState,
    indicator::{
        SeriesMap,
        plot::bar::{BarClass, BarPlot},
    },
};
use data::util::format_with_commas;

pub fn indicator_elem<'a>(
    main_chart: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a SeriesMap<(f32, f32)>,
    earliest: u64,
    latest: u64,
) -> iced::Element<'a, Message> {
    let tooltip = |&(buy, sell): &(f32, f32), _next: Option<&(f32, f32)>| {
        if buy == -1.0 {
            PlotTooltip::new(format!("Volume: {}", format_with_commas(sell)))
        } else {
            let buy_t = format!("Buy Volume: {}", format_with_commas(buy));
            let sell_t = format!("Sell Volume: {}", format_with_commas(sell));
            PlotTooltip::new(format!("{buy_t}\n{sell_t}"))
        }
    };

    let bar_kind = |&(buy, sell): &(f32, f32)| {
        if buy == -1.0 {
            BarClass::Single // bybit workaround: single color only
        } else {
            BarClass::Overlay {
                overlay: buy - sell,
            } // volume delta as overlay, sign determines up/down color
        }
    };

    let y_value = |&(buy, sell): &(f32, f32)| {
        if buy == -1.0 { sell } else { buy + sell }
    };

    let plot = BarPlot::new(y_value, bar_kind, tooltip).bar_width_factor(0.9);

    super::indicator_row(main_chart, cache, plot, datapoints, earliest..=latest)
}
