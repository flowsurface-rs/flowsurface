use crate::{
    connector::fetcher,
    style::{self, icon_text},
    widget::tooltip,
};
use exchange::proxy::{Proxy, ProxyAuth, ProxyScheme};

use iced::{
    Element, Theme,
    widget::{button, checkbox, column, container, pick_list, radio, row, text, text_input},
};

pub enum Action {
    ApplyProxy,
    TradeFetchModeChanged(fetcher::TradeFetchMode),
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchModeTag {
    Exchange,
    Server,
}

#[derive(Debug, Clone)]
pub enum Message {
    GoBack,
    ToggleShowPassword(bool),
    ToggleShowToken(bool),
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
    ServerUrlChanged(String),
    ServerAuthTokenChanged(String),
    ToggleFetchTrades(bool),
    SelectFetchMode(FetchModeTag),
    ToggleServerConfig,
    ApplyFetchSettings,
    /// Revert all drafts to match the current runtime mode.
    RevertFetchDrafts,
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
    hide_token: bool,

    /// Current text in the server URL input field (draft, not yet applied).
    server_url_input: String,
    /// Current text in the server auth token input field (draft).
    server_auth_token_input: String,
    expanded_server: bool,

    /// Draft fetch configuration — applied only when the user clicks "Apply".
    draft_fetch_active: bool,
    draft_fetch_tag: FetchModeTag,
    /// The mode that was just applied, awaiting restart confirmation.
    /// `None` means no pending apply.
    applied_draft_mode: Option<fetcher::TradeFetchMode>,
}

impl NetworkManager {
    pub fn new(proxy_cfg: Option<exchange::proxy::Proxy>) -> Self {
        let (proxy_url, scheme, host, port, username, password) =
            if let Some(cfg) = proxy_cfg.clone() {
                let url = exchange::proxy::Proxy::to_url_string(&cfg);
                (
                    Some(url),
                    cfg.scheme(),
                    cfg.host().to_string(),
                    cfg.port().to_string(),
                    cfg.auth()
                        .map(|a| a.username().to_string())
                        .unwrap_or_default(),
                    cfg.auth()
                        .map(|a| a.password().to_string())
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

        let server_url_input = fetcher::trade_fetch_mode()
            .server_url()
            .unwrap_or("")
            .to_string();
        let server_auth_token_input = fetcher::trade_fetch_mode()
            .server_auth_token()
            .unwrap_or("")
            .to_string();

        let runtime_mode = fetcher::trade_fetch_mode();
        let draft_fetch_active = runtime_mode != fetcher::TradeFetchMode::Off;
        let draft_fetch_tag = match &runtime_mode {
            fetcher::TradeFetchMode::Server { .. } => FetchModeTag::Server,
            _ => FetchModeTag::Exchange,
        };

        Self {
            proxy_url,
            effective_proxy_cfg: proxy_cfg,
            error: None,
            hide_password: true,
            hide_token: true,
            scheme,
            host,
            port,
            username,
            password,
            confirming_clear: false,
            confirming_apply: false,
            server_url_input,
            server_auth_token_input,
            expanded_server: false,
            draft_fetch_active,
            draft_fetch_tag,
            applied_draft_mode: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::ToggleShowPassword(v) => {
                self.hide_password = v;
                self.reset_transient();
            }
            Message::ToggleShowToken(v) => {
                self.hide_token = v;
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
            Message::ServerUrlChanged(v) => {
                self.server_url_input = v;
                self.error = None;
            }
            Message::ServerAuthTokenChanged(v) => {
                self.server_auth_token_input = v;
                self.error = None;
            }
            Message::ToggleFetchTrades(checked) => {
                self.error = None;
                self.draft_fetch_active = checked;
            }
            Message::SelectFetchMode(tag) => {
                self.error = None;
                self.draft_fetch_tag = tag;
            }
            Message::ToggleServerConfig => {
                self.error = None;
                self.expanded_server = !self.expanded_server;
            }
            Message::ApplyFetchSettings => {
                self.error = None;
                let mode = self.build_draft_mode();
                self.applied_draft_mode = Some(mode.clone());
                return Some(Action::TradeFetchModeChanged(mode));
            }
            Message::RevertFetchDrafts => {
                self.applied_draft_mode = None;
                let runtime = fetcher::trade_fetch_mode();
                self.draft_fetch_active = runtime != fetcher::TradeFetchMode::Off;
                self.draft_fetch_tag = match &runtime {
                    fetcher::TradeFetchMode::Server { .. } => FetchModeTag::Server,
                    _ => FetchModeTag::Exchange,
                };
                self.server_url_input = runtime.server_url().unwrap_or("").to_string();
                self.server_auth_token_input =
                    runtime.server_auth_token().unwrap_or("").to_string();
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
                    row![
                        text("Effective:").size(crate::style::text_size::SMALL),
                        text(effective).size(crate::style::text_size::BODY),
                    ]
                    .spacing(4)
                    .align_y(iced::Alignment::Center)
                    .width(iced::Length::Fill),
                ]
                .spacing(4);

                if let Some(pending) = pending_url {
                    lines = lines.push(
                        row![
                            text("Pending:").size(crate::style::text_size::SMALL),
                            text(pending).size(crate::style::text_size::BODY),
                        ]
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
                    Some("Hide"),
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
                    text("Proxy").size(crate::style::text_size::SECTION),
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
                let error_line =
                    text(err)
                        .size(crate::style::text_size::BODY)
                        .style(|theme: &iced::Theme| {
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

        let fetch_settings = {
            let mode = fetcher::trade_fetch_mode();
            let selected_tag = if self.draft_fetch_active {
                Some(self.draft_fetch_tag)
            } else {
                None
            };

            let fetch_checkbox = checkbox(self.draft_fetch_active)
                .label("Fetch trades")
                .on_toggle(Message::ToggleFetchTrades);

            let mut fetch_section = column![tooltip(
                fetch_checkbox,
                Some("Fetch historical trade data for footprint charts"),
                iced::widget::tooltip::Position::Top,
            ),]
            .spacing(8);

            if self.draft_fetch_active {
                let exchange_radio = container(radio(
                    "Exchange (Binance only)",
                    FetchModeTag::Exchange,
                    selected_tag,
                    Message::SelectFetchMode,
                ))
                .height(32)
                .style(style::modal_container)
                .padding(iced::padding::left(8).top(4).bottom(4).right(4))
                .align_y(iced::Alignment::Center)
                .width(iced::Length::Fill);

                let server_radio = {
                    let radio = row![
                        radio(
                            "Server",
                            FetchModeTag::Server,
                            selected_tag,
                            Message::SelectFetchMode,
                        ),
                        iced::widget::space::horizontal(),
                        tooltip(
                            button(icon_text(style::Icon::Cog, 11))
                                .on_press(Message::ToggleServerConfig)
                                .style(move |theme, status| {
                                    style::button::transparent(theme, status, self.expanded_server)
                                }),
                            Some("Configure server URL and settings"),
                            iced::widget::tooltip::Position::Top,
                        ),
                    ]
                    .align_y(iced::Alignment::Center);

                    let config = if self.expanded_server {
                        let server_input = row![
                            iced::widget::space::horizontal(),
                            text("URL:"),
                            text_input("http://127.0.0.1:8080", &self.server_url_input)
                                .on_input(Message::ServerUrlChanged)
                                .width(180),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center);

                        let auth_token_input = row![
                            iced::widget::space::horizontal(),
                            text("Auth:"),
                            text_input("(optional)", &self.server_auth_token_input)
                                .on_input(Message::ServerAuthTokenChanged)
                                .width(160)
                                .secure(self.hide_token),
                            tooltip(
                                checkbox(self.hide_token).on_toggle(Message::ToggleShowToken),
                                Some("Hide"),
                                iced::widget::tooltip::Position::Top,
                            ),
                        ]
                        .spacing(4)
                        .align_y(iced::Alignment::Center);

                        Some(
                            column![server_input, auth_token_input]
                                .align_x(iced::Alignment::End)
                                .spacing(6),
                        )
                    } else {
                        None
                    };

                    container(column![radio, config].spacing(4))
                        .style(style::modal_container)
                        .padding(iced::padding::left(8).top(4).bottom(4).right(4))
                        .width(iced::Length::Fill)
                };

                fetch_section = fetch_section.push(
                    container(column![exchange_radio, server_radio].spacing(4))
                        .padding(iced::padding::left(20)),
                );
            }

            let draft_changed = self.build_draft_mode() != mode;

            let apply_button = if draft_changed {
                Some(button("Apply").on_press(Message::ApplyFetchSettings))
            } else {
                None
            };

            if let Some(btn) = apply_button {
                fetch_section = fetch_section.push(
                    row![iced::widget::space::horizontal(), btn].align_y(iced::Alignment::Center),
                );
            }

            column![
                row![
                    iced::widget::rule::horizontal(1),
                    text("Historical backfill").size(crate::style::text_size::SECTION),
                    iced::widget::rule::horizontal(1),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
                container(fetch_section)
                    .width(iced::Length::Fill)
                    .style(style::modal_container)
                    .padding(8),
            ]
            .spacing(8)
        };

        container(column![modal_header, fetch_settings, proxy_settings].spacing(12))
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }

    /// Build the `TradeFetchMode` from the current draft state.
    fn build_draft_mode(&self) -> fetcher::TradeFetchMode {
        if !self.draft_fetch_active {
            return fetcher::TradeFetchMode::Off;
        }

        match self.draft_fetch_tag {
            FetchModeTag::Exchange => fetcher::TradeFetchMode::Exchange,
            FetchModeTag::Server => {
                let trimmed = self.server_url_input.trim().trim_end_matches('/');
                let auth = self.server_auth_token_input.trim();
                let auth_token = if auth.is_empty() {
                    None
                } else {
                    Some(auth.to_string())
                };
                let url = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                fetcher::TradeFetchMode::Server { url, auth_token }
            }
        }
    }

    /// Take the just-applied draft mode (if any), consuming it so it is
    /// only applied once on restart confirmation.
    pub fn take_applied_draft(&mut self) -> Option<fetcher::TradeFetchMode> {
        self.applied_draft_mode.take()
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
            Some(ProxyAuth::try_new(u, p)?)
        } else {
            None
        };

        Proxy::new(self.scheme, host.to_string(), port, auth).map(Some)
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
