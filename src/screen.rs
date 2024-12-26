use std::{collections::HashMap, fmt};

use iced::{
    widget::{button, column, container, pane_grid, text, Column}, window, Alignment, Element, Task, Theme
};
use iced_futures::MaybeSend;
use serde::{Deserialize, Serialize};

use crate::tooltip;

pub mod dashboard;
pub mod modal;

pub fn create_button<'a, M: Clone + 'a>(
    content: impl Into<Element<'a, M>>,
    message: M,
    tooltip_text: Option<&'a str>,
    tooltip_pos: tooltip::Position,
    style_fn: impl Fn(&Theme, button::Status) -> button::Style + 'static,
) -> Element<'a, M> {
    let btn = button(content)
        .style(style_fn)
        .on_press(message);
        
    if let Some(text) = tooltip_text {
        tooltip(btn, Some(text), tooltip_pos)
    } else {
        btn.into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum UserTimezone {
    #[default]
    Utc,
    Local,
}

impl fmt::Display for UserTimezone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UserTimezone::Utc => write!(f, "UTC"),
            UserTimezone::Local => {
                let local_offset = chrono::Local::now().offset().local_minus_utc();
                let hours = local_offset / 3600;
                let minutes = (local_offset % 3600) / 60;
                write!(f, "Local (UTC {hours:+03}:{minutes:02})")
            }
        }
    }
}

impl<'de> Deserialize<'de> for UserTimezone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let timezone_str = String::deserialize(deserializer)?;
        match timezone_str.to_lowercase().as_str() {
            "utc" => Ok(UserTimezone::Utc),
            "local" => Ok(UserTimezone::Local),
            _ => Err(serde::de::Error::custom("Invalid UserTimezone")),
        }
    }
}

