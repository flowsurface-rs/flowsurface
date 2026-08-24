use crate::style;
use crate::widget::chart::kline::composition::{ChartComposition, PanelId, PanelRole};
use crate::widget::chart::kline::drawing::{
    DrawingAnchor, DrawingColor, DrawingDraft, DrawingDragOperation, DrawingDragTarget,
    DrawingEntity, DrawingId, DrawingMeasure, DrawingObject, DrawingSnapshot, DrawingStyle,
    DrawingStyleDefaults, DrawingTool, KlineWidgetDrawingEvent, MAX_DRAWING_STROKE_WIDTH,
    MIN_DRAWING_STROKE_WIDTH, MeasureState,
};
use crate::widget::{color_swatch_button, labeled_slider};
use data::config::theme::{ThemeAccent, generated_theme_accent};

use iced::widget::{button, checkbox, column, row, space, text};
use iced::{Element, Length};

const SIDEBAR_WIDTH: f32 = 36.0;
const OBJECT_LIST_SIDEBAR_WIDTH: f32 = 196.0;
const DRAWING_STROKE_WIDTH_STEP: f32 = 0.5;
const DRAWING_FILL_ALPHA: f32 = 0.16;
const DRAWING_TOOLBAR_COLOR_WIDTH: f32 = 156.0;
const DRAWING_TOOLBAR_FILL_COLOR_WIDTH: f32 = 196.0;
const DRAWING_TOOLBAR_SLIDER_WIDTH: f32 = 140.0;
const DRAWING_TOOLBAR_TOGGLE_WIDTH: f32 = 112.0;
const DRAWING_TOOLBAR_DIVIDER_HEIGHT: f32 = 36.0;

fn drawing_color_picker<'a, Message, SelectColor>(
    label: &'static str,
    selected: Option<ThemeAccent>,
    alpha: f32,
    allow_none: bool,
    select_color: SelectColor,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
    SelectColor: Fn(ThemeAccent) -> DrawingMessage + Copy + 'a,
{
    let colors = ThemeAccent::ALL.into_iter().map(|accent| {
        color_swatch_button(
            move |theme: &iced::Theme| {
                generated_theme_accent(theme.extended_palette(), accent).scale_alpha(alpha)
            },
            selected == Some(accent),
            on_sidebar_message(select_color(accent)),
        )
    });
    let mut choices = row(colors).spacing(3);

    if allow_none {
        choices = choices.push(
            button(text("None").size(11))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .padding([2, 4])
                .on_press(on_sidebar_message(
                    DrawingMessage::SetSelectedDrawingFillColor(None),
                )),
        );
    }

    column![text(label).size(12), choices]
        .spacing(2)
        .width(Length::Fill)
        .into()
}

fn drawing_width_slider<'a, Message>(
    label: &'static str,
    current: f32,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    labeled_slider(
        label,
        MIN_DRAWING_STROKE_WIDTH..=MAX_DRAWING_STROKE_WIDTH,
        current,
        move |width| on_sidebar_message(DrawingMessage::SetSelectedDrawingStrokeWidth(width)),
        |value| format!("{value:.1}"),
        Some(DRAWING_STROKE_WIDTH_STEP),
    )
}

fn drawing_extend_checkbox<'a, Message>(
    checked: bool,
    label: &'static str,
    on_toggle: impl Fn(bool) -> DrawingMessage + Copy + 'a,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    row![
        checkbox(checked)
            .label(label)
            .on_toggle(move |value| on_sidebar_message(on_toggle(value))),
        space::horizontal(),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center)
    .width(Length::Fill)
    .into()
}

fn drawing_toolbar_divider<'a, Message>() -> Element<'a, Message>
where
    Message: 'a,
{
    iced::widget::container(iced::widget::rule::vertical(1.0).style(style::split_ruler))
        .height(DRAWING_TOOLBAR_DIVIDER_HEIGHT)
        .into()
}

#[derive(Debug, Clone)]
struct DraggedDrawing {
    id: DrawingId,
    origin_object: DrawingObject,
}

#[derive(Debug, Clone)]
struct DrawingDragState {
    controller_id: DrawingId,
    target: DrawingDragTarget,
    origin_anchor: DrawingAnchor,
    dragged: Vec<DraggedDrawing>,
}

