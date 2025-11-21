use crate::modal::layout_manager::LayoutManager;
use crate::screen::dashboard::{Dashboard, pane};
use data::{
    UserTimezone,
    layout::{WindowSpec, pane::Axis},
};

use iced::widget::pane_grid::{self, Configuration};
use std::vec;
use uuid::Uuid;

pub struct Layout {
    pub id: LayoutId,
    pub dashboard: Dashboard,
}

#[derive(Debug, Clone)]
pub struct LayoutId {
    pub unique: Uuid,
    pub name: String,
}

pub struct SavedState {
    pub layout_manager: LayoutManager,
    pub main_window: Option<WindowSpec>,
    pub scale_factor: data::ScaleFactor,
    pub timezone: data::UserTimezone,
    pub sidebar: data::Sidebar,
    pub theme: data::Theme,
    pub custom_theme: Option<data::Theme>,
    pub audio_cfg: data::AudioStream,
    pub volume_size_unit: exchange::SizeUnit,
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
            scale_factor: data::ScaleFactor::default(),
            timezone: UserTimezone::default(),
            sidebar: data::Sidebar::default(),
            theme: data::Theme::default(),
            custom_theme: None,
            audio_cfg: data::AudioStream::default(),
            volume_size_unit: exchange::SizeUnit::Base,
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
        let streams = pane.streams.clone().into_waiting();

        match &pane.content {
            pane::Content::Starter => data::Pane::Starter {
                link_group: pane.link_group,
            },
            pane::Content::Heatmap {
                chart,
                indicators,
                studies,
                layout,
                ..
            } => data::Pane::HeatmapChart {
                layout: chart.as_ref().map_or(layout.clone(), |c| c.chart_layout()),
                stream_type: streams,
                settings: pane.settings.clone(),
                indicators: indicators.clone(),
                studies: chart
                    .as_ref()
                    .map_or(studies.clone(), |c| c.studies.clone()),
                link_group: pane.link_group,
            },
            pane::Content::Kline {
                chart,
                indicators,
                kind,
                layout,
                ..
            } => data::Pane::KlineChart {
                layout: chart.as_ref().map_or(layout.clone(), |c| c.chart_layout()),
                kind: kind.clone(),
                stream_type: streams,
                settings: pane.settings.clone(),
                indicators: indicators.clone(),
                link_group: pane.link_group,
            },
            pane::Content::TimeAndSales(_) => data::Pane::TimeAndSales {
                stream_type: streams,
                settings: pane.settings.clone(),
                link_group: pane.link_group,
            },
            pane::Content::Ladder(_) => data::Pane::Ladder {
                stream_type: streams,
                settings: pane.settings.clone(),
                link_group: pane.link_group,
            },
            pane::Content::Comparison(chart) => {
                let settings = data::layout::pane::Settings {
                    visual_config: chart.as_ref().map(|c| {
                        data::layout::pane::VisualConfig::Comparison(c.serializable_config())
                    }),
                    ..pane.settings.clone()
                };

                data::Pane::ComparisonChart {
                    stream_type: streams,
                    settings,
                    link_group: pane.link_group,
                }
            }
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
            let content = pane::Content::Heatmap {
                chart: None,
                indicators: indicators.clone(),
                layout,
                studies,
            };

            Configuration::Pane(pane::State::from_config(
                content,
                stream_type,
                settings,
                link_group,
            ))
        }
        data::Pane::KlineChart {
            layout,
            kind,
            stream_type,
            settings,
            indicators,
            link_group,
        } => {
            let content = pane::Content::Kline {
                chart: None,
                indicators: indicators.clone(),
                layout,
                kind,
            };

            Configuration::Pane(pane::State::from_config(
                content,
                stream_type,
                settings,
                link_group,
            ))
        }
        data::Pane::ComparisonChart {
            stream_type,
            settings,
            link_group,
        } => {
            let content = pane::Content::Comparison(None);

            Configuration::Pane(pane::State::from_config(
                content,
                stream_type,
                settings,
                link_group,
            ))
        }
        data::Pane::TimeAndSales {
            stream_type,
            settings,
            link_group,
        } => {
            let content = pane::Content::TimeAndSales(None);

            Configuration::Pane(pane::State::from_config(
                content,
                stream_type,
                settings,
                link_group,
            ))
        }
        data::Pane::Ladder {
            stream_type,
            settings,
            link_group,
        } => {
            let content = pane::Content::Ladder(None);

            Configuration::Pane(pane::State::from_config(
                content,
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

            let layout_manager = {
                let mut layouts = Vec::with_capacity(de_layouts.len());

                for (name, layout_id, dashboard) in de_layouts {
                    let id = LayoutId {
                        unique: layout_id,
                        name,
                    };
                    layouts.push(Layout { id, dashboard });
                }

                let active_layout =
                    state
                        .layout_manager
                        .active_layout
                        .as_ref()
                        .and_then(|target_name| {
                            layouts
                                .iter()
                                .find(|layout| layout.id.name == *target_name)
                                .map(|layout| layout.id.clone())
                        });

                LayoutManager::from_config(layouts, active_layout)
            };

            exchange::fetcher::toggle_trade_fetch(state.trade_fetch_enabled);
            exchange::set_preferred_currency(state.size_in_quote_ccy);

            SavedState {
                theme: state.selected_theme,
                custom_theme: state.custom_theme,
                layout_manager,
                main_window: state.main_window,
                timezone: state.timezone,
                sidebar: state.sidebar,
                scale_factor: state.scale_factor,
                audio_cfg: state.audio_cfg,
                volume_size_unit: state.size_in_quote_ccy,
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
