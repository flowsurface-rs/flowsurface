use super::plot::{PlotTooltip, indicator_row};
use crate::chart::{
    Caches, Message, ViewState,
    indicator::plot::{BarClass, BarPlot, ReversedBTreeSeries},
};
use data::{chart::Basis, util::format_with_commas};

pub fn indicator_elem<'a>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a std::collections::BTreeMap<u64, (f32, f32)>,
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

    let bar_plot_kind = |&(buy, sell): &(f32, f32)| {
        if buy == -1.0 {
            BarClass::Single // bybit workaround: single color only
        } else {
            BarClass::Overlay {
                overlay: buy - sell,
            } // volume delta as overlay, sign determines up/down color
        }
    };

    let plot = BarPlot::new(
        |&(buy, sell): &(f32, f32)| {
            if buy == -1.0 { sell } else { buy + sell }
        },
        bar_plot_kind,
        tooltip,
    )
    .bar_width_factor(0.9)
    .padding(0.0);

    match chart_state.basis {
        Basis::Tick(_) => {
            let reversed = ReversedBTreeSeries::new(datapoints);
            indicator_row(chart_state, cache, plot, reversed, earliest..=latest)
        }
        Basis::Time(_) => {
            // Normal left-to-right
            indicator_row(chart_state, cache, plot, datapoints, earliest..=latest)
        }
    }
}
