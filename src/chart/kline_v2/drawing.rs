use crate::style;
use crate::widget::chart::kline::composition::{ChartComposition, PanelId, PanelRole};
use crate::widget::chart::kline::drawing::{
    DrawingAnchor, DrawingDraft, DrawingDragTarget, DrawingEntity, DrawingId, DrawingObject,
    DrawingStyle, DrawingTool,
};
use crate::widget::chart::kline::{KlineWidgetDrawingEvent, KlineWidgetEvent};

use iced::widget::row;

const SIDEBAR_WIDTH: f32 = 36.0;
const DETAILS_SIDEBAR_WIDTH: f32 = 108.0;
const OBJECT_LIST_SIDEBAR_WIDTH: f32 = 196.0;

#[derive(Debug, Clone)]
struct DrawingDragState {
    id: DrawingId,
    target: DrawingDragTarget,
    origin_anchor: DrawingAnchor,
    origin_object: DrawingObject,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawingMessage {
    SelectTool(DrawingTool),
    ShowToolsPage,
    ShowObjectListPage,
    SelectDrawing(DrawingId),
    OpenDrawingSettings(DrawingId),
    SelectedDrawingBorderColor,
    SelectedDrawingFillColor,
    SelectedDrawingBorderWidth,
    SelectedDrawingLineColor,
    SelectedDrawingLineWidth,
    DeleteSelectedDrawing,
    DeleteDrawing(DrawingId),
    DeleteAllDrawings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidebarPage {
    Tools,
    ObjectList,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Event {
    Selected(Option<DrawingId>),
    AnchorPressed(DrawingAnchor),
    AnchorMoved(DrawingAnchor),
    DragStarted {
        id: DrawingId,
        target: DrawingDragTarget,
        anchor: DrawingAnchor,
    },
    DragMoved {
        id: DrawingId,
        target: DrawingDragTarget,
        anchor: DrawingAnchor,
    },
    DragFinished {
        id: DrawingId,
    },
    DraftCanceled,
}

impl Event {
    pub fn from_kline_widget_event(event: &KlineWidgetEvent) -> Option<Self> {
        match event {
            KlineWidgetEvent::Drawing(event) => Some(Self::from_widget_drawing_event(*event)),
            _ => None,
        }
    }

    fn from_widget_drawing_event(event: KlineWidgetDrawingEvent) -> Self {
        match event {
            KlineWidgetDrawingEvent::Selected(selected) => Self::Selected(selected),
            KlineWidgetDrawingEvent::AnchorPressed(anchor) => Self::AnchorPressed(anchor),
            KlineWidgetDrawingEvent::AnchorMoved(anchor) => Self::AnchorMoved(anchor),
            KlineWidgetDrawingEvent::DragStarted { id, target, anchor } => {
                Self::DragStarted { id, target, anchor }
            }
            KlineWidgetDrawingEvent::DragMoved { id, target, anchor } => {
                Self::DragMoved { id, target, anchor }
            }
            KlineWidgetDrawingEvent::DragFinished { id } => Self::DragFinished { id },
            KlineWidgetDrawingEvent::DraftCanceled => Self::DraftCanceled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawingTools {
    active_tool: DrawingTool,
    sidebar_page: SidebarPage,
    drawings: Vec<DrawingEntity>,
    selected_drawing: Option<DrawingId>,
    interaction: DrawingInteraction,
    next_drawing_id: u64,
}

impl Default for DrawingTools {
    fn default() -> Self {
        Self {
            active_tool: DrawingTool::Cursor,
            sidebar_page: SidebarPage::Tools,
            drawings: Vec::new(),
            selected_drawing: None,
            interaction: DrawingInteraction::Idle,
            next_drawing_id: 1,
        }
    }
}

impl DrawingTools {
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
                self.selected_drawing = None;

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
            DrawingMessage::SelectedDrawingBorderColor
            | DrawingMessage::SelectedDrawingFillColor
            | DrawingMessage::SelectedDrawingBorderWidth
            | DrawingMessage::SelectedDrawingLineColor
            | DrawingMessage::SelectedDrawingLineWidth => DrawingUpdate::None,
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

    pub fn view<'a, Message>(
        &'a self,
        composition: &'a ChartComposition,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a,
    {
        match self.sidebar_page {
            SidebarPage::Tools => {
                if let Some(drawing) = self.selected_entity() {
                    self.view_selected_sidebar(drawing, on_sidebar_message)
                } else {
                    self.view_tools_sidebar(on_sidebar_message)
                }
            }
            SidebarPage::ObjectList => {
                self.view_object_list_sidebar(composition, on_sidebar_message)
            }
        }
    }

    fn view_selected_sidebar<'a, Message>(
        &'a self,
        drawing: &DrawingEntity,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a,
    {
        let mut details = iced::widget::Column::new()
            .spacing(4)
            .height(iced::Length::Fill)
            .push(iced::widget::text(drawing.object.title()).size(13))
            .push(iced::widget::rule::horizontal(1.0))
            .align_x(iced::Alignment::Center);

        match drawing.object {
            DrawingObject::Box { .. } => {
                details = details
                    .push(
                        iced::widget::button(
                            iced::widget::text("Border Color").align_x(iced::Alignment::Center),
                        )
                        .style(|theme, status| style::button::transparent(theme, status, false))
                        .width(iced::Length::Fill)
                        .on_press(on_sidebar_message(
                            DrawingMessage::SelectedDrawingBorderColor,
                        )),
                    )
                    .push(
                        iced::widget::button(
                            iced::widget::text("Fill Color").align_x(iced::Alignment::Center),
                        )
                        .style(|theme, status| style::button::transparent(theme, status, false))
                        .width(iced::Length::Fill)
                        .on_press(on_sidebar_message(DrawingMessage::SelectedDrawingFillColor)),
                    )
                    .push(
                        iced::widget::button(
                            iced::widget::text("Border Width").align_x(iced::Alignment::Center),
                        )
                        .style(|theme, status| style::button::transparent(theme, status, false))
                        .width(iced::Length::Fill)
                        .on_press(on_sidebar_message(
                            DrawingMessage::SelectedDrawingBorderWidth,
                        )),
                    );
            }
            DrawingObject::Trendline { .. }
            | DrawingObject::HorizontalLine { .. }
            | DrawingObject::VerticalLine { .. } => {
                details = details
                    .push(
                        iced::widget::button(
                            iced::widget::text("Line Color").align_x(iced::Alignment::Center),
                        )
                        .style(|theme, status| style::button::transparent(theme, status, false))
                        .width(iced::Length::Fill)
                        .on_press(on_sidebar_message(DrawingMessage::SelectedDrawingLineColor)),
                    )
                    .push(
                        iced::widget::button(
                            iced::widget::text("Line Width").align_x(iced::Alignment::Center),
                        )
                        .style(|theme, status| style::button::transparent(theme, status, false))
                        .width(iced::Length::Fill)
                        .on_press(on_sidebar_message(DrawingMessage::SelectedDrawingLineWidth)),
                    );
            }
        }

        details = details.push(
            iced::widget::button(iced::widget::text("Delete").align_x(iced::Alignment::Center))
                .style(|theme, status| style::button::cancel(theme, status, false))
                .width(iced::Length::Fill)
                .on_press(on_sidebar_message(DrawingMessage::DeleteSelectedDrawing)),
        );

        iced::widget::container(details)
            .style(style::chart_sidebar_container)
            .width(DETAILS_SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 6])
    }

    fn view_object_row<'a, Message>(
        &'a self,
        drawing: &DrawingEntity,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Row<'a, Message>
    where
        Message: Clone + 'a,
    {
        let selected = self.selected_drawing == Some(drawing.id);
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
        Message: Clone + 'a,
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

    pub fn handle_kline_widget_event(&mut self, event: &KlineWidgetEvent) -> Option<DrawingUpdate> {
        let update = match Event::from_kline_widget_event(event)? {
            Event::Selected(selected) => {
                if self.select_drawing(selected) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            Event::AnchorPressed(anchor) => {
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
            Event::AnchorMoved(anchor) => {
                if self.update_drawing_draft_anchor(anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            Event::DragStarted { id, target, anchor } => {
                if self.start_drawing_drag(id, target, anchor) {
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            Event::DragMoved { id, target, anchor } => {
                if self.update_drawing_drag(id, target, anchor) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            Event::DragFinished { id } => {
                if self.finish_drawing_drag(id) {
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            Event::DraftCanceled => {
                if self.cancel_drawing_draft() {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
        };
        Some(update)
    }

    pub fn active_tool(&self) -> DrawingTool {
        self.active_tool
    }

    pub fn drawings(&self) -> &[DrawingEntity] {
        &self.drawings
    }

    pub fn selected_drawing(&self) -> Option<DrawingId> {
        self.selected_drawing
    }

    pub fn drawing_draft(&self) -> Option<&DrawingDraft> {
        match &self.interaction {
            DrawingInteraction::Drafting(draft) => Some(draft),
            DrawingInteraction::Idle | DrawingInteraction::Dragging(_) => None,
        }
    }

    pub fn select_drawing(&mut self, selected: Option<DrawingId>) -> bool {
        if let Some(id) = selected
            && !self.drawings.iter().any(|drawing| drawing.id == id)
        {
            return false;
        }

        if self.selected_drawing == selected {
            return false;
        }

        self.selected_drawing = selected;
        self.clear_drag();
        true
    }

    pub fn remove_selected_drawing(&mut self) -> bool {
        let Some(id) = self.selected_drawing else {
            return false;
        };

        self.remove_drawing(id)
    }

    pub fn remove_drawing(&mut self, id: DrawingId) -> bool {
        let Some(index) = self.drawings.iter().position(|drawing| drawing.id == id) else {
            return false;
        };

        if self.drawings[index].locked {
            return false;
        }

        self.drawings.remove(index);

        if self.selected_drawing == Some(id) {
            self.selected_drawing = None;
        }

        if matches!(self.interaction, DrawingInteraction::Dragging(ref drag) if drag.id == id) {
            self.interaction = DrawingInteraction::Idle;
        }

        true
    }

    pub fn remove_all_drawings(&mut self) -> bool {
        if self.drawings.is_empty()
            && matches!(self.interaction, DrawingInteraction::Idle)
            && self.selected_drawing.is_none()
        {
            return false;
        }

        self.drawings.clear();
        self.selected_drawing = None;
        self.interaction = DrawingInteraction::Idle;
        true
    }

    pub fn start_drawing_from_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        match self.active_tool {
            DrawingTool::Cursor => false,
            DrawingTool::Trendline => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Trendline {
                    start: anchor,
                    current: anchor,
                    style: DrawingTool::Trendline.default_style(),
                });
                self.selected_drawing = None;
                true
            }
            DrawingTool::Box => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Box {
                    start: anchor,
                    current: anchor,
                    style: DrawingTool::Box.default_style(),
                });
                self.selected_drawing = None;
                true
            }
            DrawingTool::HorizontalLine => {
                let object = DrawingObject::HorizontalLine {
                    panel_id: anchor.panel_id,
                    y_unit: anchor.y_unit,
                };
                self.push_drawing(object, DrawingTool::HorizontalLine.default_style());
                true
            }
            DrawingTool::VerticalLine => {
                let object = DrawingObject::VerticalLine { time: anchor.time };
                self.push_drawing(object, DrawingTool::VerticalLine.default_style());
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

        if self
            .selected_drawing
            .map(|id| self.drawing_index(id).is_none())
            .unwrap_or(false)
        {
            self.selected_drawing = None;
            changed = true;
        }

        if self
            .active_drag_any()
            .map(|drag| self.drawing_index(drag.id).is_none())
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

        changed
    }

    fn open_drawing_settings(&mut self, id: DrawingId) -> bool {
        let mut changed = false;
        changed |= self.select_drawing(Some(id));
        changed |= self.set_sidebar_page(SidebarPage::Tools);
        changed
    }

    fn selected_entity(&self) -> Option<&DrawingEntity> {
        let selected = self.selected_drawing?;
        self.drawings.iter().find(|drawing| drawing.id == selected)
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

        self.selected_drawing = Some(id);
        self.interaction = DrawingInteraction::Dragging(DrawingDragState {
            id,
            target,
            origin_anchor: anchor,
            origin_object: drawing.object.clone(),
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

        let Some(index) = self.drawing_index(id) else {
            self.clear_drag();
            return false;
        };

        if matches!(target, DrawingDragTarget::Translate)
            && anchor.panel_id != drag_state.origin_anchor.panel_id
        {
            return false;
        }

        if matches!(target, DrawingDragTarget::Handle(_))
            && anchor.panel_id != drag_state.origin_anchor.panel_id
        {
            return false;
        }

        if self.drawings[index].locked {
            self.clear_drag();
            return false;
        }

        let moved = match target {
            DrawingDragTarget::Translate => drag_state
                .origin_object
                .translated(drag_state.origin_anchor, anchor),
            DrawingDragTarget::Handle(handle) => {
                let Some(edited) = drag_state.origin_object.handle_dragged(handle, anchor) else {
                    return false;
                };

                edited
            }
        };

        if self.drawings[index].object == moved {
            return false;
        }

        self.drawings[index].object = moved;
        self.selected_drawing = Some(id);
        true
    }

    fn finish_drawing_drag(&mut self, id: DrawingId) -> bool {
        if matches!(self.interaction, DrawingInteraction::Dragging(ref drag) if drag.id == id) {
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
        true
    }

    fn drawing_index(&self, id: DrawingId) -> Option<usize> {
        self.drawings.iter().position(|drawing| drawing.id == id)
    }

    fn active_drag(&self, id: DrawingId) -> Option<&DrawingDragState> {
        match &self.interaction {
            DrawingInteraction::Dragging(drag) if drag.id == id => Some(drag),
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
        let id = DrawingId(self.next_drawing_id.max(1));
        self.next_drawing_id = self.next_drawing_id.wrapping_add(1).max(1);

        self.drawings.push(DrawingEntity {
            id,
            object,
            style,
            locked: false,
            visible: true,
        });

        self.selected_drawing = Some(id);
        self.interaction = DrawingInteraction::Idle;
        id
    }
}
