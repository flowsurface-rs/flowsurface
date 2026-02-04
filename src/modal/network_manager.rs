use crate::{
    style::{self, icon_text},
    widget::tooltip,
};
use exchange::proxy::{Proxy, ProxyAuth, ProxyScheme};

use iced::{
    Element, Theme,
    widget::{button, checkbox, column, container, pick_list, row, text, text_input},
};

pub enum Action {
    ApplyProxy,
    Exit,
}

#[derive(Debug, Clone)]
pub enum Message {
    GoBack,
    ToggleShowPassword(bool),
    SchemeChanged(ProxyScheme),
    HostChanged(String),
    PortChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    Apply,
    RequestClear,
    RequestApply,
    Cancel,
    Clear,
}

#[derive(Debug, Clone)]
pub struct NetworkManager {
    /// Saved/selected config (takes effect after restart).
    /// This is the "next run" proxy config, persisted by the parent on Action::ApplyProxy.
    pub proxy_url: Option<String>,

    /// Effective proxy at runtime (current process).
    effective_proxy_cfg: Option<Proxy>,

    error: Option<String>,

    scheme: ProxyScheme,
    host: String,
    port: String,
    username: String,
    password: String,

    confirming_clear: bool,
    confirming_apply: bool,
    hide_password: bool,
}

