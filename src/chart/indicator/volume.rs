use super::plot::{Tooltip, indicator_row};
use crate::chart::{
    Caches, Message, ViewState,
    indicator::plot::{BarClass, BarPlot},
};
use data::util::format_with_commas;

use iced::Element;
use std::collections::BTreeMap;

pub fn indicator_elem<'a>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a BTreeMap<u64, (f32, f32)>,
    earliest: u64,
    latest: u64,
) -> Element<'a, Message> {
    let plot = BarPlot::new(
        // total (main height)
        |&(buy, sell): &(f32, f32)| {
            if buy == -1.0 { sell } else { buy + sell }
        },
        |&(buy, sell): &(f32, f32)| {
            if buy == -1.0 {
                BarClass::Single // bybit workaround: single color only
            } else {
                BarClass::Overlay {
                    overlay: buy - sell,
                } // sign determines up/down color
            }
        },
        // tooltip
        |&(buy, sell): &(f32, f32), _next: Option<&(f32, f32)>| {
            if buy == -1.0 {
                let text = format!("Volume: {}", format_with_commas(sell));
                Tooltip {
                    text,
                    width: (format_with_commas(sell).len() as f32 + 8.0) * 8.0,
                    height: 14.0,
                }
            } else {
                let buy_t = format!("Buy Volume: {}\n", format_with_commas(buy));
                let sell_t = format!("Sell Volume: {}", format_with_commas(sell));
                let width = (buy_t.len().max(sell_t.len()) as f32) * 8.0;
                Tooltip {
                    text: format!("{}{}", buy_t, sell_t),
                    width,
                    height: 28.0,
                }
            }
        },
    )
    .bar_width_factor(0.9)
    .padding(0.0);

    indicator_row(chart_state, cache, plot, datapoints, earliest..=latest)
}
