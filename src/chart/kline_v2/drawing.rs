use crate::style;
use crate::widget::chart::kline::composition::PanelId;
use crate::widget::chart::kline::{
    DrawingAnchor, DrawingDraft, DrawingEntity, DrawingId, DrawingObject, DrawingStyle,
    DrawingTool, KlineWidgetDrawingEvent, KlineWidgetEvent, YUnit,
};

use exchange::UnixMs;

const SIDEBAR_WIDTH: f32 = 36.0;
const DETAILS_SIDEBAR_WIDTH: f32 = 108.0;

#[derive(Debug, Clone)]
struct DrawingDragState {
    id: DrawingId,
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
    SelectedDrawingBorderColor,
    SelectedDrawingFillColor,
    SelectedDrawingBorderWidth,
    SelectedDrawingLineColor,
    SelectedDrawingLineWidth,
    DeleteSelectedDrawing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Event {
    Selected(Option<DrawingId>),
    AnchorPressed(DrawingAnchor),
    AnchorMoved(DrawingAnchor),
    DragStarted {
        id: DrawingId,
        anchor: DrawingAnchor,
    },
    DragMoved {
        id: DrawingId,
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
            KlineWidgetDrawingEvent::DragStarted { id, anchor } => Self::DragStarted { id, anchor },
            KlineWidgetDrawingEvent::DragMoved { id, anchor } => Self::DragMoved { id, anchor },
            KlineWidgetDrawingEvent::DragFinished { id } => Self::DragFinished { id },
            KlineWidgetDrawingEvent::DraftCanceled => Self::DraftCanceled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawingTools {
    active_tool: DrawingTool,
    drawings: Vec<DrawingEntity>,
    selected_drawing: Option<DrawingId>,
    interaction: DrawingInteraction,
    next_drawing_id: u64,
}

impl Default for DrawingTools {
    fn default() -> Self {
        Self {
            active_tool: DrawingTool::Cursor,
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
        }
    }

    fn handle_event(&mut self, event: Event) -> DrawingUpdate {
        match event {
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
            Event::DragStarted { id, anchor } => {
                if self.start_drawing_drag(id, anchor) {
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            Event::DragMoved { id, anchor } => {
                if self.update_drawing_drag(id, anchor) {
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
        }
    }

    pub fn handle_kline_widget_event(&mut self, event: &KlineWidgetEvent) -> Option<DrawingUpdate> {
        let event = Event::from_kline_widget_event(event)?;
        Some(self.handle_event(event))
    }

    pub fn view<'a, Message>(
        &'a self,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a,
    {
        if let Some(drawing) = self.selected_entity() {
            self.view_selected_sidebar(drawing, on_sidebar_message)
        } else {
            self.view_tools_sidebar(on_sidebar_message)
        }
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

    fn selected_entity(&self) -> Option<&DrawingEntity> {
        let selected = self.selected_drawing?;
        self.drawings.iter().find(|drawing| drawing.id == selected)
    }

    fn drawing_object_title(object: &DrawingObject) -> &'static str {
        match object {
            DrawingObject::Box { .. } => "Box",
            DrawingObject::Trendline { .. } => "Trendline",
            DrawingObject::HorizontalLine { .. } => "Horizontal Line",
            DrawingObject::VerticalLine { .. } => "Vertical Line",
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
            .push(iced::widget::text(Self::drawing_object_title(&drawing.object)).size(13))
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
    }

    fn view_tools_sidebar<'a, Message>(
        &'a self,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a,
    {
        let mut tools = iced::widget::Column::new().spacing(6);

        for tool in Self::drawing_tools().iter().copied() {
            let selected = self.active_tool == tool;
            let label = match tool {
                DrawingTool::Cursor => "C",
                DrawingTool::HorizontalLine => "H",
                DrawingTool::VerticalLine => "V",
                DrawingTool::Trendline => "TL",
                DrawingTool::Box => "B",
            }
            .to_string();

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

        iced::widget::container(tools)
            .style(style::chart_sidebar_container)
            .width(SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 2])
    }

    fn drawing_tools() -> &'static [DrawingTool] {
        &[
            DrawingTool::Cursor,
            DrawingTool::Trendline,
            DrawingTool::Box,
            DrawingTool::HorizontalLine,
            DrawingTool::VerticalLine,
        ]
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

    pub fn start_drawing_from_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        match self.active_tool {
            DrawingTool::Cursor => false,
            DrawingTool::Trendline => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Trendline {
                    start: anchor,
                    current: anchor,
                    style: Self::style_for_tool(DrawingTool::Trendline),
                });
                self.selected_drawing = None;
                true
            }
            DrawingTool::Box => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Box {
                    start: anchor,
                    current: anchor,
                    style: Self::style_for_tool(DrawingTool::Box),
                });
                self.selected_drawing = None;
                true
            }
            DrawingTool::HorizontalLine => {
                let object = DrawingObject::HorizontalLine {
                    panel_id: anchor.panel_id,
                    y_unit: anchor.y_unit,
                };
                self.push_drawing(object, Self::style_for_tool(DrawingTool::HorizontalLine));
                true
            }
            DrawingTool::VerticalLine => {
                let object = DrawingObject::VerticalLine { time: anchor.time };
                self.push_drawing(object, Self::style_for_tool(DrawingTool::VerticalLine));
                true
            }
        }
    }

    pub fn update_drawing_draft_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        let DrawingInteraction::Drafting(draft) = &mut self.interaction else {
            return false;
        };

        match draft {
            DrawingDraft::Trendline { current, .. } | DrawingDraft::Box { current, .. } => {
                *current = anchor;
            }
        }

        true
    }

    pub fn commit_drawing_draft(&mut self, end: DrawingAnchor) -> Option<DrawingId> {
        let draft = self.take_draft()?;

        let (object, style) = match draft {
            DrawingDraft::Trendline { start, style, .. } => {
                (DrawingObject::Trendline { start, end }, style)
            }
            DrawingDraft::Box { start, style, .. } => (DrawingObject::Box { start, end }, style),
        };

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
            .retain(|drawing| !Self::drawing_belongs_to_panel(drawing, panel_id));

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
            .map(|draft| Self::draft_belongs_to_panel(draft, panel_id))
            .unwrap_or(false)
        {
            self.interaction = DrawingInteraction::Idle;
            changed = true;
        }

        changed
    }

    fn start_drawing_drag(&mut self, id: DrawingId, anchor: DrawingAnchor) -> bool {
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
            origin_anchor: anchor,
            origin_object: drawing.object.clone(),
        });
        true
    }

