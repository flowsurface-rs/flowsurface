use data::chart::kline::FootprintStudy;
use iced::{
    Element, padding,
    widget::{button, column, container, horizontal_space, row, slider, text},
};

use crate::style::{self, Icon, icon_text};

#[derive(Debug, Clone)]
pub enum Message {
    CardToggled(FootprintStudy),
    StudyToggled(FootprintStudy, bool),
    ImbalancePctChanged(f32),
}

pub enum Action {
    None,
    ToggleStudy(FootprintStudy, bool),
    ConfigureStudy(FootprintStudy),
}

#[derive(Default)]
pub struct ChartStudy {
    expanded_card: Option<FootprintStudy>,
}

impl ChartStudy {
    pub fn new() -> Self {
        Self {
            expanded_card: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::CardToggled(study) => {
                let should_collapse = self
                    .expanded_card
                    .as_ref()
                    .is_some_and(|expanded| expanded.is_same_type(&study));

                if should_collapse {
                    self.expanded_card = None;
                } else {
                    self.expanded_card = Some(study);
                }
            }
            Message::StudyToggled(study, is_checked) => {
                return Action::ToggleStudy(study, is_checked);
            }
            Message::ImbalancePctChanged(value) => {
                let study = FootprintStudy::Imbalance {
                    threshold: value as i32,
                };
                return Action::ConfigureStudy(study);
            }
        }

        Action::None
    }

    pub fn view(&self, studies: &Vec<FootprintStudy>) -> Element<Message> {
        let mut content = column![].spacing(4);

        for available_study in FootprintStudy::ALL {
            let (is_selected, value) = {
                let mut is_selected = false;
                let mut threshold = None;

                for s in studies {
                    if s.is_same_type(&available_study) {
                        is_selected = true;
                        threshold = match s {
                            FootprintStudy::Imbalance { threshold } => Some(threshold),
                            _ => None,
                        };
                    }
                }
                (is_selected, threshold)
            };

            let checkbox = iced::widget::checkbox(available_study.to_string(), is_selected)
                .on_toggle(move |value| Message::StudyToggled(available_study, value));

            let mut checkbox_row = row![checkbox, horizontal_space()]
                .height(36)
                .align_y(iced::Alignment::Center)
                .padding(4)
                .spacing(4);

            match available_study {
                FootprintStudy::NPoC => {
                    let column = column![checkbox_row].padding(padding::left(4));
                    content = content.push(container(column).style(style::modal_container));
                }
                FootprintStudy::Imbalance { .. } => {
                    let is_expanded = self
                        .expanded_card
                        .as_ref()
                        .is_some_and(|expanded| expanded.is_same_type(&available_study));

                    if is_selected {
                        checkbox_row = checkbox_row.push(
                            button(icon_text(Icon::Cog, 12))
                                .on_press(Message::CardToggled(available_study))
                                .style(move |theme, status| {
                                    style::button::transparent(theme, status, is_expanded)
                                }),
                        )
                    }

                    let mut column = column![checkbox_row].padding(padding::left(4));

                    if is_expanded {
                        if let Some(threshold) = value {
                            let threshold_slider =
                                slider(100.0..=800.0, *threshold as f32, move |value| {
                                    Message::ImbalancePctChanged(value)
                                })
                                .step(25.0);

                            column = column.push(
                                column![
                                    text(format!("Ask:Bid threshold: {threshold}%")),
                                    threshold_slider
                                ]
                                .padding(8)
                                .spacing(4),
                            );
                        };
                    }

                    content = content.push(container(column).style(style::modal_container));
                }
            }
        }

        content.into()
    }
}
