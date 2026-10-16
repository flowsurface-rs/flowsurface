use crate::screen::dashboard::pane::content::kline::{Message, SettingsMessage};
use crate::tooltip;
use crate::widget::inspector::{InspectorContent, InspectorSection};
use data::chart::kline::LegendVisibility;
use iced::{
    Length,
    widget::{
        button, checkbox, column, container, pick_list, text, tooltip::Position as TooltipPosition,
    },
};

pub fn kline_cfg_view(
    trade_data_enabled: bool,
    legend_visibility: LegendVisibility,
    trade_data_lock_reason: Option<&'static str>,
) -> InspectorContent<'static, Message> {
    let trade_data_enabled_checkbox = {
        let cb = checkbox(trade_data_enabled)
            .label("Trade data")
            .on_toggle_maybe(trade_data_lock_reason.is_none().then_some(|enabled| {
                Message::Settings(SettingsMessage::TradeDataEnabledChanged(enabled))
            }));

        tooltip(
            cb,
            Some(
                trade_data_lock_reason
                    .unwrap_or("Allow plots that need trade data to subscribe and fetch it"),
            ),
            TooltipPosition::Left,
        )
    };
    let mut trade_data_setting = column![trade_data_enabled_checkbox].spacing(4);
    if trade_data_enabled {
        trade_data_setting = trade_data_setting
            .push(
                text("Live trade data is enabled.\n\nHistorical trade data depends on your network settings.")
                    .size(crate::style::text_size::SMALL),
            )
            .push(button("Open Network settings").on_press(Message::OpenNetworkSettings));
    }

    let legend_picklist = tooltip(
        pick_list(
            LegendVisibility::ALL,
            Some(legend_visibility),
            |visibility| Message::Settings(SettingsMessage::LegendVisibilityChanged(visibility)),
        )
        .width(Length::Fill),
        None,
        TooltipPosition::Left,
    );

    InspectorContent::new(vec![
        InspectorSection::titled(
            "Data",
            container(trade_data_setting)
                .width(Length::Fill)
                .padding(8)
                .style(crate::style::inspector_control),
        )
        .with_modal_container(),
        InspectorSection::titled("Legend visibility", legend_picklist).with_modal_container(),
    ])
}
