use super::plot::{PlotTooltip, line::LinePlot};
use crate::chart::{Basis, Caches, Message, ViewState, indicator::SeriesMap};
use data::util::format_with_commas;
use exchange::Timeframe;

use iced::widget::{center, row, text};

pub fn indicator_elem<'a>(
    main_chart: &'a ViewState,
    cache: &'a Caches,
    datapoints: &'a SeriesMap<f32>,
    earliest: u64,
    latest: u64,
) -> iced::Element<'a, Message> {
    match main_chart.basis {
        Basis::Time(timeframe) => {
            if latest < earliest {
                return row![].into();
            }

            let exchange = match main_chart.ticker_info.as_ref() {
                Some(info) => info.exchange(),
                None => return row![].into(),
            };
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
        }
        Basis::Tick(_) => {
            return center(text("WIP: Open Interest is not available for tick charts.")).into();
        }
    }

    let tooltip = |value: &f32, next: Option<&f32>| {
        let value_text = format!("Open Interest: {}", format_with_commas(*value));
        let change_text = if let Some(next_value) = next {
            let delta = next_value - *value;
            let sign = if delta >= 0.0 { "+" } else { "" };
            format!("Change: {}{}", sign, format_with_commas(delta))
        } else {
            "Change: N/A".to_string()
        };
        PlotTooltip::new(format!("{value_text}\n{change_text}"))
    };

    let y_value = |v: &f32| *v;

    let plot = LinePlot::new(y_value, tooltip)
        .stroke_width(1.0)
        .show_points(true)
        .point_radius_factor(0.2)
        .padding(0.08);

    super::indicator_row(main_chart, cache, plot, datapoints, earliest..=latest)
}
