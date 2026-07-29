use crate::{
    connector::fetcher,
    style::{self, icon_text},
    widget::tooltip,
};
use exchange::proxy::{Proxy, ProxyScheme};

use iced::{
    Element, Theme,
    widget::{button, checkbox, column, container, pick_list, radio, row, text, text_input},
};

pub enum Action {
    ApplyProxy(Option<exchange::proxy::Proxy>),
    ApplyServerConfig {
        mode: fetcher::TradeFetchMode,
        url: Option<String>,
        auth_token: Option<String>,
    },
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmState {
    Idle,
    ProxyClear,
    FetchClear,
}

#[derive(Debug, Clone)]
pub enum Message {
    Proxy(ProxyMsg),
    Fetch(FetchMsg),
    GoBack,
}

#[derive(Debug, Clone)]
pub struct NetworkEditor {
    proxy: ProxyForm,
    fetch: FetchForm,
    confirm: ConfirmState,
    error: Option<String>,
    /// Snapshot of the effective config when the editor was opened or last
    /// reset. Used to revert drafts when the user cancels a restart confirmation.
    effective: data::Network,
    /// The full network config that was applied (proxy or server) but needs
    /// a restart to take effect. `None` when no pending restart is needed.
    pending_apply: Option<data::Network>,
}

impl NetworkEditor {
    /// Create a new `NetworkEditor` with form fields pre-populated from
    /// the caller's effective network config.
    pub fn new(network: &data::Network, pending_apply: Option<data::Network>) -> Self {
        let mut proxy = ProxyForm::new();
        proxy.populate_from(network.proxy.as_ref());
        Self {
            proxy,
            fetch: FetchForm::new(
                &network.trade_fetch_mode,
                network.server_url.as_deref(),
                network.server_auth_token.as_deref(),
            ),
            confirm: ConfirmState::Idle,
            error: None,
            effective: network.clone(),
            pending_apply,
        }
    }

    pub fn update(&mut self, message: Message) -> Option<Action> {
        match message {
            Message::GoBack => {
                self.confirm = ConfirmState::Idle;
                Some(Action::Exit)
            }
            Message::Proxy(msg) => {
                let action = self.proxy.update(
                    msg,
                    &mut self.confirm,
                    &mut self.error,
                    self.effective.proxy.as_ref(),
                );
                if let Some(Action::ApplyProxy(ref cfg)) = action {
                    self.pending_apply = Some(data::Network {
                        proxy: cfg.clone(),
                        ..self.effective.clone()
                    });
                }
                action
            }
            Message::Fetch(msg) => {
                let action = self.fetch.update(
                    msg,
                    &mut self.confirm,
                    &mut self.error,
                    &self.effective.trade_fetch_mode,
                    self.effective.server_url.as_deref(),
                    self.effective.server_auth_token.as_deref(),
                );
                if let Some(Action::ApplyServerConfig {
                    mode,
                    url,
                    auth_token,
                }) = &action
                {
                    self.pending_apply = Some(data::Network {
                        trade_fetch_mode: mode.clone(),
                        server_url: url.clone(),
                        server_auth_token: auth_token.clone(),
                        ..self.effective.clone()
                    });
                }
                action
            }
        }
    }

    /// Borrow the pending config that needs a restart to take effect, if any.
    pub fn pending_apply(&self) -> Option<&data::Network> {
        self.pending_apply.as_ref()
    }