    fn update_drawing_drag(&mut self, id: DrawingId, anchor: DrawingAnchor) -> bool {
        let Some(drag_state) = self.active_drag(id).cloned() else {
            return false;
        };

        let Some(index) = self.drawing_index(id) else {
            self.clear_drag();
            return false;
        };

        if anchor.panel_id != drag_state.origin_anchor.panel_id {
            return false;
        }

        if self.drawings[index].locked {
            self.clear_drag();
            return false;
        }

        let moved = Self::translated_drawing_object(
            &drag_state.origin_object,
            drag_state.origin_anchor,
            anchor,
        );

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

    fn style_for_tool(tool: DrawingTool) -> DrawingStyle {
        let mut style = DrawingStyle::default();

        match tool {
            DrawingTool::Cursor => {}
            DrawingTool::Trendline => {
                style.stroke_color = iced::Color::from_rgb(0.78, 0.86, 0.98);
                style.stroke_width = 1.2;
            }
            DrawingTool::Box => {
                style.stroke_color = iced::Color::from_rgb(0.72, 0.84, 0.98);
                style.stroke_width = 1.2;
                style.fill_color = Some(iced::Color::from_rgba(0.50, 0.66, 0.98, 0.16));
            }
            DrawingTool::HorizontalLine => {
                style.stroke_color = iced::Color::from_rgb(0.96, 0.80, 0.40);
                style.stroke_width = 1.0;
            }
            DrawingTool::VerticalLine => {
                style.stroke_color = iced::Color::from_rgb(0.74, 0.90, 0.74);
                style.stroke_width = 1.0;
            }
        }

        style
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

    fn translated_drawing_object(
        object: &DrawingObject,
        origin_anchor: DrawingAnchor,
        current_anchor: DrawingAnchor,
    ) -> DrawingObject {
        let (delta_ms, forward_in_time) = Self::time_delta(origin_anchor.time, current_anchor.time);
        let delta_y = current_anchor
            .y_unit
            .0
            .saturating_sub(origin_anchor.y_unit.0);

        match object {
            DrawingObject::Trendline { start, end } => DrawingObject::Trendline {
                start: Self::shift_anchor_by_delta(*start, delta_ms, forward_in_time, delta_y),
                end: Self::shift_anchor_by_delta(*end, delta_ms, forward_in_time, delta_y),
            },
            DrawingObject::Box { start, end } => DrawingObject::Box {
                start: Self::shift_anchor_by_delta(*start, delta_ms, forward_in_time, delta_y),
                end: Self::shift_anchor_by_delta(*end, delta_ms, forward_in_time, delta_y),
            },
            DrawingObject::HorizontalLine { panel_id, y_unit } => DrawingObject::HorizontalLine {
                panel_id: *panel_id,
                y_unit: Self::shift_y_by_delta(*y_unit, delta_y),
            },
            DrawingObject::VerticalLine { time } => DrawingObject::VerticalLine {
                time: Self::shift_time_by_delta_ms(*time, delta_ms, forward_in_time),
            },
        }
    }

    fn shift_anchor_by_delta(
        anchor: DrawingAnchor,
        delta_ms: u64,
        forward_in_time: bool,
        delta_y: i64,
    ) -> DrawingAnchor {
        DrawingAnchor {
            panel_id: anchor.panel_id,
            time: Self::shift_time_by_delta_ms(anchor.time, delta_ms, forward_in_time),
            y_unit: Self::shift_y_by_delta(anchor.y_unit, delta_y),
        }
    }

    fn shift_y_by_delta(y_unit: YUnit, delta_y: i64) -> YUnit {
        YUnit(y_unit.0.saturating_add(delta_y))
    }

    fn shift_time_by_delta_ms(time: UnixMs, delta_ms: u64, forward_in_time: bool) -> UnixMs {
        if forward_in_time {
            time.saturating_add(delta_ms)
        } else {
            time.saturating_sub(delta_ms)
        }
    }

    fn time_delta(origin: UnixMs, current: UnixMs) -> (u64, bool) {
        if current >= origin {
            (current.saturating_diff(origin), true)
        } else {
            (origin.saturating_diff(current), false)
        }
    }

    fn drawing_belongs_to_panel(drawing: &DrawingEntity, panel_id: PanelId) -> bool {
        match drawing.object {
            DrawingObject::Trendline { start, end } | DrawingObject::Box { start, end } => {
                start.panel_id == panel_id || end.panel_id == panel_id
            }
            DrawingObject::HorizontalLine {
                panel_id: drawing_panel,
                ..
            } => drawing_panel == panel_id,
            DrawingObject::VerticalLine { .. } => false,
        }
    }

    fn draft_belongs_to_panel(draft: &DrawingDraft, panel_id: PanelId) -> bool {
        match draft {
            DrawingDraft::Trendline { start, current, .. }
            | DrawingDraft::Box { start, current, .. } => {
                start.panel_id == panel_id || current.panel_id == panel_id
            }
        }
    }
}
