use crate::chart::{heatmap::HeatmapChart, kline::KlineChart};
use crate::modal::layout_manager::LayoutManager;
use crate::screen::dashboard::{Dashboard, pane, panel::timeandsales::TimeAndSales};
use data::{
    UserTimezone,
    chart::Basis,
    layout::{WindowSpec, pane::Axis},
};
use exchange::{TickMultiplier, Ticker, Timeframe, adapter::Exchange};

use iced::widget::pane_grid::{self, Configuration};
use std::{collections::HashMap, vec};
use uuid::Uuid;

#[derive(Eq, Hash, Debug, Clone, PartialEq)]
pub struct Layout {
    pub id: Uuid,
    pub name: String,
}

pub struct SavedState {
    pub layout_manager: LayoutManager,
    pub main_window: Option<WindowSpec>,
    pub favorited_tickers: Vec<(Exchange, Ticker)>,
    pub scale_factor: data::ScaleFactor,
    pub timezone: data::UserTimezone,
    pub sidebar: data::Sidebar,
    pub theme: data::Theme,
    pub custom_theme: Option<data::Theme>,
    pub audio_cfg: data::AudioStream,
    pub preferred_currency: exchange::PreferredCurrency,
}

impl SavedState {
    pub fn window(&self) -> (iced::window::Position, iced::Size) {
        let position = self.main_window.map(|w| w.position()).map_or(
            iced::window::Position::Centered,
            iced::window::Position::Specific,
        );
        let size = self
            .main_window
            .map_or_else(crate::window::default_size, |w| w.size());

        (position, size)
    }
}

impl Default for SavedState {
    fn default() -> Self {
        SavedState {
            layout_manager: LayoutManager::new(),
            main_window: None,
            favorited_tickers: Vec::new(),
            scale_factor: data::ScaleFactor::default(),
            timezone: UserTimezone::default(),
            sidebar: data::Sidebar::default(),
            theme: data::Theme::default(),
            custom_theme: None,
            audio_cfg: data::AudioStream::default(),
            preferred_currency: exchange::PreferredCurrency::Base,
        }
    }
}

impl From<&Dashboard> for data::Dashboard {
    fn from(dashboard: &Dashboard) -> Self {
        use pane_grid::Node;

        fn from_layout(panes: &pane_grid::State<pane::State>, node: pane_grid::Node) -> data::Pane {
            match node {
                Node::Split {
                    axis, ratio, a, b, ..
                } => data::Pane::Split {
                    axis: match axis {
                        pane_grid::Axis::Horizontal => Axis::Horizontal,
                        pane_grid::Axis::Vertical => Axis::Vertical,
                    },
                    ratio,
                    a: Box::new(from_layout(panes, *a)),
                    b: Box::new(from_layout(panes, *b)),
                },
                Node::Pane(pane) => panes
                    .get(pane)
                    .map_or(data::Pane::default(), data::Pane::from),
            }
        }

        let main_window_layout = dashboard.panes.layout().clone();

        let popouts_layout: Vec<(data::Pane, WindowSpec)> = dashboard
            .popout
            .iter()
            .map(|(_, (pane, spec))| (from_layout(pane, pane.layout().clone()), *spec))
            .collect();

        data::Dashboard {
            pane: from_layout(&dashboard.panes, main_window_layout),
            popout: {
                popouts_layout
                    .iter()
                    .map(|(pane, window_spec)| (pane.clone(), *window_spec))
                    .collect()
            },
        }
    }
}

impl From<&pane::State> for data::Pane {
    fn from(pane: &pane::State) -> Self {
        let streams = pane.streams.clone();

        match &pane.content {
            pane::Content::Starter => data::Pane::Starter {
                link_group: pane.link_group,
            },
            pane::Content::Heatmap(chart, indicators) => data::Pane::HeatmapChart {
                layout: chart.chart_layout(),
                stream_type: streams,
                settings: pane.settings,
                indicators: indicators.clone(),
                studies: chart.studies.clone(),
                link_group: pane.link_group,
            },
            pane::Content::Kline(chart, indicators) => data::Pane::KlineChart {
                layout: chart.chart_layout(),
                kind: chart.kind().clone(),
                stream_type: streams,
                settings: pane.settings,
                indicators: indicators.clone(),
                link_group: pane.link_group,
            },
            pane::Content::TimeAndSales(_) => data::Pane::TimeAndSales {
                stream_type: streams,
                settings: pane.settings,
                link_group: pane.link_group,
            },
        }
    }
}