    pub fn view<'a>(&'a self, network: &'a data::Network) -> Element<'a, Message> {
        let modal_header = row![
            button(style::icon_text(style::Icon::Return, 11)).on_press(Message::GoBack),
            iced::widget::space::horizontal(),
        ];

        let proxy_settings = self
            .proxy
            .view(network.proxy.as_ref(), self.confirm)
            .map(Message::Proxy);
        let fetch_settings = self
            .fetch
            .view(
                self.confirm,
                &network.trade_fetch_mode,
                network.server_url.as_deref(),
                network.server_auth_token.as_deref(),
            )
            .map(Message::Fetch);

        let mut content = column![modal_header, fetch_settings, proxy_settings].spacing(12);

        if self.pending_apply.is_some() {
            let banner = container(
                text("Restart required.\nChanges will take effect after restart")
                    .size(crate::style::text_size::SMALL)
                    .style(|theme: &iced::Theme| {
                        let palette = theme.palette();
                        iced::widget::text::Style {
                            color: Some(palette.primary),
                        }
                    }),
            )
            .padding(6)
            .width(iced::Length::Fill)
            .align_x(iced::Alignment::Center)
            .style(style::modal_container);
            content = content.push(banner);
        }

        let content = if let Some(err) = &self.error {
            let error_line =
                text(err)
                    .size(crate::style::text_size::BODY)
                    .style(|theme: &iced::Theme| {
                        let palette = theme.palette();
                        iced::widget::text::Style {
                            color: Some(palette.danger),
                        }
                    });
            content.push(
                container(error_line)
                    .align_x(iced::Alignment::Center)
                    .width(iced::Length::Fill),
            )
        } else {
            content
        };

        container(content)
            .max_width(320)
            .padding(24)
            .style(style::dashboard_modal)
            .into()
    }
}

#[derive(Debug, Clone)]
struct ProxyForm {
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
    RequestClear,
    Clear,
    Cancel,
}

impl ProxyForm {
    fn new() -> Self {
        Self {
            scheme: ProxyScheme::Http,
            host: String::new(),
            port: String::new(),
            username: String::new(),
            password: String::new(),
            hide_password: true,
        }
    }

    /// Fill form fields from a reference proxy config (used on init).
    fn populate_from(&mut self, cfg: Option<&Proxy>) {
        if let Some(cfg) = cfg {
            self.scheme = cfg.scheme();
            self.host = cfg.host().to_string();
            self.port = cfg.port().to_string();
            self.username = cfg
                .auth()
                .map(|a| a.username().to_string())
                .unwrap_or_default();
            self.password = cfg
                .auth()
                .map(|a| a.password().to_string())
                .unwrap_or_default();
        }
    }

    /// Whether the form state differs from the effective (in-use) config.
    fn is_pending(&self, effective_cfg: Option<&Proxy>) -> bool {
        let draft_has_content = ![&self.host, &self.port, &self.username, &self.password]
            .iter()
            .all(|s| s.trim().is_empty());

        match effective_cfg {
            Some(effective) => {
                draft_has_content
                    && (self.scheme != effective.scheme()
                        || self.host.trim() != effective.host()
                        || self.port.trim() != effective.port().to_string()
                        || self.username.trim()
                            != effective.auth().map(|a| a.username()).unwrap_or("")
                        || self.password.trim()
                            != effective.auth().map(|a| a.password()).unwrap_or(""))
            }
            None => draft_has_content,
        }
    }

    /// Build a `Proxy` from the form inputs.
    /// - `Ok(None)` means "no proxy" (all fields empty).
    /// - `Err(...)` means invalid draft.
    fn build_from_parts(&self) -> Result<Option<Proxy>, String> {
        let all_empty = [&self.host, &self.port, &self.username, &self.password]
            .iter()
            .all(|s| s.trim().is_empty());

        if all_empty {
            Ok(None)
        } else {
            Proxy::from_raw_parts(
                self.scheme,
                &self.host,
                &self.port,
                &self.username,
                &self.password,
            )
            .map(Some)
        }
    }

    fn view<'a>(
        &'a self,
        effective_cfg: Option<&'a Proxy>,
        confirm: ConfirmState,
    ) -> Element<'a, ProxyMsg> {
        let is_pending = self.is_pending(effective_cfg);