impl NetworkManager {
    pub fn new(proxy_cfg: Option<exchange::proxy::Proxy>) -> Self {
        let (proxy_url, scheme, host, port, username, password) =
            if let Some(cfg) = proxy_cfg.clone() {
                let url = exchange::proxy::Proxy::to_url_string(&cfg);
                (
                    Some(url),
                    cfg.scheme,
                    cfg.host,
                    cfg.port.to_string(),
                    cfg.auth
                        .as_ref()
                        .map(|a| a.username.clone())
                        .unwrap_or_default(),
                    cfg.auth
                        .as_ref()
                        .map(|a| a.password.clone())
                        .unwrap_or_default(),
                )
            } else {
                (
                    None,
                    ProxyScheme::Http,
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            };

        Self {
            proxy_url,
            effective_proxy_cfg: exchange::proxy::runtime_proxy_cfg(),
            error: None,
            hide_password: true,
            scheme,
            host,
            port,
            username,
            password,
            confirming_clear: false,
            confirming_apply: false,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::ToggleShowPassword(v) => {
                self.hide_password = v;
                self.reset_transient();
            }
            Message::SchemeChanged(v) => {
                self.scheme = v;
                self.reset_transient();
            }
            Message::HostChanged(v) => {
                self.host = v;
                self.reset_transient();
            }
            Message::PortChanged(v) => {
                self.port = v;
                self.reset_transient();
            }
            Message::UsernameChanged(v) => {
                self.username = v;
                self.reset_transient();
            }
            Message::PasswordChanged(v) => {
                self.password = v;
                self.reset_transient();
            }
            Message::RequestApply => {
                self.confirming_clear = false;

                match self.build_proxy_cfg_from_parts() {
                    Ok(draft_cfg) => {
                        let current_cfg = self.proxy_cfg();
                        if draft_cfg == current_cfg {
                            self.confirming_apply = false;
                            self.error = None;
                        } else {
                            self.confirming_apply = true;
                            self.error = None;
                        }
                    }
                    Err(e) => {
                        self.confirming_apply = false;
                        self.error = Some(e);
                    }
                }
            }
            Message::Apply => {
                self.confirming_clear = false;
                self.confirming_apply = false;

                match self.build_proxy_cfg_from_parts() {
                    Ok(Some(cfg)) => match cfg.try_to_url_string() {
                        Ok(url) => {
                            self.proxy_url = Some(url);
                            self.error = None;
                            return Some(Action::ApplyProxy);
                        }
                        Err(e) => {
                            self.error = Some(e);
                        }
                    },
                    Ok(None) => {
                        self.proxy_url = None;
                        self.error = None;
                        return Some(Action::ApplyProxy);
                    }
                    Err(e) => {
                        self.error = Some(e);
                    }
                }
            }
            Message::RequestClear => {
                self.confirming_clear = true;
                self.confirming_apply = false;
                self.error = None;
            }
            Message::Clear => {
                self.reset_transient();

                self.proxy_url = None;

                self.host.clear();
                self.port.clear();
                self.username.clear();
                self.password.clear();
                self.scheme = ProxyScheme::Http;

                return Some(Action::ApplyProxy);
            }
            Message::Cancel => {
                self.confirming_clear = false;
                self.confirming_apply = false;
            }
            Message::GoBack => {
                self.confirming_clear = false;
                self.confirming_apply = false;
                return Some(Action::Exit);
            }
        }
        None
    }

    pub fn view(&self) -> Element<'_, Message> {
        let modal_header = row![
            button(style::icon_text(style::Icon::Return, 11)).on_press(Message::GoBack),
            iced::widget::space::horizontal(),
        ];

        let proxy_settings = {
            let saved_cfg = self.proxy_cfg();
            let is_pending = { saved_cfg != self.effective_proxy_cfg };

            let applied_proxy = {
                let effective = self
                    .effective_proxy_cfg
                    .as_ref()
                    .map(|c| c.to_ui_string())
                    .unwrap_or_else(|| "None (direct connection)".to_string());

                let pending_url = if is_pending {
                    Some(
                        saved_cfg
                            .as_ref()
                            .map(|c| c.to_ui_string())
                            .unwrap_or_else(|| "None (direct connection)".to_string()),
                    )
                } else {
                    None
                };

                let mut lines = column![
                    row![text("Effective:").size(11), text(effective).size(12),]
                        .spacing(4)
                        .align_y(iced::Alignment::Center)
                        .width(iced::Length::Fill),
                ]
                .spacing(4);

                if let Some(pending) = pending_url {
                    lines = lines.push(
                        row![text("Pending:").size(11), text(pending).size(12),]
                            .spacing(4)
                            .align_y(iced::Alignment::Center)
                            .width(iced::Length::Fill),
                    );
                }
                lines
            };

            let scheme = {
                row![
                    iced::widget::space::horizontal(),
                    text("Scheme:"),
                    pick_list(ProxyScheme::ALL, Some(self.scheme), Message::SchemeChanged)
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center)
            };

            let host = row![
                iced::widget::space::horizontal(),
                text("Host:"),
                text_input("e.g. 127.0.0.1", &self.host)
                    .on_input(Message::HostChanged)
                    .width(200)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            let port = row![
                iced::widget::space::horizontal(),
                text("Port:"),
                text_input("e.g. 8080", &self.port)
                    .on_input(Message::PortChanged)
                    .width(200)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            let username = row![
                iced::widget::space::horizontal(),
                text("Username:"),
                text_input("(optional)", &self.username)
                    .on_input(Message::UsernameChanged)
                    .width(200)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);

            let password = row![
                iced::widget::space::horizontal(),
                text("Password:"),
                text_input("(optional)", &self.password)
                    .on_input(Message::PasswordChanged)
                    .width(180)
                    .secure(self.hide_password),
                tooltip(
                    checkbox(self.hide_password).on_toggle(Message::ToggleShowPassword),
                    Some("Hide password"),
                    iced::widget::tooltip::Position::Top,
                ),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center);

            let confirm_btn = |msg: Message| {
                create_icon_button(
                    style::Icon::Checkmark,
                    12,
                    |theme, status| style::button::confirm(theme, *status, true),
                    Some(msg),
                )
            };
            let cancel_btn = || {
                create_icon_button(
                    style::Icon::Close,
                    12,
                    |theme, status| style::button::cancel(theme, *status, true),
                    Some(Message::Cancel),
                )
            };

            let buttons = if self.confirming_clear {
                row![
                    iced::widget::space::horizontal(),
                    container(
                        row![
                            text("Unset proxy and clear inputs?"),
                            confirm_btn(Message::Clear),
                            cancel_btn()
                        ]
                        .padding(iced::padding::left(8))
                        .align_y(iced::Alignment::Center)
                    )
                    .style(style::modal_container)
                ]
                .align_y(iced::Alignment::Center)
            } else if self.confirming_apply {
                row![
                    iced::widget::space::horizontal(),
                    container(
                        row![
                            text("Changes will take effect after a restart"),
                            confirm_btn(Message::Apply),
                            cancel_btn()
                        ]
                        .padding(iced::padding::left(8))
                        .align_y(iced::Alignment::Center)
                    )
                    .style(style::modal_container)
                ]
                .align_y(iced::Alignment::Center)
            } else {
                let pending_info = if is_pending {
                    Some(tooltip(
                        button("i").style(style::button::info),
                        Some("Pending changes require a full restart"),
                        iced::widget::tooltip::Position::Top,
                    ))
                } else {
                    None
                };

                let mut row_buttons = row![
                    iced::widget::space::horizontal(),
                    pending_info,
                    button("Apply").on_press(Message::RequestApply),
                ]
                .spacing(8);

                if self.proxy_url.is_some() {
                    row_buttons = row_buttons.push(tooltip(
                        button(style::icon_text(style::Icon::TrashBin, 11))
                            .on_press(Message::RequestClear)
                            .style(|theme, status| style::button::modifier(theme, status, true)),
                        Some("Unset proxy settings"),
                        iced::widget::tooltip::Position::Top,
                    ));
                }
                row_buttons
            };

            let mut body = column![
                row![
                    iced::widget::rule::horizontal(1),
                    text("Proxy").size(14),
                    iced::widget::rule::horizontal(1),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
                container(applied_proxy)
                    .style(style::modal_container)
                    .padding(8),
                column![scheme, column![host, port, username, password].spacing(6),].spacing(8),
            ]
            .spacing(12);

            body = if let Some(err) = &self.error {
                let error_line = text(err).size(12).style(|theme: &iced::Theme| {
                    let palette = theme.palette();
                    iced::widget::text::Style {
                        color: Some(palette.danger),
                    }
                });
                body.push(
                    container(error_line)
                        .align_x(iced::Alignment::Center)
                        .width(iced::Length::Fill),
                )
            } else {
                body
            };

            body.push(buttons)
        };

        container(column![modal_header, proxy_settings].spacing(12))
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }

    pub fn proxy_cfg(&self) -> Option<exchange::proxy::Proxy> {
        exchange::proxy::Proxy::try_from_str_strict(self.proxy_url.as_deref().unwrap_or("")).ok()
    }

    fn reset_transient(&mut self) {
        self.confirming_clear = false;
        self.confirming_apply = false;
        self.error = None;
    }

    /// Draft (form inputs) -> Option<Proxy>
    /// - Ok(None) means "no proxy" (all fields empty)
    /// - Err(...) means invalid draft
    fn build_proxy_cfg_from_parts(&self) -> Result<Option<Proxy>, String> {
        let host = self.host.trim();
        let port_s = self.port.trim();
        let u = self.username.trim();
        let p = self.password.trim();

        // All empty => None
        if host.is_empty() && port_s.is_empty() && u.is_empty() && p.is_empty() {
            return Ok(None);
        }

        if host.is_empty() {
            return Err("Proxy host is required".to_string());
        }

        let port: u16 = port_s
            .parse()
            .map_err(|_| "Proxy port must be a number (1-65535)".to_string())?;
        if port == 0 {
            return Err("Proxy port must be a number (1-65535)".to_string());
        }

        let has_user = !u.is_empty();
        let has_pass = !p.is_empty();
        if has_user ^ has_pass {
            return Err("Provide both username and password (or neither)".to_string());
        }

        let auth = if has_user && has_pass {
            Some(ProxyAuth {
                username: u.to_string(),
                password: p.to_string(),
            })
        } else {
            None
        };

        Ok(Some(Proxy {
            scheme: self.scheme,
            host: host.to_string(),
            port,
            auth,
        }))
    }
}

fn create_icon_button<'a>(
    icon: style::Icon,
    size: u16,
    style_fn: impl Fn(&Theme, &button::Status) -> button::Style + 'static,
    on_press: Option<Message>,
) -> button::Button<'a, Message> {
    let mut btn = button(icon_text(icon, size).align_y(iced::Alignment::Center))
        .style(move |theme, status| style_fn(theme, &status));

    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }

    btn
}