#[derive(Debug, Clone)]
enum DrawingInteraction {
    Idle,
    Drafting(DrawingDraft),
    Dragging(DrawingDragState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingUpdate {
    None,
    StateOnly,
    Visual,
}

impl DrawingUpdate {
    pub fn should_bump(self) -> bool {
        matches!(self, Self::Visual)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawingMessage {
    SelectTool(DrawingTool),
    ShowToolsPage,
    ShowObjectListPage,
    ClearSelection,
    SelectDrawing(DrawingId),
    OpenDrawingSettings(DrawingId),
    SetSelectedDrawingStrokeColor(ThemeAccent),
    SetSelectedDrawingFillColor(Option<ThemeAccent>),
    SetSelectedDrawingStrokeWidth(f32),
    SetHorizontalLineRay(bool),
    SetBoxExtendRight(bool),
    DeleteSelectedDrawing,
    DeleteDrawing(DrawingId),
    DeleteAllDrawings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarPage {
    Tools,
    ObjectList,
}

#[derive(Debug, Clone)]
pub struct DrawingTools {
    active_tool: DrawingTool,
    sidebar_page: SidebarPage,
    drawings: Vec<DrawingEntity>,
    selected_drawings: Vec<DrawingId>,
    interaction: DrawingInteraction,
    next_drawing_id: u64,
    style_defaults: DrawingStyleDefaults,
    horizontal_line_ray: bool,
    box_extend_right: bool,
    measure: Option<DrawingMeasure>,
}

impl Default for DrawingTools {
    fn default() -> Self {
        Self {
            active_tool: DrawingTool::Cursor,
            sidebar_page: SidebarPage::Tools,
            drawings: Vec::new(),
            selected_drawings: Vec::new(),
            interaction: DrawingInteraction::Idle,
            next_drawing_id: 1,
            style_defaults: DrawingStyleDefaults::default(),
            horizontal_line_ray: false,
            box_extend_right: false,
            measure: None,
        }
    }
}

impl DrawingTools {
    pub fn from_parts(
        active_tool: DrawingTool,
        drawings: Vec<DrawingEntity>,
        style_defaults: DrawingStyleDefaults,
        horizontal_line_ray: bool,
        box_extend_right: bool,
    ) -> Self {
        let next_drawing_id = drawings
            .iter()
            .map(|drawing| drawing.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);

        Self {
            active_tool,
            sidebar_page: SidebarPage::Tools,
            drawings,
            selected_drawings: Vec::new(),
            interaction: DrawingInteraction::Idle,
            next_drawing_id,
            style_defaults,
            horizontal_line_ray,
            box_extend_right,
            measure: None,
        }
    }

    pub fn active_tool(&self) -> DrawingTool {
        self.active_tool
    }

    pub fn horizontal_line_ray(&self) -> bool {
        self.horizontal_line_ray
    }

    pub fn style_defaults(&self) -> DrawingStyleDefaults {
        self.style_defaults
    }

    pub fn box_extend_right(&self) -> bool {
        self.box_extend_right
    }

    pub fn drawings(&self) -> &[DrawingEntity] {
        &self.drawings
    }

    pub fn update(&mut self, message: DrawingMessage) -> DrawingUpdate {
        match message {
            DrawingMessage::SelectTool(tool) => {
                if self.select_tool(tool) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::ShowToolsPage => {
                self.selected_drawings.clear();

                if self.set_sidebar_page(SidebarPage::Tools) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::ShowObjectListPage => {
                if self.set_sidebar_page(SidebarPage::ObjectList) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::ClearSelection => {
                if self.select_drawing(None) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SelectDrawing(id) => {
                if self.select_drawing(Some(id)) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::OpenDrawingSettings(id) => {
                if self.open_drawing_settings(id) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingStrokeColor(color) => {
                if self.update_selected_style(|style| {
                    style.stroke_color = DrawingColor::accent(color, 1.0);
                }) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingFillColor(color) => {
                if self.update_selected_style(|style| {
                    style.fill_color =
                        color.map(|accent| DrawingColor::accent(accent, DRAWING_FILL_ALPHA));
                }) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingStrokeWidth(width) => {
                if self.update_selected_style(|style| {
                    style.set_stroke_width(width);
                }) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetHorizontalLineRay(ray) => {
                let default_changed = self.horizontal_line_ray != ray;
                self.horizontal_line_ray = ray;
                let object_changed = self.update_selected_object(|object| {
                    if let DrawingObject::HorizontalLine { ray: current, .. } = object {
                        *current = ray;
                    }
                });
                if default_changed || object_changed {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetBoxExtendRight(extend) => {
                let default_changed = self.box_extend_right != extend;
                self.box_extend_right = extend;
                let object_changed = self.update_selected_object(|object| {
                    if let DrawingObject::Box {
                        extend_right: current,
                        ..
                    } = object
                    {
                        *current = extend;
                    }
                });
                if default_changed || object_changed {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteSelectedDrawing => {
                if self.remove_selected_drawing() {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteDrawing(id) => {
                if self.remove_drawing(id) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteAllDrawings => {
                if self.remove_all_drawings() {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
        }
    }

    pub fn sidebar_view<'a, Message>(
        &'a self,
        composition: &'a ChartComposition,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a + 'static,
    {
        match self.sidebar_page {
            SidebarPage::Tools => self.view_tools_sidebar(on_sidebar_message),
            SidebarPage::ObjectList => {
                self.view_object_list_sidebar(composition, on_sidebar_message)
            }
        }
    }

    pub fn selected_toolbar_view<'a, Message>(
        &'a self,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> Option<Element<'a, Message>>
    where
        Message: Clone + 'a + 'static,
    {
        matches!(self.sidebar_page, SidebarPage::Tools)
            .then(|| self.selected_entity())
            .flatten()
            .map(|drawing| {
                self.view_selected_toolbar(drawing, on_sidebar_message)
                    .into()
            })
    }

    fn view_selected_toolbar<'a, Message>(
        &'a self,
        drawing: &DrawingEntity,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a + 'static,
    {
        let mut controls = iced::widget::row![
            iced::widget::text(drawing.object.title()).size(13),
            drawing_toolbar_divider(),
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center);

        let stroke_width = drawing.style.normalized_stroke_width();

        match drawing.object {
            DrawingObject::Box { extend_right, .. } => {
                controls = controls
                    .push(
                        iced::widget::container(drawing_color_picker(
                            "Border color",
                            drawing.style.stroke_color.accent_value(),
                            1.0,
                            false,
                            DrawingMessage::SetSelectedDrawingStrokeColor,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_COLOR_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_color_picker(
                            "Fill color",
                            drawing
                                .style
                                .fill_color
                                .and_then(|color| color.accent_value()),
                            DRAWING_FILL_ALPHA,
                            true,
                            |color| DrawingMessage::SetSelectedDrawingFillColor(Some(color)),
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_FILL_COLOR_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_width_slider(
                            "Border width",
                            stroke_width,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_SLIDER_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_extend_checkbox(
                            extend_right,
                            "Extend right",
                            DrawingMessage::SetBoxExtendRight,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_TOGGLE_WIDTH),
                    );
            }
            DrawingObject::HorizontalLine { ray, .. } => {
                controls = controls
                    .push(
                        iced::widget::container(drawing_color_picker(
                            "Line color",
                            drawing.style.stroke_color.accent_value(),
                            1.0,
                            false,
                            DrawingMessage::SetSelectedDrawingStrokeColor,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_COLOR_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_width_slider(
                            "Line width",
                            stroke_width,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_SLIDER_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_extend_checkbox(
                            ray,
                            "Ray",
                            DrawingMessage::SetHorizontalLineRay,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_TOGGLE_WIDTH),
                    );
            }
            DrawingObject::Trendline { .. } | DrawingObject::VerticalLine { .. } => {
                controls = controls
                    .push(
                        iced::widget::container(drawing_color_picker(
                            "Line color",
                            drawing.style.stroke_color.accent_value(),
                            1.0,
                            false,
                            DrawingMessage::SetSelectedDrawingStrokeColor,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_COLOR_WIDTH),
                    )
                    .push(
                        iced::widget::container(drawing_width_slider(
                            "Line width",
                            stroke_width,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_SLIDER_WIDTH),
                    );
            }
        }

        controls = controls
            .push(drawing_toolbar_divider())
            .push(
                iced::widget::button(style::icon_text(style::Icon::TrashBin, 12))
                    .style(|theme, status| style::button::cancel(theme, status, false))
                    .padding([4, 6])
                    .on_press(on_sidebar_message(DrawingMessage::DeleteSelectedDrawing)),
            )
            .push(
                iced::widget::button(style::icon_text(style::Icon::Close, 12))
                    .style(|theme, status| style::button::transparent(theme, status, false))
                    .padding([4, 6])
                    .on_press(on_sidebar_message(DrawingMessage::ClearSelection)),
            );

        iced::widget::container(controls)
            .height(Length::Fill)
            .padding([0, 4])
            .align_y(iced::alignment::Vertical::Center)
    }

    fn view_object_row<'a, Message>(
        &'a self,
        drawing: &DrawingEntity,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Row<'a, Message>
    where
        Message: Clone + 'a + 'static,
    {
        let selected = self.selected_drawings.contains(&drawing.id);
        let object_label = format!("{} #{}", drawing.object.title(), drawing.id.0);

        let select_button = iced::widget::button(iced::widget::text(object_label).size(12))
            .style(move |theme, status| style::button::transparent(theme, status, selected))
            .width(iced::Length::Fill)
            .on_press(on_sidebar_message(DrawingMessage::SelectDrawing(
                drawing.id,
            )));

        let settings_button = iced::widget::button(style::icon_text(style::Icon::Cog, 12))
            .style(|theme, status| style::button::transparent(theme, status, false))
            .padding([2, 6])
            .on_press(on_sidebar_message(DrawingMessage::OpenDrawingSettings(
                drawing.id,
            )));

        let remove_button = iced::widget::button(style::icon_text(style::Icon::TrashBin, 12))
            .style(|theme, status| style::button::cancel(theme, status, false))
            .padding([2, 6]);

        let remove_button = if drawing.locked {
            remove_button
        } else {
            remove_button.on_press(on_sidebar_message(DrawingMessage::DeleteDrawing(
                drawing.id,
            )))
        };

        iced::widget::row![select_button, settings_button, remove_button]
            .spacing(4)
            .align_y(iced::Alignment::Center)
            .width(iced::Length::Fill)
    }

    fn view_object_list_sidebar<'a, Message>(
        &'a self,
        composition: &'a ChartComposition,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a + 'static,
    {
        let header_nav = row![
            iced::widget::space::horizontal(),
            iced::widget::button(style::icon_text(style::Icon::Return, 12))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .on_press(on_sidebar_message(DrawingMessage::ShowToolsPage))
        ];

        let mut content = iced::widget::Column::new()
            .spacing(4)
            .push(header_nav)
            .push(iced::widget::rule::horizontal(1.0));

        if self.drawings.is_empty() {
            content = content.push(iced::widget::text("No drawings").size(12));
        } else {
            let mut grouped: Vec<(Option<PanelId>, Vec<&DrawingEntity>)> = Vec::new();

            for drawing in &self.drawings {
                let panel_id = drawing.object.panel_id();

                if let Some((_, entries)) = grouped
                    .iter_mut()
                    .find(|(group_panel_id, _)| *group_panel_id == panel_id)
                {
                    entries.push(drawing);
                } else {
                    grouped.push((panel_id, vec![drawing]));
                }
            }

            grouped.sort_by_key(|(panel_id, _)| panel_id.map(|id| id.0).unwrap_or(u32::MAX));

            for (panel_id, drawings) in grouped {
                content = content.push(
                    iced::widget::text(Self::panel_group_title(composition, panel_id)).size(12),
                );

                for drawing in drawings {
                    content = content.push(self.view_object_row(drawing, on_sidebar_message));
                }

                content = content.push(iced::widget::rule::horizontal(1.0));
            }
        }

        let remove_all_button =
            iced::widget::button(iced::widget::text("Remove All").align_x(iced::Alignment::Center))
                .style(|theme, status| style::button::cancel(theme, status, false))
                .width(iced::Length::Fill);

        content = content.push(if self.drawings.is_empty() {
            remove_all_button
        } else {
            remove_all_button.on_press(on_sidebar_message(DrawingMessage::DeleteAllDrawings))
        });

        iced::widget::container(iced::widget::scrollable(content).height(iced::Length::Fill))
            .style(style::chart_sidebar_container)
            .width(OBJECT_LIST_SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 6])
    }

    fn view_tools_sidebar<'a, Message>(
        &'a self,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a,
    {
        let mut tools = iced::widget::Column::new()
            .spacing(6)
            .height(iced::Length::Fill);

        for tool in DrawingTool::all().iter().copied() {
            let selected = self.active_tool == tool;
            let label = tool.short_label().to_string();

            let btn =
                iced::widget::button(iced::widget::text(label).align_x(iced::Alignment::Center))
                    .style(move |theme, status| style::button::transparent(theme, status, selected))
                    .width(iced::Length::Fill);

            tools = tools.push(if selected {
                btn
            } else {
                btn.on_press(on_sidebar_message(DrawingMessage::SelectTool(tool)))
            });
        }

        tools = tools.push(iced::widget::space::vertical());

        tools = tools.push(
            iced::widget::button(iced::widget::text("O").align_x(iced::Alignment::Center))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .width(iced::Length::Fill)
                .on_press(on_sidebar_message(DrawingMessage::ShowObjectListPage)),
        );

        iced::widget::container(tools)
            .style(style::chart_sidebar_container)
            .width(SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 2])
    }

    pub fn snapshot(&self) -> DrawingSnapshot<'_> {
        DrawingSnapshot::new(
            self.active_tool,
            &self.drawings,
            &self.selected_drawings,
            self.drawing_draft(),
            self.measure,
        )
    }

    pub fn handle_widget_drawing_event(&mut self, event: KlineWidgetDrawingEvent) -> DrawingUpdate {
        match event {
            KlineWidgetDrawingEvent::Selected(selected) => {
                if self.select_drawing(selected) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::MarqueeSelected(selected) => {
                if self.select_drawings(selected) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::AnchorPressed(anchor) => {
                if self.drawing_draft().is_some() {
                    if self.commit_drawing_draft(anchor).is_some() {
                        self.active_tool = DrawingTool::Cursor;
                        DrawingUpdate::Visual
                    } else {
                        DrawingUpdate::None
                    }
                } else {
                    let started = self.start_drawing_from_anchor(anchor);
                    if started && self.drawing_draft().is_none() {
                        self.active_tool = DrawingTool::Cursor;
                    }

                    if started {
                        DrawingUpdate::Visual
                    } else {
                        DrawingUpdate::None
                    }
                }
            }
            KlineWidgetDrawingEvent::AnchorMoved(anchor) => {
                if self.update_drawing_draft_anchor(anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DragStarted {
                id,
                target,
                operation,
                anchor,
            } => {
                if self.start_drawing_drag(id, target, operation, anchor) {
                    match operation {
                        DrawingDragOperation::Move => DrawingUpdate::StateOnly,
                        DrawingDragOperation::Copy => DrawingUpdate::Visual,
                    }
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DragMoved { id, target, anchor } => {
                if self.update_drawing_drag(id, target, anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DragFinished { id } => {
                if self.finish_drawing_drag(id) {
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::MeasureClick(anchor) => {
                if self.measure_click(anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::MeasureMove(anchor) => {
                if self.measure_move(anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::MeasureCanceled => {
                if self.measure.take().is_some() {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DraftCanceled => {
                if self.cancel_drawing_draft() {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
        }
    }

    pub fn drawing_draft(&self) -> Option<&DrawingDraft> {
        match &self.interaction {
            DrawingInteraction::Drafting(draft) => Some(draft),
            DrawingInteraction::Idle | DrawingInteraction::Dragging(_) => None,
        }
    }

    pub fn select_drawing(&mut self, selected: Option<DrawingId>) -> bool {
        let ids = selected.into_iter().collect::<Vec<_>>();
        self.select_drawings(ids)
    }

    pub fn select_drawings(&mut self, selected: Vec<DrawingId>) -> bool {
        if selected
            .iter()
            .any(|id| !self.drawings.iter().any(|drawing| drawing.id == *id))
        {
            return false;
        }

        if self.selected_drawings == selected {
            return false;
        }

        self.selected_drawings = selected;
        self.clear_drag();
        true
    }

    pub fn remove_selected_drawing(&mut self) -> bool {
        let ids = std::mem::take(&mut self.selected_drawings);
        if ids.is_empty() {
            return false;
        }

        let mut changed = false;
        for id in ids {
            changed |= self.remove_drawing(id);
        }
        changed
    }

    fn single_selected_index(&self) -> Option<usize> {
        if self.selected_drawings.len() == 1 {
            self.selected_drawings
                .first()
                .and_then(|id| self.drawing_index(*id))
        } else {
            None
        }
    }

    fn update_selected_style(&mut self, update: impl FnOnce(&mut DrawingStyle)) -> bool {
        let Some(index) = self.single_selected_index() else {
            return false;
        };

        if self.drawings[index].locked {
            return false;
        }

        let drawing = &mut self.drawings[index];
        let before = drawing.style;
        update(&mut drawing.style);
        if drawing.style == before {
            return false;
        }

        self.style_defaults
            .set_style(drawing.object.tool(), drawing.style);
        true
    }

    fn update_selected_object(&mut self, update: impl FnOnce(&mut DrawingObject)) -> bool {
        let Some(index) = self.single_selected_index() else {
            return false;
        };

        if self.drawings[index].locked {
            return false;
        }

        let before = self.drawings[index].object.clone();
        update(&mut self.drawings[index].object);
        self.drawings[index].object != before
    }

    pub fn remove_drawing(&mut self, id: DrawingId) -> bool {
        let Some(index) = self.drawings.iter().position(|drawing| drawing.id == id) else {
            return false;
        };

        if self.drawings[index].locked {
            return false;
        }

        self.drawings.remove(index);

        self.selected_drawings.retain(|selected| *selected != id);

        if matches!(self.interaction, DrawingInteraction::Dragging(ref drag) if drag.dragged.iter().any(|d| d.id == id))
        {
            self.interaction = DrawingInteraction::Idle;
        }

        true
    }

    pub fn remove_all_drawings(&mut self) -> bool {
        if self.drawings.is_empty()
            && matches!(self.interaction, DrawingInteraction::Idle)
            && self.selected_drawings.is_empty()
        {
            return false;
        }

        self.drawings.clear();
        self.selected_drawings.clear();
        self.interaction = DrawingInteraction::Idle;
        true
    }

    pub fn start_drawing_from_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        match self.active_tool {
            DrawingTool::Cursor | DrawingTool::Measure => false,
            DrawingTool::Trendline => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Trendline {
                    start: anchor,
                    current: anchor,
                    style: self.style_defaults.style_for(DrawingTool::Trendline),
                });
                self.selected_drawings.clear();
                true
            }
            DrawingTool::Box => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Box {
                    start: anchor,
                    current: anchor,
                    style: self.style_defaults.style_for(DrawingTool::Box),
                    extend_right: self.box_extend_right,
                });
                self.selected_drawings.clear();
                true
            }
            DrawingTool::HorizontalLine => {
                let object = DrawingObject::HorizontalLine {
                    panel_id: anchor.panel_id,
                    y_unit: anchor.y_unit,
                    time: anchor.time,
                    ray: self.horizontal_line_ray,
                };
                self.push_drawing(
                    object,
                    self.style_defaults.style_for(DrawingTool::HorizontalLine),
                );
                true
            }
            DrawingTool::VerticalLine => {
                let object = DrawingObject::VerticalLine { time: anchor.time };
                self.push_drawing(
                    object,
                    self.style_defaults.style_for(DrawingTool::VerticalLine),
                );
                true
            }
        }
    }

    pub fn update_drawing_draft_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        let DrawingInteraction::Drafting(draft) = &mut self.interaction else {
            return false;
        };

        match draft {
            DrawingDraft::Trendline { start, current, .. }
            | DrawingDraft::Box { start, current, .. } => {
                if anchor.panel_id != start.panel_id {
                    return false;
                }

                *current = anchor;
            }
        }

        true
    }

    pub fn commit_drawing_draft(&mut self, end: DrawingAnchor) -> Option<DrawingId> {
        let draft = self.take_draft()?;
        let (object, style) = draft.try_commit(end)?;

        Some(self.push_drawing(object, style))
    }

    pub fn cancel_drawing_draft(&mut self) -> bool {
        if !matches!(self.interaction, DrawingInteraction::Drafting(_)) {
            return false;
        }

        self.interaction = DrawingInteraction::Idle;
        true
    }

    pub fn prune_panel_drawings(&mut self, panel_id: PanelId) -> bool {
        let before = self.drawings.len();
        self.drawings
            .retain(|drawing| !drawing.belongs_to_panel(panel_id));

        let drawings_pruned = before != self.drawings.len();

        let mut changed = drawings_pruned;

        let selection_pruned = !self.selected_drawings.is_empty()
            && self
                .selected_drawings
                .iter()
                .any(|id| self.drawing_index(*id).is_none());
        if selection_pruned {
            let kept = self
                .selected_drawings
                .iter()
                .filter(|id| self.drawing_index(**id).is_some())
                .copied()
                .collect();
            self.selected_drawings = kept;
            changed = true;
        }

        if self
            .active_drag_any()
            .map(|drag| {
                drag.dragged
                    .iter()
                    .any(|d| self.drawing_index(d.id).is_none())
            })
            .unwrap_or(false)
        {
            self.interaction = DrawingInteraction::Idle;
            changed = true;
        }

        if self
            .drawing_draft()
            .map(|draft| draft.belongs_to_panel(panel_id))
            .unwrap_or(false)
        {
            self.interaction = DrawingInteraction::Idle;
            changed = true;
        }

        if self
            .measure
            .map(|measure| measure.state().panel_id == panel_id)
            .unwrap_or(false)
        {
            self.measure = None;
            changed = true;
        }

        changed
    }

    fn measure_click(&mut self, anchor: DrawingAnchor) -> bool {
        match self.measure {
            None => {
                self.measure = Some(DrawingMeasure::Pending(MeasureState {
                    panel_id: anchor.panel_id,
                    start: anchor,
                    end: anchor,
                }));
                true
            }
            Some(DrawingMeasure::Pending(state)) => {
                self.measure = Some(DrawingMeasure::Committed(MeasureState {
                    panel_id: state.panel_id,
                    start: state.start,
                    end: anchor,
                }));
                if self.active_tool == DrawingTool::Measure {
                    self.active_tool = DrawingTool::Cursor;
                }
                true
            }
            Some(DrawingMeasure::Committed(_)) => {
                self.measure = None;
                true
            }
        }
    }

    fn measure_move(&mut self, anchor: DrawingAnchor) -> bool {
        let Some(DrawingMeasure::Pending(state)) = self.measure else {
            return false;
        };

        self.measure = Some(DrawingMeasure::Pending(MeasureState {
            panel_id: state.panel_id,
            start: state.start,
            end: anchor,
        }));
        true
    }

    fn open_drawing_settings(&mut self, id: DrawingId) -> bool {
        let mut changed = false;
        changed |= self.select_drawing(Some(id));
        changed |= self.set_sidebar_page(SidebarPage::Tools);
        changed
    }

    fn selected_entity(&self) -> Option<&DrawingEntity> {
        let id = self.single_selected_index()?;
        self.drawings.get(id)
    }

    fn set_sidebar_page(&mut self, page: SidebarPage) -> bool {
        if self.sidebar_page == page {
            return false;
        }

        self.sidebar_page = page;
        true
    }

    fn panel_group_title(composition: &ChartComposition, panel_id: Option<PanelId>) -> String {
        match panel_id {
            Some(panel_id) => {
                let Some(panel) = composition.panel(panel_id) else {
                    return format!("Panel {}", panel_id.0);
                };

                if let Some(title) = panel.title.as_deref()
                    && !title.is_empty()
                {
                    return title.to_string();
                }

                if matches!(panel.role, PanelRole::Primary) {
                    "Main".to_string()
                } else {
                    format!("Panel {}", panel_id.0)
                }
            }
            None => "All Panels".to_string(),
        }
    }

    fn start_drawing_drag(
        &mut self,
        id: DrawingId,
        target: DrawingDragTarget,
        operation: DrawingDragOperation,
        anchor: DrawingAnchor,
    ) -> bool {
        let Some(index) = self.drawing_index(id) else {
            self.clear_drag();
            return false;
        };

        let drawing = &self.drawings[index];
        if drawing.locked || !drawing.visible {
            self.clear_drag();
            return false;
        }

        let dragged = if matches!(target, DrawingDragTarget::Translate) {
            let mut ids = vec![id];
            for selected in &self.selected_drawings {
                if *selected == id {
                    continue;
                }
                if let Some(index) = self.drawing_index(*selected)
                    && self.drawings[index].object.panel_id() == Some(anchor.panel_id)
                {
                    ids.push(*selected);
                }
            }
            ids
        } else {
            vec![id]
        };

        let mut dragged = dragged
            .into_iter()
            .filter_map(|dragged_id| {
                self.drawing_index(dragged_id).and_then(|index| {
                    let drawing = &self.drawings[index];
                    (!drawing.locked && drawing.visible).then(|| DraggedDrawing {
                        id: dragged_id,
                        origin_object: drawing.object.clone(),
                    })
                })
            })
            .collect::<Vec<_>>();

        if dragged.is_empty() {
            self.clear_drag();
            return false;
        }

        if matches!(operation, DrawingDragOperation::Copy) {
            for dragged in &mut dragged {
                let Some(index) = self.drawing_index(dragged.id) else {
                    self.clear_drag();
                    return false;
                };
                let mut copy = self.drawings[index].clone();
                let copy_id = self.allocate_drawing_id();
                copy.id = copy_id;
                self.drawings.push(copy);
                dragged.id = copy_id;
            }
        }

        self.selected_drawings = dragged.iter().map(|d| d.id).collect();
        self.interaction = DrawingInteraction::Dragging(DrawingDragState {
            controller_id: id,
            target,
            origin_anchor: anchor,
            dragged,
        });
        true
    }

    fn update_drawing_drag(
        &mut self,
        id: DrawingId,
        target: DrawingDragTarget,
        anchor: DrawingAnchor,
    ) -> bool {
        let Some(drag_state) = self.active_drag(id).cloned() else {
            return false;
        };

        if drag_state.target != target {
            return false;
        }

        if matches!(
            target,
            DrawingDragTarget::Translate | DrawingDragTarget::Handle(_)
        ) && anchor.panel_id != drag_state.origin_anchor.panel_id
        {
            return false;
        }

        let mut changed = false;

        match target {
            DrawingDragTarget::Translate => {
                for dragged in &drag_state.dragged {
                    let Some(index) = self.drawing_index(dragged.id) else {
                        self.clear_drag();
                        return false;
                    };
                    if self.drawings[index].locked {
                        self.clear_drag();
                        return false;
                    }
                    let moved = dragged
                        .origin_object
                        .translated(drag_state.origin_anchor, anchor);
                    if self.drawings[index].object != moved {
                        self.drawings[index].object = moved;
                        changed = true;
                    }
                }
            }
            DrawingDragTarget::Handle(handle) => {
                let Some(primary) = drag_state.dragged.iter().find(|d| d.id == id) else {
                    self.clear_drag();
                    return false;
                };
                let Some(index) = self.drawing_index(id) else {
                    self.clear_drag();
                    return false;
                };
                if self.drawings[index].locked {
                    self.clear_drag();
                    return false;
                }
                let Some(edited) = primary.origin_object.handle_dragged(handle, anchor) else {
                    return false;
                };
                if self.drawings[index].object != edited {
                    self.drawings[index].object = edited;
                    changed = true;
                }
            }
        }

        if changed {
            self.selected_drawings = drag_state.dragged.iter().map(|d| d.id).collect();
        }
        changed
    }

    fn finish_drawing_drag(&mut self, id: DrawingId) -> bool {
        if matches!(self.interaction, DrawingInteraction::Dragging(ref drag) if drag.controller_id == id)
        {
            self.interaction = DrawingInteraction::Idle;
            true
        } else {
            false
        }
    }

    fn select_tool(&mut self, tool: DrawingTool) -> bool {
        if self.active_tool == tool {
            return false;
        }

        self.active_tool = tool;
        self.interaction = DrawingInteraction::Idle;
        self.measure = None;
        true
    }

    fn drawing_index(&self, id: DrawingId) -> Option<usize> {
        self.drawings.iter().position(|drawing| drawing.id == id)
    }

    fn active_drag(&self, id: DrawingId) -> Option<&DrawingDragState> {
        match &self.interaction {
            DrawingInteraction::Dragging(drag) if drag.controller_id == id => Some(drag),
            DrawingInteraction::Idle
            | DrawingInteraction::Drafting(_)
            | DrawingInteraction::Dragging(_) => None,
        }
    }

    fn active_drag_any(&self) -> Option<&DrawingDragState> {
        match &self.interaction {
            DrawingInteraction::Dragging(drag) => Some(drag),
            DrawingInteraction::Idle | DrawingInteraction::Drafting(_) => None,
        }
    }

    fn clear_drag(&mut self) -> bool {
        if !matches!(self.interaction, DrawingInteraction::Dragging(_)) {
            return false;
        }

        self.interaction = DrawingInteraction::Idle;
        true
    }

    fn take_draft(&mut self) -> Option<DrawingDraft> {
        match std::mem::replace(&mut self.interaction, DrawingInteraction::Idle) {
            DrawingInteraction::Drafting(draft) => Some(draft),
            interaction => {
                self.interaction = interaction;
                None
            }
        }
    }

    fn push_drawing(&mut self, object: DrawingObject, style: DrawingStyle) -> DrawingId {
        let id = self.allocate_drawing_id();

        self.drawings.push(DrawingEntity {
            id,
            object,
            style,
            locked: false,
            visible: true,
        });

        self.selected_drawings = vec![id];
        self.interaction = DrawingInteraction::Idle;
        id
    }

    fn allocate_drawing_id(&mut self) -> DrawingId {
        let id = DrawingId(self.next_drawing_id.max(1));
        self.next_drawing_id = self.next_drawing_id.wrapping_add(1).max(1);
        id
    }
}

impl DrawingTools {
    pub(super) const fn compact_sidebar_width() -> f32 {
        SIDEBAR_WIDTH
    }

    pub(super) fn sidebar_width(&self) -> f32 {
        match self.sidebar_page {
            SidebarPage::Tools => SIDEBAR_WIDTH,
            SidebarPage::ObjectList => OBJECT_LIST_SIDEBAR_WIDTH,
        }
    }

    pub fn drawing_panels_exist(composition: &ChartComposition, object: &DrawingObject) -> bool {
        let panel_exists = |panel_id: PanelId| composition.panel(panel_id).is_some();

        match object {
            DrawingObject::Trendline { start, end } | DrawingObject::Box { start, end, .. } => {
                panel_exists(start.panel_id) && panel_exists(end.panel_id)
            }
            DrawingObject::HorizontalLine { panel_id, .. } => panel_exists(*panel_id),
            DrawingObject::VerticalLine { .. } => true,
        }
    }
}