        let applied_proxy = {
            let effective = effective_cfg
                .map(|c| c.to_ui_string())
                .unwrap_or_else(|| "None (direct connection)".to_string());

            let pending_url = if is_pending {
                Some(
                    self.build_from_parts()
                        .ok()
                        .flatten()
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

        let buttons: Element<'_, ProxyMsg> = {
            let msg = "Restore proxy settings to default";

            match confirm {
                ConfirmState::ProxyClear => confirm_row(msg, ProxyMsg::Clear, ProxyMsg::Cancel),
                _ => apply_action_row(
                    is_pending,
                    is_pending,
                    effective_cfg.is_some(),
                    ProxyMsg::RequestApply,
                    ProxyMsg::RequestClear,
                    msg,
                ),
            }
        };

        let body = column![
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

        body.push(buttons).into()
    }

    fn update(
        &mut self,
        message: ProxyMsg,
        confirm: &mut ConfirmState,
        error: &mut Option<String>,
        _effective_cfg: Option<&Proxy>,
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
                    Ok(cfg) => {
                        *error = None;
                        return Some(Action::ApplyProxy(cfg));
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
                self.host.clear();
                self.port.clear();
                self.username.clear();
                self.password.clear();
                self.scheme = ProxyScheme::Http;
                return Some(Action::ApplyProxy(None));
            }
            ProxyMsg::Cancel => {
                *confirm = ConfirmState::Idle;
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
struct FetchForm {
    server_url_input: String,
    server_auth_token_input: String,
    expanded_server: bool,
    draft_active: bool,
    draft_tag: FetchModeTag,
    hide_token: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchModeTag {
    Exchange,
    Server,
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
    Cancel,
    RequestClearAuth,
    ConfirmClearAuth,
}

impl FetchForm {
    fn new(
        effective_mode: &fetcher::TradeFetchMode,
        effective_url: Option<&str>,
        effective_auth_token: Option<&str>,
    ) -> Self {
        Self {
            server_url_input: effective_url.unwrap_or("").to_string(),
            server_auth_token_input: effective_auth_token.unwrap_or("").to_string(),
            expanded_server: false,
            draft_active: *effective_mode != fetcher::TradeFetchMode::Off,
            draft_tag: match effective_mode {
                fetcher::TradeFetchMode::Server => FetchModeTag::Server,
                _ => FetchModeTag::Exchange,
            },
            hide_token: true,
        }
    }

    fn build_mode(&self) -> fetcher::TradeFetchMode {
        if !self.draft_active {
            return fetcher::TradeFetchMode::Off;
        }
        match self.draft_tag {
            FetchModeTag::Exchange => fetcher::TradeFetchMode::Exchange,
            FetchModeTag::Server => fetcher::TradeFetchMode::Server,
        }
    }

    fn trimmed_url(&self) -> Option<String> {
        let trimmed = self.server_url_input.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn trimmed_auth(&self) -> Option<String> {
        let auth = self.server_auth_token_input.trim();
        if auth.is_empty() {
            None
        } else {
            Some(auth.to_string())
        }
    }

    fn server_config_pending(
        &self,
        effective_mode: &fetcher::TradeFetchMode,
        effective_url: Option<&str>,
        effective_auth: Option<&str>,
    ) -> bool {
        let draft_mode = self.build_mode();
        if draft_mode != *effective_mode {
            return true;
        }
        if (draft_mode == fetcher::TradeFetchMode::Server
            && self.trimmed_url().as_deref() != effective_url)
            || self.trimmed_auth().as_deref() != effective_auth
        {
            return true;
        }
        false
    }

    fn view<'a>(
        &'a self,
        confirm: ConfirmState,
        effective_mode: &'a fetcher::TradeFetchMode,
        effective_url: Option<&'a str>,
        effective_auth: Option<&'a str>,
    ) -> Element<'a, FetchMsg> {
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

        // Pending = draft differs from the effective (applied) config
        let pending = self.server_config_pending(effective_mode, effective_url, effective_auth);
        let draft_changed = pending;
        let has_server_config = effective_auth.is_some()
            || (*effective_mode == fetcher::TradeFetchMode::Server && effective_url.is_some());

        let buttons: Element<'_, FetchMsg> = {
            let msg = "Restore fetch settings to default";

            match confirm {
                ConfirmState::FetchClear => {
                    confirm_row(msg, FetchMsg::ConfirmClearAuth, FetchMsg::Cancel)
                }
                _ => apply_action_row(
                    pending,
                    draft_changed,
                    has_server_config,
                    FetchMsg::RequestApplyFetch,
                    FetchMsg::RequestClearAuth,
                    msg,
                ),
            }
        };

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
            buttons,
        ]
        .spacing(8)
        .into()
    }

    fn update(
        &mut self,
        message: FetchMsg,
        confirm: &mut ConfirmState,
        error: &mut Option<String>,
        _effective_mode: &fetcher::TradeFetchMode,
        _effective_url: Option<&str>,
        _effective_auth: Option<&str>,
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

                if self.draft_tag == FetchModeTag::Server && self.draft_active {
                    let url = self.trimmed_url();
                    if url.is_none() || url.as_deref().is_none_or(|u| u.trim().is_empty()) {
                        *error =
                            Some("Server URL is required when mode is set to Server".to_string());
                        return None;
                    }
                    if let Some(ref u) = url
                        && url::Url::parse(u).is_err()
                    {
                        *error = Some("Server URL is not a valid URL".to_string());
                        return None;
                    }
                }

                let mode = self.build_mode();
                let url = self.trimmed_url();
                let auth_token = self.trimmed_auth();
                return Some(Action::ApplyServerConfig {
                    mode,
                    url,
                    auth_token,
                });
            }
            FetchMsg::Cancel => {
                *confirm = ConfirmState::Idle;
            }
            FetchMsg::RequestClearAuth => {
                *confirm = ConfirmState::FetchClear;
                *error = None;
            }
            FetchMsg::ConfirmClearAuth => {
                *confirm = ConfirmState::Idle;
                *error = None;
                self.server_url_input.clear();
                self.server_auth_token_input.clear();
                self.draft_active = false;
                self.expanded_server = false;
                return Some(Action::ApplyServerConfig {
                    mode: fetcher::TradeFetchMode::Off,
                    url: None,
                    auth_token: None,
                });
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
                text(label).height(28).align_y(iced::Alignment::Center),
                create_icon_button(
                    style::Icon::Checkmark,
                    12,
                    |theme, status| style::button::confirm(theme, *status, true),
                    Some(confirm_msg),
                )
                .height(28),
                create_icon_button(
                    style::Icon::Close,
                    12,
                    |theme, status| style::button::cancel(theme, *status, true),
                    Some(cancel_msg),
                )
                .height(28),
            ]
            .padding(iced::padding::left(8))
            .align_y(iced::Alignment::Center)
        )
        .style(style::modal_container)
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

/// The common pattern shared by both proxy and fetch forms:
/// Spacer, pending-info tooltip, Apply button and TrashBin button.
fn apply_action_row<'a, M: Clone + 'a + 'static>(
    pending: bool,
    draft_differs: bool,
    has_config: bool,
    apply_msg: M,
    clear_msg: M,
    clear_tooltip: &'a str,
) -> Element<'a, M> {
    let mut row_buttons = row![
        iced::widget::space::horizontal(),
        pending_info::<M>(pending),
    ]
    .spacing(8);

    if draft_differs {
        row_buttons = row_buttons.push(button("Apply").on_press(apply_msg).height(28));
    }

    if has_config {
        row_buttons = row_buttons.push(tooltip(
            button(style::icon_text(style::Icon::TrashBin, 11))
                .on_press(clear_msg)
                .height(28)
                .style(|theme, status| style::button::modifier(theme, status, true)),
            Some(clear_tooltip),
            iced::widget::tooltip::Position::Top,
        ));
    }

    row_buttons.into()
}

/// Pending-restart tooltip, if applicable.
fn pending_info<M: Clone + 'static>(is_pending: bool) -> Option<Element<'static, M>> {
    if is_pending {
        Some(tooltip(
            button("i").style(style::button::info).height(28),
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
