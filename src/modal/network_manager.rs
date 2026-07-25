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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmState {
    Idle,
    ProxyClear,
    ProxyApply,
    FetchApply,
}

#[derive(Debug, Clone)]
pub enum Message {
    Proxy(ProxyMsg),
    Fetch(FetchMsg),
    GoBack,
}

#[derive(Debug, Clone)]
pub struct NetworkManager {
    proxy: ProxySection,
    fetch: FetchSection,
    confirm: ConfirmState,
    error: Option<String>,
}

impl NetworkManager {
    pub fn new(proxy_cfg: Option<exchange::proxy::Proxy>) -> Self {
        Self {
            proxy: ProxySection::new(proxy_cfg),
            fetch: FetchSection::new(),
            confirm: ConfirmState::Idle,
            error: None,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::GoBack => {
                self.confirm = ConfirmState::Idle;
                self.fetch.revert_to_runtime();
                Some(Action::Exit)
            }
            Message::Proxy(msg) => self.proxy.update(msg, &mut self.confirm, &mut self.error),
            Message::Fetch(msg) => self.fetch.update(msg, &mut self.confirm, &mut self.error),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let modal_header = row![
            button(style::icon_text(style::Icon::Return, 11)).on_press(Message::GoBack),
            iced::widget::space::horizontal(),
        ];

        let proxy_settings = self
            .proxy
            .view(self.confirm, self.error.as_deref())
            .map(Message::Proxy);
        let fetch_settings = self.fetch.view(self.confirm).map(Message::Fetch);

        container(column![modal_header, fetch_settings, proxy_settings].spacing(12))
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }

    pub fn take_applied_draft(&mut self) -> Option<fetcher::TradeFetchMode> {
        self.fetch.take_applied()
    }

    pub fn proxy_cfg(&self) -> Option<exchange::proxy::Proxy> {
        self.proxy.as_config()
    }

    pub fn revert_fetch_drafts(&mut self) {
        self.confirm = ConfirmState::Idle;
        self.fetch.revert_to_runtime();
    }
}

#[derive(Debug, Clone)]
struct ProxySection {
    /// Saved/selected config (takes effect after restart).
    saved_url: Option<String>,
    /// Effective proxy at runtime (current process).
    effective_cfg: Option<Proxy>,
    scheme: ProxyScheme,
    host: String,
    port: String,
    username: String,
    password: String,
    hide_password: bool,
}

#[derive(Debug, Clone)]
pub enum ProxyMsg {
    ToggleShowPassword(bool),
    SchemeChanged(ProxyScheme),
    HostChanged(String),
    PortChanged(String),
    UsernameChanged(String),
    PasswordChanged(String),
    RequestApply,
    Apply,
    RequestClear,
    Clear,
    Cancel,
}

impl ProxySection {
    fn new(proxy_cfg: Option<Proxy>) -> Self {
        let (saved_url, scheme, host, port, username, password) = if let Some(ref cfg) = proxy_cfg {
            (
                Some(Proxy::to_url_string(cfg)),
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

        Self {
            saved_url,
            effective_cfg: proxy_cfg,
            scheme,
            host,
            port,
            username,
            password,
            hide_password: true,
        }
    }

    fn as_config(&self) -> Option<Proxy> {
        Proxy::try_from_str_strict(self.saved_url.as_deref().unwrap_or("")).ok()
    }

    fn is_pending(&self) -> bool {
        self.as_config() != self.effective_cfg
    }

    /// Build a `Proxy` from the form inputs.
    /// - `Ok(None)` means "no proxy" (all fields empty).
    /// - `Err(...)` means invalid draft.
    fn build_from_parts(&self) -> Result<Option<Proxy>, String> {
        let host = self.host.trim();
        let port_s = self.port.trim();
        let u = self.username.trim();
        let p = self.password.trim();

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

    fn view<'a>(&'a self, confirm: ConfirmState, error: Option<&'a str>) -> Element<'a, ProxyMsg> {
        let saved_cfg = self.as_config();
        let is_pending = self.is_pending();

        let applied_proxy = {
            let effective = self
                .effective_cfg
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

        let scheme_row = row![
            iced::widget::space::horizontal(),
            text("Scheme:"),
            pick_list(ProxyScheme::ALL, Some(self.scheme), ProxyMsg::SchemeChanged)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let host_row = row![
            iced::widget::space::horizontal(),
            text("Host:"),
            text_input("e.g. 127.0.0.1", &self.host)
                .on_input(ProxyMsg::HostChanged)
                .width(200)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let port_row = row![
            iced::widget::space::horizontal(),
            text("Port:"),
            text_input("e.g. 8080", &self.port)
                .on_input(ProxyMsg::PortChanged)
                .width(200)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let username_row = row![
            iced::widget::space::horizontal(),
            text("Username:"),
            text_input("(optional)", &self.username)
                .on_input(ProxyMsg::UsernameChanged)
                .width(200)
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let password_row = row![
            iced::widget::space::horizontal(),
            text("Password:"),
            text_input("(optional)", &self.password)
                .on_input(ProxyMsg::PasswordChanged)
                .width(180)
                .secure(self.hide_password),
            tooltip(
                checkbox(self.hide_password).on_toggle(ProxyMsg::ToggleShowPassword),
                Some("Hide"),
                iced::widget::tooltip::Position::Top,
            ),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);

        let buttons: Element<'_, ProxyMsg> = match confirm {
            ConfirmState::ProxyClear => confirm_row(
                "Unset proxy and clear inputs?",
                ProxyMsg::Clear,
                ProxyMsg::Cancel,
            ),
            ConfirmState::ProxyApply => confirm_row(
                "Changes will take effect after a restart",
                ProxyMsg::Apply,
                ProxyMsg::Cancel,
            ),
            _ => {
                let proxy_draft_differs = self.build_from_parts().ok() != Some(self.as_config());

                let mut row_buttons = row![
                    iced::widget::space::horizontal(),
                    pending_info::<ProxyMsg>(is_pending),
                ]
                .spacing(8);

                if proxy_draft_differs {
                    row_buttons =
                        row_buttons.push(button("Apply").on_press(ProxyMsg::RequestApply));
                }

                if self.saved_url.is_some() {
                    row_buttons = row_buttons.push(tooltip(
                        button(style::icon_text(style::Icon::TrashBin, 11))
                            .on_press(ProxyMsg::RequestClear)
                            .style(|theme, status| style::button::modifier(theme, status, true)),
                        Some("Unset proxy settings"),
                        iced::widget::tooltip::Position::Top,
                    ));
                }
                row_buttons.into()
            }
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
            column![
                scheme_row,
                column![host_row, port_row, username_row, password_row].spacing(6),
            ]
            .spacing(8),
        ]
        .spacing(12);

        if let Some(err) = error {
            let error_line =
                text(err)
                    .size(crate::style::text_size::BODY)
                    .style(|theme: &iced::Theme| {
                        let palette = theme.palette();
                        iced::widget::text::Style {
                            color: Some(palette.danger),
                        }
                    });
            body = body.push(
                container(error_line)
                    .align_x(iced::Alignment::Center)
                    .width(iced::Length::Fill),
            );
        }

        body.push(buttons).into()
    }

    fn update(
        &mut self,
        message: ProxyMsg,
        confirm: &mut ConfirmState,
        error: &mut Option<String>,
    ) -> Option<Action> {
        match message {
            ProxyMsg::ToggleShowPassword(v) => {
                self.hide_password = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::SchemeChanged(v) => {
                self.scheme = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::HostChanged(v) => {
                self.host = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::PortChanged(v) => {
                self.port = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::UsernameChanged(v) => {
                self.username = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::PasswordChanged(v) => {
                self.password = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            ProxyMsg::RequestApply => {
                *confirm = ConfirmState::Idle;
                match self.build_from_parts() {
                    Ok(draft_cfg) => {
                        if draft_cfg != self.as_config() {
                            *confirm = ConfirmState::ProxyApply;
                        }
                        *error = None;
                    }
                    Err(e) => {
                        *error = Some(e);
                    }
                }
            }
            ProxyMsg::Apply => {
                *confirm = ConfirmState::Idle;
                match self.build_from_parts() {
                    Ok(Some(cfg)) => match cfg.try_to_url_string() {
                        Ok(url) => {
                            self.saved_url = Some(url);
                            *error = None;
                            return Some(Action::ApplyProxy);
                        }
                        Err(e) => {
                            *error = Some(e);
                        }
                    },
                    Ok(None) => {
                        self.saved_url = None;
                        *error = None;
                        return Some(Action::ApplyProxy);
                    }
                    Err(e) => {
                        *error = Some(e);
                    }
                }
            }
            ProxyMsg::RequestClear => {
                *confirm = ConfirmState::ProxyClear;
                *error = None;
            }
            ProxyMsg::Clear => {
                *confirm = ConfirmState::Idle;
                *error = None;
                self.saved_url = None;
                self.host.clear();
                self.port.clear();
                self.username.clear();
                self.password.clear();
                self.scheme = ProxyScheme::Http;
                return Some(Action::ApplyProxy);
            }
            ProxyMsg::Cancel => {
                *confirm = ConfirmState::Idle;
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
struct FetchSection {
    server_url_input: String,
    server_auth_token_input: String,
    expanded_server: bool,
    draft_active: bool,
    draft_tag: FetchModeTag,
    /// The mode that was just applied, awaiting restart confirmation.
    applied_mode: Option<fetcher::TradeFetchMode>,
    hide_token: bool,
}

#[derive(Debug, Clone)]
pub enum FetchMsg {
    ToggleShowToken(bool),
    ServerUrlChanged(String),
    ServerAuthTokenChanged(String),
    ToggleFetchTrades(bool),
    SelectFetchMode(FetchModeTag),
    ToggleServerConfig,
    RequestApplyFetch,
    ApplyFetchSettings,
    Cancel,
}

impl FetchSection {
    fn new() -> Self {
        let runtime = fetcher::trade_fetch_mode();
        Self {
            server_url_input: runtime.server_url().unwrap_or("").to_string(),
            server_auth_token_input: runtime.server_auth_token().unwrap_or("").to_string(),
            expanded_server: false,
            draft_active: runtime != fetcher::TradeFetchMode::Off,
            draft_tag: match &runtime {
                fetcher::TradeFetchMode::Server { .. } => FetchModeTag::Server,
                _ => FetchModeTag::Exchange,
            },
            applied_mode: None,
            hide_token: true,
        }
    }

    fn build_mode(&self) -> fetcher::TradeFetchMode {
        if !self.draft_active {
            return fetcher::TradeFetchMode::Off;
        }
        match self.draft_tag {
            FetchModeTag::Exchange => fetcher::TradeFetchMode::Exchange,
            FetchModeTag::Server => {
                let trimmed = self.server_url_input.trim().trim_end_matches('/');
                let auth = self.server_auth_token_input.trim();
                let url = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                let auth_token = if auth.is_empty() {
                    None
                } else {
                    Some(auth.to_string())
                };
                fetcher::TradeFetchMode::Server { url, auth_token }
            }
        }
    }

    fn take_applied(&mut self) -> Option<fetcher::TradeFetchMode> {
        self.applied_mode.take()
    }

    fn revert_to_runtime(&mut self) {
        self.applied_mode = None;
        let runtime = fetcher::trade_fetch_mode();
        self.draft_active = runtime != fetcher::TradeFetchMode::Off;
        self.draft_tag = match &runtime {
            fetcher::TradeFetchMode::Server { .. } => FetchModeTag::Server,
            _ => FetchModeTag::Exchange,
        };
        self.server_url_input = runtime.server_url().unwrap_or("").to_string();
        self.server_auth_token_input = runtime.server_auth_token().unwrap_or("").to_string();
    }

    fn view(&self, confirm: ConfirmState) -> Element<'_, FetchMsg> {
        let mode = fetcher::trade_fetch_mode();
        let selected_tag = if self.draft_active {
            Some(self.draft_tag)
        } else {
            None
        };

        let fetch_checkbox = checkbox(self.draft_active)
            .label("Fetch trades")
            .on_toggle(FetchMsg::ToggleFetchTrades);

        let mut fetch_section = column![tooltip(
            fetch_checkbox,
            Some("Fetch historical trade data for footprint charts"),
            iced::widget::tooltip::Position::Top,
        ),]
        .spacing(8);

        if self.draft_active {
            let exchange_radio = container(radio(
                "Exchange (Binance only)",
                FetchModeTag::Exchange,
                selected_tag,
                FetchMsg::SelectFetchMode,
            ))
            .height(32)
            .style(style::modal_container)
            .padding(iced::padding::left(8).top(4).bottom(4).right(4))
            .align_y(iced::Alignment::Center)
            .width(iced::Length::Fill);

            let server_radio = {
                let radio_row = row![
                    radio(
                        "Server",
                        FetchModeTag::Server,
                        selected_tag,
                        FetchMsg::SelectFetchMode,
                    ),
                    iced::widget::space::horizontal(),
                    button(icon_text(style::Icon::Cog, 11))
                        .on_press(FetchMsg::ToggleServerConfig)
                        .style(move |theme, status| {
                            style::button::transparent(theme, status, self.expanded_server)
                        })
                ]
                .align_y(iced::Alignment::Center);

                let config = if self.expanded_server {
                    let server_input = row![
                        iced::widget::space::horizontal(),
                        text("URL:"),
                        text_input("http://127.0.0.1:8080", &self.server_url_input)
                            .on_input(FetchMsg::ServerUrlChanged)
                            .width(180),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center);

                    let auth_token_input = row![
                        iced::widget::space::horizontal(),
                        text("Auth:"),
                        text_input("(optional)", &self.server_auth_token_input)
                            .on_input(FetchMsg::ServerAuthTokenChanged)
                            .width(160)
                            .secure(self.hide_token),
                        tooltip(
                            checkbox(self.hide_token).on_toggle(FetchMsg::ToggleShowToken),
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

                container(column![radio_row, config].spacing(4))
                    .style(style::modal_container)
                    .padding(iced::padding::left(8).top(4).bottom(4).right(4))
                    .width(iced::Length::Fill)
            };

            fetch_section = fetch_section.push(
                container(column![exchange_radio, server_radio].spacing(4))
                    .padding(iced::padding::left(20)),
            );
        }

        let draft_changed = self.build_mode() != mode;

        let buttons: Element<'_, FetchMsg> = if confirm == ConfirmState::FetchApply {
            confirm_row(
                "Changes will take effect after a restart",
                FetchMsg::ApplyFetchSettings,
                FetchMsg::Cancel,
            )
        } else {
            let mut row_buttons = row![
                iced::widget::space::horizontal(),
                pending_info::<FetchMsg>(self.applied_mode.is_some()),
            ]
            .spacing(8);

            if draft_changed {
                row_buttons =
                    row_buttons.push(button("Apply").on_press(FetchMsg::RequestApplyFetch));
            }
            row_buttons.into()
        };

        fetch_section = fetch_section.push(buttons);

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
        .into()
    }

    fn update(
        &mut self,
        message: FetchMsg,
        confirm: &mut ConfirmState,
        error: &mut Option<String>,
    ) -> Option<Action> {
        match message {
            FetchMsg::ToggleShowToken(v) => {
                self.hide_token = v;
                *confirm = ConfirmState::Idle;
                *error = None;
            }
            FetchMsg::ServerUrlChanged(v) => {
                self.server_url_input = v;
                *error = None;
            }
            FetchMsg::ServerAuthTokenChanged(v) => {
                self.server_auth_token_input = v;
                *error = None;
            }
            FetchMsg::ToggleFetchTrades(checked) => {
                *error = None;
                self.draft_active = checked;
            }
            FetchMsg::SelectFetchMode(tag) => {
                *error = None;
                self.draft_tag = tag;
            }
            FetchMsg::ToggleServerConfig => {
                *error = None;
                self.expanded_server = !self.expanded_server;
            }
            FetchMsg::RequestApplyFetch => {
                *error = None;
                *confirm = ConfirmState::FetchApply;
            }
            FetchMsg::ApplyFetchSettings => {
                *error = None;
                let mode = self.build_mode();
                self.applied_mode = Some(mode.clone());
                return Some(Action::TradeFetchModeChanged(mode));
            }
            FetchMsg::Cancel => {
                *confirm = ConfirmState::Idle;
            }
        }
        None
    }
}

fn confirm_row<'a, M: Clone + 'a>(label: &'a str, confirm_msg: M, cancel_msg: M) -> Element<'a, M> {
    row![
        iced::widget::space::horizontal(),
        container(
            row![
                text(label),
                create_icon_button(
                    style::Icon::Checkmark,
                    12,
                    |theme, status| style::button::confirm(theme, *status, true),
                    Some(confirm_msg),
                ),
                create_icon_button(
                    style::Icon::Close,
                    12,
                    |theme, status| style::button::cancel(theme, *status, true),
                    Some(cancel_msg),
                ),
            ]
            .padding(iced::padding::left(8))
            .align_y(iced::Alignment::Center)
        )
        .style(style::modal_container)
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

/// Pending-restart tooltip, if applicable.
fn pending_info<M: Clone + 'static>(is_pending: bool) -> Option<Element<'static, M>> {
    if is_pending {
        Some(tooltip(
            button("i").style(style::button::info),
            Some("Pending changes require a full restart"),
            iced::widget::tooltip::Position::Top,
        ))
    } else {
        None
    }
}

fn create_icon_button<'a, M: Clone + 'a>(
    icon: style::Icon,
    size: u16,
    style_fn: impl Fn(&Theme, &button::Status) -> button::Style + 'static,
    on_press: Option<M>,
) -> button::Button<'a, M> {
    let mut btn = button(icon_text(icon, size).align_y(iced::Alignment::Center))
        .style(move |theme, status| style_fn(theme, &status));

    if let Some(msg) = on_press {
        btn = btn.on_press(msg);
    }

    btn
}