pub fn configuration(pane: data::Pane) -> Configuration<pane::State> {
    match pane {
        data::Pane::Split { axis, ratio, a, b } => Configuration::Split {
            axis: match axis {
                Axis::Horizontal => pane_grid::Axis::Horizontal,
                Axis::Vertical => pane_grid::Axis::Vertical,
            },
            ratio,
            a: Box::new(configuration(*a)),
            b: Box::new(configuration(*b)),
        },
        data::Pane::Starter { link_group } => Configuration::Pane(pane::State::from_config(
            pane::Content::Starter,
            vec![],
            data::layout::pane::Settings::default(),
            link_group,
        )),
        data::Pane::HeatmapChart {
            layout,
            studies,
            stream_type,
            settings,
            indicators,
            link_group,
        } => {
            if let Some(ticker_info) = settings.ticker_info {
                let tick_size = settings
                    .tick_multiply
                    .unwrap_or(TickMultiplier(10))
                    .multiply_with_min_tick_size(ticker_info);

                let config = settings.visual_config.and_then(|cfg| cfg.heatmap());
                let basis = settings
                    .selected_basis
                    .unwrap_or(Basis::default_heatmap_time(Some(ticker_info)));

                Configuration::Pane(pane::State::from_config(
                    pane::Content::Heatmap(
                        HeatmapChart::new(
                            layout,
                            basis,
                            tick_size,
                            &indicators,
                            settings.ticker_info,
                            config,
                            studies,
                        ),
                        indicators,
                    ),
                    stream_type,
                    settings,
                    link_group,
                ))
            } else {
                log::info!("Skipping a HeatmapChart initialization due to missing ticker info");
                Configuration::Pane(pane::State::new())
            }
        }
        data::Pane::KlineChart {
            layout,
            kind,
            stream_type,
            settings,
            indicators,
            link_group,
        } => match kind {
            data::chart::KlineChartKind::Footprint { .. } => {
                if let Some(ticker_info) = settings.ticker_info {
                    let tick_size = settings
                        .tick_multiply
                        .unwrap_or(TickMultiplier(50))
                        .multiply_with_min_tick_size(ticker_info);
                    let basis = settings.selected_basis.unwrap_or(Timeframe::M5.into());

                    Configuration::Pane(pane::State::from_config(
                        pane::Content::Kline(
                            KlineChart::new(
                                layout,
                                basis,
                                tick_size,
                                &[],
                                vec![],
                                &indicators,
                                settings.ticker_info,
                                &kind,
                            ),
                            indicators,
                        ),
                        stream_type,
                        settings,
                        link_group,
                    ))
                } else {
                    log::info!(
                        "Skipping a FootprintChart initialization due to missing ticker info"
                    );
                    Configuration::Pane(pane::State::new())
                }
            }
            data::chart::KlineChartKind::Candles => {
                if let Some(ticker_info) = settings.ticker_info {
                    let basis = settings.selected_basis.unwrap_or(Timeframe::M15.into());

                    let tick_size = settings
                        .tick_multiply
                        .unwrap_or(TickMultiplier(1))
                        .multiply_with_min_tick_size(ticker_info);

                    Configuration::Pane(pane::State::from_config(
                        pane::Content::Kline(
                            KlineChart::new(
                                layout,
                                basis,
                                tick_size,
                                &[],
                                vec![],
                                &indicators,
                                settings.ticker_info,
                                &kind,
                            ),
                            indicators,
                        ),
                        stream_type,
                        settings,
                        link_group,
                    ))
                } else {
                    log::info!(
                        "Skipping a CandlestickChart initialization due to missing ticker info"
                    );
                    Configuration::Pane(pane::State::new())
                }
            }
        },
        data::Pane::TimeAndSales {
            stream_type,
            settings,
            link_group,
        } => {
            if settings.ticker_info.is_none() {
                log::info!("Skipping a TimeAndSales initialization due to missing ticker info");
                return Configuration::Pane(pane::State::new());
            }

            let config = settings.visual_config.and_then(|cfg| cfg.time_and_sales());

            Configuration::Pane(pane::State::from_config(
                pane::Content::TimeAndSales(TimeAndSales::new(config, settings.ticker_info)),
                stream_type,
                settings,
                link_group,
            ))
        }
    }
}

pub fn load_saved_state() -> SavedState {
    match data::read_from_file(data::SAVED_STATE_PATH) {
        Ok(state) => {
            let mut de_layouts = vec![];

            for layout in &state.layout_manager.layouts {
                let mut popout_windows = Vec::new();

                for (pane, window_spec) in &layout.dashboard.popout {
                    let configuration = configuration(pane.clone());
                    popout_windows.push((configuration, *window_spec));
                }

                let layout_id = Uuid::new_v4();

                let dashboard = Dashboard::from_config(
                    configuration(layout.dashboard.pane.clone()),
                    popout_windows,
                    layout_id,
                );

                de_layouts.push((layout.name.clone(), layout_id, dashboard));
            }

            let layout_manager: LayoutManager = {
                let mut layouts = HashMap::new();

                let active_layout = Layout {
                    id: Uuid::new_v4(),
                    name: state.layout_manager.active_layout.clone(),
                };

                let mut layout_order = vec![];

                for (name, layout_id, dashboard) in de_layouts {
                    let layout = Layout {
                        id: {
                            if name == active_layout.name {
                                active_layout.id
                            } else {
                                layout_id
                            }
                        },
                        name,
                    };

                    layout_order.push(layout.id);
                    layouts.insert(layout.id, (layout.clone(), dashboard));
                }

                LayoutManager::from_config(layout_order, layouts, active_layout)
            };

            exchange::fetcher::toggle_trade_fetch(state.trade_fetch_enabled);

            exchange::SIZE_IN_QUOTE_CURRENCY
                .set(state.size_in_quote_currency)
                .expect("Failed to set SIZE_IN_QUOTE_CURRENCY");

            SavedState {
                theme: state.selected_theme,
                custom_theme: state.custom_theme,
                layout_manager,
                favorited_tickers: state.favorited_tickers,
                main_window: state.main_window,
                timezone: state.timezone,
                sidebar: state.sidebar,
                scale_factor: state.scale_factor,
                audio_cfg: state.audio_cfg,
                preferred_currency: if state.size_in_quote_currency {
                    exchange::PreferredCurrency::Quote
                } else {
                    exchange::PreferredCurrency::Base
                },
            }
        }
        Err(e) => {
            log::error!(
                "Failed to load/find layout state: {}. Starting with a new layout.",
                e
            );

            SavedState::default()
        }
    }
}