impl Serialize for UserTimezone {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            UserTimezone::Utc => serializer.serialize_str("UTC"),
            UserTimezone::Local => serializer.serialize_str("Local"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InfoType {
    FetchingKlines,
    FetchingTrades(usize),
    FetchingOI,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    Error(String),
    Info(InfoType),
    Warn(String),
}

pub fn handle_error<M, F>(err: &str, report: &str, message: F) -> Task<M> 
where
    F: Fn(Notification) -> M + Send + 'static,
    M: MaybeSend + 'static,
{
    log::error!("{err}: {report}");

    Task::done(message(
        Notification::Error(report.to_string())
    ))
}

#[derive(Default)]
pub struct NotificationManager {
    notifications: HashMap<window::Id, HashMap<pane_grid::Pane, Vec<Notification>>>,
    global_notifications: Vec<Notification>,
}

#[allow(dead_code)]
impl NotificationManager {
    pub fn new() -> Self {
        Self {
            notifications: HashMap::new(),
            global_notifications: Vec::new(),
        }
    }

    /// Helper method to get or create window entry
    fn get_or_create_window(
        &mut self,
        window: window::Id,
    ) -> &mut HashMap<pane_grid::Pane, Vec<Notification>> {
        self.notifications.entry(window).or_default()
    }

    /// Helper method to get or create notification list
    fn get_or_create_notifications(
        &mut self,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> &mut Vec<Notification> {
        let window_map = self.get_or_create_window(window);
        window_map.entry(pane).or_default()
    }

    /// Add a notification for a specific pane in a window
    pub fn push(&mut self, window: window::Id, pane: pane_grid::Pane, notification: Notification) {
        self.get_or_create_notifications(window, pane)
            .push(notification);
    }

    pub fn increment_fetching_trades(
        &mut self,
        window: window::Id,
        pane: &pane_grid::Pane,
        increment_by: usize,
    ) {
        if let Some(window_map) = self.notifications.get_mut(&window) {
            if let Some(notification_list) = window_map.get_mut(pane) {
                let found = notification_list.iter_mut().any(|notification| {
                    if let Notification::Info(InfoType::FetchingTrades(count)) = notification {
                        *count += increment_by;
                        return true;
                    }
                    false
                });

                if !found {
                    notification_list.push(Notification::Info(InfoType::FetchingTrades(increment_by)));
                }
            } else {
                window_map.insert(*pane, vec![Notification::Info(InfoType::FetchingTrades(increment_by))]);
            }
        } else {
            let mut pane_map = HashMap::new();
            pane_map.insert(*pane, vec![Notification::Info(InfoType::FetchingTrades(increment_by))]);
            self.notifications.insert(window, pane_map);
        }
    }

    pub fn find_and_remove(&mut self, window: window::Id, pane: pane_grid::Pane, notification: Notification) {
        if let Some(window_map) = self.notifications.get_mut(&window) {
            if let Some(notification_list) = window_map.get_mut(&pane) {
                notification_list.retain(|n| n != &notification);
            }
        }
    }

    /// Remove notifications of a specific type for a pane in a window
    pub fn remove_info_type(
        &mut self,
        window: window::Id,
        pane: &pane_grid::Pane,
        info_type: &InfoType,
    ) {
        if let Some(window_map) = self.notifications.get_mut(&window) {
            if let Some(notification_list) = window_map.get_mut(pane) {
                notification_list.retain(|notification| {
                    !matches!(notification,
                        Notification::Info(current_type)
                        if std::mem::discriminant(current_type) == std::mem::discriminant(info_type)
                    )
                });
            }
        }
    }

    /// Get notifications for a specific pane in a window
    pub fn get(&self, window: &window::Id, pane: &pane_grid::Pane) -> Option<&Vec<Notification>> {
        self.notifications
            .get(window)
            .and_then(|window_map| window_map.get(pane))
    }

    /// Get mutable notifications for a specific pane in a window
    pub fn get_mut(
        &mut self,
        window: &window::Id,
        pane: &pane_grid::Pane,
    ) -> Option<&mut Vec<Notification>> {
        self.notifications
            .get_mut(window)
            .and_then(|window_map| window_map.get_mut(pane))
    }

    /// Handle error notifications with special fetch error logic
    pub fn handle_error(&mut self, window: window::Id, pane: pane_grid::Pane, err: DashboardError) {
        log::error!("{:?}", err);

        let notification_list = self.get_or_create_notifications(window, pane);
        notification_list.push(Notification::Error(err.to_string()));

        // If it's a fetch error, remove any pending fetch notifications
        if matches!(err, DashboardError::Fetch(_)) {
            notification_list.retain(|notification| {
                !matches!(
                    notification,
                    Notification::Info(InfoType::FetchingKlines)
                )
            });
        }
    }

    /// Remove the last notification for a specific pane in a window
    pub fn remove_last(&mut self, window: &window::Id, pane: &pane_grid::Pane) {
        if let Some(window_map) = self.notifications.get_mut(window) {
            if let Some(notification_list) = window_map.get_mut(pane) {
                notification_list.pop();
            }
        }
    }

    /// Clear all notifications for a specific pane in a window
    pub fn clear(&mut self, window: &window::Id, pane: &pane_grid::Pane) {
        if let Some(window_map) = self.notifications.get_mut(window) {
            if let Some(notification_list) = window_map.get_mut(pane) {
                notification_list.clear();
            }
        }
    }

    /// Clear all notifications for a window
    pub fn clear_window(&mut self, window: &window::Id) {
        self.notifications.remove(window);
    }

    /// Check if notifications exist for a specific pane in a window
    pub fn has_notification(&self, window: &window::Id, pane: &pane_grid::Pane) -> bool {
        self.notifications
            .get(window)
            .and_then(|window_map| window_map.get(pane))
            .map_or(false, |notifications| !notifications.is_empty())
    }

    /// Get all notifications for a window
    pub fn get_window_notifications(
        &self,
        window: &window::Id,
    ) -> Option<&HashMap<pane_grid::Pane, Vec<Notification>>> {
        self.notifications.get(window)
    }
}

fn create_notis_column<'a, M: 'a + Clone>(
    notifications: &'a [Notification],
    close_message: M,
) -> Column<'a, M> {
    let mut notifications_column = column![].align_x(Alignment::End).spacing(6);

    for notification in notifications.iter().rev().take(5) {
        let notification_str = match notification {
            Notification::Error(error) => error.to_string(),
            Notification::Warn(warn) => warn.to_string(),
            Notification::Info(info) => match info {
                InfoType::FetchingKlines => "Fetching klines...".to_string(),
                InfoType::FetchingTrades(total_fetched) => format!(
                    "Fetching trades...\n({} fetched)",
                    total_fetched
                ),
                InfoType::FetchingOI => "Fetching open interest...".to_string(),
            },
        };

        notifications_column = notifications_column
            .push(
                button(container(text(notification_str)).padding(6))
                    .on_press(close_message.clone()),
            )
            .padding(12);
    }

    notifications_column
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum DashboardError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Pane set error: {0}")]
    PaneSet(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}
