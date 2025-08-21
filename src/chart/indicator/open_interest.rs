use std::collections::BTreeMap;

use iced::Element;
use iced::widget::{center, row, text};

use super::plot::{LinePlot, Tooltip, indicator_row};
use crate::chart::{Basis, Caches, Message, ViewState};
use data::util::format_with_commas;
use exchange::Timeframe;

pub fn indicator_elem<'a>(
    chart_state: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a BTreeMap<u64, f32>,
    earliest: u64,
    latest: u64,
) -> Element<'a, Message> {
    match chart_state.basis {
        Basis::Time(timeframe) => {
            let Some(ticker_info) = chart_state.ticker_info else {
                return row![].into();
            };
            let exchange = ticker_info.exchange();
            if exchange == exchange::adapter::Exchange::HyperliquidLinear {
                return center(text(format!(
                    "WIP: Open Interest is not available for {exchange}"
                )))
                .into();
            }
            if timeframe < Timeframe::M5 || timeframe == Timeframe::H2 || timeframe > Timeframe::H4
            {
                return center(text(format!(
                    "WIP: Open Interest is not available on {timeframe} timeframe"
                )))
                .into();
            }
            if latest < earliest {
                return row![].into();
            }
        }
        Basis::Tick(_) => {
            return center(text("WIP: Open Interest is not available for tick charts.")).into();
        }
    }

    let plot = LinePlot::new(
        |v: &f32| *v,
        |v: &f32, next: Option<&f32>| {
            let value_text = format!("Value: {}", format_with_commas(*v));
            let change_text = if let Some(n) = next {
                let d = *n - *v;
                let sign = if d >= 0.0 { "+" } else { "" };
                format!("Change: {}{}", sign, format_with_commas(d))
            } else {
                "Change: N/A".to_string()
            };
            let width = (value_text.len().max(change_text.len()) as f32) * 8.0;
            Tooltip {
                text: format!("{}\n{}", value_text, change_text),
                width,
                height: 28.0,
            }
        },
    )
    .stroke_width(1.0)
    .show_points(true)
    .point_radius_factor(0.2)
    .padding(0.08);

    indicator_row(chart_state, cache, plot, datapoints, earliest..=latest)
}
