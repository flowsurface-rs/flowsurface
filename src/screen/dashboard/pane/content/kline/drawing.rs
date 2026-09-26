use super::toolbar::{FloatingToolbarEvent, FloatingToolbarState};
use super::{FLOATING_TOOLBAR_HANDLE_WIDTH, FLOATING_TOOLBAR_HEIGHT};
use crate::style;
use crate::theme::ThemeAccents;
use crate::widget::chart::kline::drawing::{
    DrawingAnchor, DrawingColor, DrawingDraft, DrawingDragOperation, DrawingDragTarget,
    DrawingEntity, DrawingId, DrawingMeasure, DrawingObject, DrawingSnapshot, DrawingStyle,
    DrawingTool, DrawingX, KlineWidgetDrawingEvent, MAX_DRAWING_STROKE_WIDTH,
    MIN_DRAWING_STROKE_WIDTH, MeasureState,
};
use crate::widget::{color_swatch_button, tooltip};
use data::chart::kline::DrawingSettings;
use data::chart::kline::composition::{ChartComposition, PanelId, PanelRole};
use data::config::theme::{ThemeAccent, ThemeColor, ThemeColorRole, ThemeColorShade};

use iced::widget::{checkbox, pick_list, row, text, tooltip::Position as TooltipPosition};
use iced::{Element, Length};

const SIDEBAR_WIDTH: f32 = 36.0;
const OBJECT_LIST_SIDEBAR_WIDTH: f32 = 196.0;
pub(super) const DRAWING_SIDEBAR_PADDING: f32 = 4.0;
const DRAWING_STROKE_WIDTH_STEP: f32 = 0.5;
const DRAWING_FILL_ALPHA: f32 = 0.16;
const DRAWING_TOOLBAR_SHADE_WIDTH: f32 = 76.0;
const DRAWING_TOOLBAR_STYLE_TARGET_WIDTH: f32 = 72.0;
const DRAWING_TOOLBAR_WIDTH_CONTROL_WIDTH: f32 = 64.0;
const DRAWING_TOOLBAR_DIVIDER_HEIGHT: f32 = 24.0;

fn ensure_box_fill_color(style: &mut DrawingStyle) {
    if style.fill_color.is_none() {
        style.fill_color = Some(style.stroke_color.scale_alpha(DRAWING_FILL_ALPHA));
    }
}

fn drawing_color_picker<'a, Message, SelectColor>(
    theme_accents: ThemeAccents,
    selected: SharedValue<Option<DrawingColor>>,
    shade: SharedValue<ThemeColorShade>,
    alpha: f32,
    select_color: SelectColor,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
    SelectColor: Fn(ThemeAccent) -> DrawingMessage + Copy + 'a,
{
    let selected_accent = selected.uniform().flatten().map(|color| color.accent);
    let shade = shade.representative().unwrap_or_default();
    let colors = ThemeAccent::ALL.into_iter().map(|accent| {
        color_swatch_button(
            move |theme: &iced::Theme| {
                let palette = theme.extended_palette();
                crate::widget::resolve_theme_color(
                    ThemeColor::with_accent(ThemeColorRole::Primary, Some(accent), shade),
                    palette,
                    theme_accents.as_array(),
                )
                .scale_alpha(alpha)
            },
            selected_accent == Some(accent),
            on_sidebar_message(select_color(accent)),
        )
    });

    row(colors).spacing(3).into()
}

fn drawing_shade_picker<'a, Message>(
    selected: SharedValue<ThemeColorShade>,
    on_select: impl Fn(ThemeColorShade) -> DrawingMessage + Copy + 'a,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    let mut picker = pick_list(ThemeColorShade::ALL, selected.uniform(), move |shade| {
        on_sidebar_message(on_select(shade))
    })
    .padding(4)
    .width(Length::Fill);

    if selected.is_mixed() {
        picker = picker.placeholder("Mixed");
    }

    picker.into()
}

fn drawing_width_control<'a, Message>(
    current: SharedValue<f32>,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    let current_value = current.representative().unwrap_or(MIN_DRAWING_STROKE_WIDTH);
    let is_mixed = current.is_mixed();

    let tooltip = if is_mixed {
        "Width: Mixed".to_string()
    } else {
        format!("Width: {current_value}")
    };
    let slider = iced::widget::slider(
        MIN_DRAWING_STROKE_WIDTH..=MAX_DRAWING_STROKE_WIDTH,
        current_value,
        move |width| on_sidebar_message(DrawingMessage::SetSelectedDrawingStrokeWidth(width)),
    )
    .step(DRAWING_STROKE_WIDTH_STEP)
    .width(Length::Fill);

    iced::widget::tooltip(
        slider,
        iced::widget::container(text(tooltip))
            .style(style::tooltip)
            .padding(8),
        TooltipPosition::Top,
    )
    .into()
}

fn drawing_extend_checkbox<'a, Message>(
    checked: SharedValue<bool>,
    label: &'static str,
    on_toggle: impl Fn(bool) -> DrawingMessage + Copy + 'a,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    let label = if checked.is_mixed() {
        format!("{label} (Mixed)")
    } else {
        label.to_string()
    };

    checkbox(checked.representative().unwrap_or(false))
        .label(label)
        .spacing(8)
        .on_toggle(move |value| on_sidebar_message(on_toggle(value)))
        .into()
}

fn drawing_stroke_controls<'a, Message>(
    theme_accents: ThemeAccents,
    stroke_color: SharedValue<DrawingColor>,
    stroke_shade: SharedValue<ThemeColorShade>,
    stroke_width: SharedValue<f32>,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    row![
        iced::widget::container(drawing_color_picker(
            theme_accents,
            stroke_color.map(Some),
            stroke_shade,
            1.0,
            DrawingMessage::SetSelectedDrawingStrokeColor,
            on_sidebar_message,
        ))
        .width(Length::Shrink),
        iced::widget::container(drawing_shade_picker(
            stroke_shade,
            DrawingMessage::SetSelectedDrawingStrokeShade,
            on_sidebar_message,
        ))
        .width(DRAWING_TOOLBAR_SHADE_WIDTH),
        iced::widget::container(drawing_width_control(stroke_width, on_sidebar_message,))
            .width(DRAWING_TOOLBAR_WIDTH_CONTROL_WIDTH),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center)
    .into()
}

fn drawing_box_style_target<'a, Message>(
    selected: BoxStyleTarget,
    on_sidebar_message: fn(DrawingMessage) -> Message,
) -> Element<'a, Message>
where
    Message: Clone + 'a + 'static,
{
    pick_list(BoxStyleTarget::ALL, Some(selected), move |target| {
        on_sidebar_message(target.selection_message())
    })
    .padding(4)
    .width(DRAWING_TOOLBAR_STYLE_TARGET_WIDTH)
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
    AtomicEdit,
    GestureStarted { visual_changed: bool },
    GestureUpdated,
    GestureFinished { visual_changed: bool },
}

impl DrawingUpdate {
    pub fn should_bump(self) -> bool {
        matches!(
            self,
            Self::Visual
                | Self::AtomicEdit
                | Self::GestureStarted {
                    visual_changed: true
                }
                | Self::GestureUpdated
                | Self::GestureFinished {
                    visual_changed: true
                }
        )
    }
}

#[derive(Default)]
pub(super) struct DrawingState {
    pub(super) tools: DrawingTools,
    pub(super) floating_toolbar: FloatingToolbarState,
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Tools(DrawingMessage),
    FloatingToolbar(FloatingToolbarEvent),
}

impl DrawingState {
    pub(super) fn update(&mut self, event: Event, settings: &mut DrawingSettings) -> DrawingUpdate {
        match event {
            Event::Tools(message) => self.tools.update(message, settings),
            Event::FloatingToolbar(event) => {
                self.floating_toolbar.update(event);
                DrawingUpdate::None
            }
        }
    }

    pub(super) fn view<'a>(&'a self, theme_accents: ThemeAccents) -> iced::Element<'a, Event> {
        let floating_toolbar = iced::widget::responsive(move |overlay_size| {
            self.tools
                .selected_toolbar_view(theme_accents, Event::Tools)
                .map(|controls| self.floating_toolbar_content(controls, overlay_size))
                .unwrap_or_else(|| self.empty_floating_toolbar_content())
        });

        let mut overlay = iced::widget::Stack::new()
            .width(Length::Fill)
            .height(Length::Fill)
            .push(floating_toolbar);

        if self.floating_toolbar.is_dragging {
            overlay = overlay.push(iced::widget::responsive(move |overlay_size| {
                self.floating_toolbar_drag_capture(overlay_size)
            }));
        }

        overlay.into()
    }

    pub(super) fn sidebar_view<'a>(
        &'a self,
        composition: &'a ChartComposition,
    ) -> iced::Element<'a, Event> {
        self.tools
            .sidebar_view(composition, Event::Tools)
            .padding(DRAWING_SIDEBAR_PADDING)
            .into()
    }

    fn empty_floating_toolbar_content<'a>(&self) -> iced::Element<'a, Event> {
        iced::widget::container(iced::widget::space::horizontal())
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn floating_toolbar_content<'a>(
        &'a self,
        controls: iced::Element<'a, Event>,
        overlay_size: iced::Size,
    ) -> iced::Element<'a, Event> {
        let drag_handle = iced::widget::mouse_area(
            iced::widget::container(style::icon_text(style::Icon::DragHandle, 12))
                .width(FLOATING_TOOLBAR_HANDLE_WIDTH)
                .height(Length::Fill)
                .align_x(iced::alignment::Horizontal::Center)
                .align_y(iced::alignment::Vertical::Center),
        )
        .on_move(|cursor| Event::FloatingToolbar(FloatingToolbarEvent::HandleMoved(cursor)))
        .on_press(Event::FloatingToolbar(FloatingToolbarEvent::DragStarted))
        .interaction(iced::mouse::Interaction::Grab);

        let scrollable_controls = iced::widget::scrollable::Scrollable::with_direction(
            controls,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::new()
                    .width(6)
                    .scroller_width(6),
            ),
        )
        .anchor_left()
        .width(Length::Shrink)
        .height(Length::Fill)
        .style(style::scroll_bar);
        let toolbar = iced::widget::mouse_area(
            iced::widget::container(iced::widget::row![drag_handle, scrollable_controls])
                .style(style::chart_toolbar)
                .width(Length::Shrink)
                .height(FLOATING_TOOLBAR_HEIGHT),
        )
        .interaction(iced::mouse::Interaction::Idle);
        let position = self.floating_toolbar.position_within(overlay_size);

        iced::widget::container(
            iced::widget::column![
                iced::widget::space::vertical().height(position.y),
                iced::widget::row![iced::widget::space::horizontal().width(position.x), toolbar,],
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn floating_toolbar_drag_capture<'a>(
        &'a self,
        overlay_size: iced::Size,
    ) -> iced::Element<'a, Event> {
        iced::widget::mouse_area(
            iced::widget::space::horizontal()
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_move(move |cursor| {
            Event::FloatingToolbar(FloatingToolbarEvent::DragMoved {
                cursor,
                overlay_size,
            })
        })
        .on_release(Event::FloatingToolbar(FloatingToolbarEvent::DragFinished))
        .interaction(iced::mouse::Interaction::Grabbing)
        .into()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct DrawingEditState {
    drawings: Vec<DrawingEntity>,
    inactive_drawings: Vec<DrawingEntity>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SharedValue<T> {
    Unavailable,
    Uniform(T),
    Mixed(T),
}

impl<T: Copy> SharedValue<T> {
    fn representative(self) -> Option<T> {
        match self {
            Self::Unavailable => None,
            Self::Uniform(value) | Self::Mixed(value) => Some(value),
        }
    }

    fn uniform(self) -> Option<T> {
        match self {
            Self::Uniform(value) => Some(value),
            Self::Unavailable | Self::Mixed(_) => None,
        }
    }

    fn is_mixed(self) -> bool {
        matches!(self, Self::Mixed(_))
    }

    fn map<U: Copy>(self, map: impl FnOnce(T) -> U) -> SharedValue<U> {
        match self {
            Self::Unavailable => SharedValue::Unavailable,
            Self::Uniform(value) => SharedValue::Uniform(map(value)),
            Self::Mixed(value) => SharedValue::Mixed(map(value)),
        }
    }
}

fn shared_value<T, Values>(mut values: Values) -> SharedValue<T>
where
    T: Copy + PartialEq,
    Values: Iterator<Item = T>,
{
    let Some(representative) = values.next() else {
        return SharedValue::Unavailable;
    };

    if values.all(|value| value == representative) {
        SharedValue::Uniform(representative)
    } else {
        SharedValue::Mixed(representative)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionCapabilities {
    editable_count: usize,
    editable_box_count: usize,
    editable_horizontal_line_count: usize,
}

impl SelectionCapabilities {
    fn has_editable(self) -> bool {
        self.editable_count > 0
    }

    fn has_boxes(self) -> bool {
        self.editable_box_count > 0
    }

    fn has_horizontal_lines(self) -> bool {
        self.editable_horizontal_line_count > 0
    }
}

#[derive(Debug, Clone)]
struct DrawingSelectionSummary<'a> {
    selected: Vec<&'a DrawingEntity>,
    capabilities: SelectionCapabilities,
    stroke_color: SharedValue<DrawingColor>,
    stroke_shade: SharedValue<ThemeColorShade>,
    stroke_width: SharedValue<f32>,
    fill_color: SharedValue<Option<DrawingColor>>,
    fill_shade: SharedValue<ThemeColorShade>,
    box_extend_right: SharedValue<bool>,
    horizontal_line_ray: SharedValue<bool>,
}

impl DrawingSelectionSummary<'_> {
    fn title(&self) -> String {
        if self.selected.len() == 1 {
            self.selected[0].object.title().to_string()
        } else {
            format!("{} drawings", self.selected.len())
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DrawingStyleChange {
    StrokeColor(ThemeAccent),
    StrokeShade(ThemeColorShade),
    FillColor(ThemeAccent),
    FillShade(ThemeColorShade),
    StrokeWidth(f32),
}

impl DrawingStyleChange {
    fn applies_to(self, object: &DrawingObject) -> bool {
        match self {
            Self::FillColor(_) | Self::FillShade(_) => {
                matches!(object, DrawingObject::Box { .. })
            }
            Self::StrokeColor(_) | Self::StrokeShade(_) | Self::StrokeWidth(_) => true,
        }
    }

    fn apply(self, style: &mut DrawingStyle) -> bool {
        let before = *style;

        match self {
            Self::StrokeColor(color) => {
                style.stroke_color = style.stroke_color.with_accent(color, 1.0);
            }
            Self::StrokeShade(shade) => {
                style.stroke_color.shade = shade;
            }
            Self::FillColor(color) => {
                let shade = style
                    .fill_color
                    .map(|color| color.shade)
                    .unwrap_or_default();
                style.fill_color = Some(DrawingColor::themed(color, shade, DRAWING_FILL_ALPHA));
            }
            Self::FillShade(shade) => {
                if let Some(color) = &mut style.fill_color {
                    color.shade = shade;
                }
            }
            Self::StrokeWidth(width) => {
                style.set_stroke_width(width);
            }
        }

        *style != before
    }
}

#[derive(Debug, Clone, Copy)]
enum DrawingObjectChange {
    HorizontalLineRay(bool),
    BoxExtendRight(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrawingMessage {
    SelectTool(DrawingTool),
    ToggleMagnet,
    ShowToolsPage,
    ShowObjectListPage,
    ClearSelection,
    OpenDrawingToolbar(DrawingId),
    SetSelectedDrawingStrokeColor(ThemeAccent),
    SetSelectedDrawingStrokeShade(ThemeColorShade),
    SetSelectedDrawingFillColor(ThemeAccent),
    SetSelectedDrawingFillShade(ThemeColorShade),
    SetSelectedDrawingStrokeWidth(f32),
    SelectBoxBorderStyle,
    SelectBoxFillStyle,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum BoxStyleTarget {
    #[default]
    Border,
    Fill,
}

impl BoxStyleTarget {
    const ALL: &'static [Self] = &[Self::Border, Self::Fill];

    fn selection_message(self) -> DrawingMessage {
        match self {
            Self::Border => DrawingMessage::SelectBoxBorderStyle,
            Self::Fill => DrawingMessage::SelectBoxFillStyle,
        }
    }
}

impl std::fmt::Display for BoxStyleTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Border => formatter.write_str("Border"),
            Self::Fill => formatter.write_str("Fill"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrawingTools {
    active_tool: DrawingTool,
    sidebar_page: SidebarPage,
    drawings: Vec<DrawingEntity>,
    inactive_drawings: Vec<DrawingEntity>,
    tick_mode: bool,
    tick_sequence_id: Option<uuid::Uuid>,
    selected_drawings: Vec<DrawingId>,
    interaction: DrawingInteraction,
    next_drawing_id: u64,
    magnet_enabled: bool,
    magnet_modifier_active: bool,
    box_style_target: BoxStyleTarget,
    measure: Option<DrawingMeasure>,
}

impl Default for DrawingTools {
    fn default() -> Self {
        Self {
            active_tool: DrawingTool::Cursor,
            sidebar_page: SidebarPage::Tools,
            drawings: Vec::new(),
            inactive_drawings: Vec::new(),
            tick_mode: false,
            tick_sequence_id: None,
            selected_drawings: Vec::new(),
            interaction: DrawingInteraction::Idle,
            next_drawing_id: 1,
            magnet_enabled: false,
            magnet_modifier_active: false,
            box_style_target: BoxStyleTarget::default(),
            measure: None,
        }
    }
}

impl DrawingTools {
    pub(super) fn edit_state(&self) -> DrawingEditState {
        DrawingEditState {
            drawings: self.drawings.clone(),
            inactive_drawings: self.inactive_drawings.clone(),
        }
    }

    pub(super) fn restore_edit_state(&mut self, state: DrawingEditState) {
        self.drawings = state.drawings;
        self.inactive_drawings = state.inactive_drawings;
        self.next_drawing_id = self
            .drawings
            .iter()
            .chain(&self.inactive_drawings)
            .map(|drawing| drawing.id.0)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(1);
        self.selected_drawings.clear();
        self.interaction = DrawingInteraction::Idle;
        self.measure = None;
    }

    pub(super) fn is_dragging(&self) -> bool {
        matches!(self.interaction, DrawingInteraction::Dragging(_))
    }

    pub(super) fn can_handle_escape(&self) -> bool {
        self.active_drag_any().is_some()
            || self.measure.is_some()
            || self.drawing_draft().is_some()
            || !self.selected_drawings.is_empty()
            || self.active_tool != DrawingTool::Cursor
            || self.sidebar_page == SidebarPage::ObjectList
    }

    pub fn from_parts(active_tool: DrawingTool, mut drawings: Vec<DrawingEntity>) -> Self {
        drawings.retain(|drawing| drawing.object.coordinates_match(DrawingX::default()));
        for drawing in &mut drawings {
            if matches!(&drawing.object, DrawingObject::Box { .. }) {
                ensure_box_fill_color(&mut drawing.style);
            }
        }
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
            inactive_drawings: Vec::new(),
            tick_mode: false,
            tick_sequence_id: None,
            selected_drawings: Vec::new(),
            interaction: DrawingInteraction::Idle,
            next_drawing_id,
            magnet_enabled: false,
            magnet_modifier_active: false,
            box_style_target: BoxStyleTarget::default(),
            measure: None,
        }
    }

    pub fn active_tool(&self) -> DrawingTool {
        self.active_tool
    }

    fn magnet_active(&self) -> bool {
        self.active_tool.supports_magnet() && (self.magnet_enabled || self.magnet_modifier_active)
    }

    pub fn time_drawings(&self) -> &[DrawingEntity] {
        if self.tick_mode {
            &self.inactive_drawings
        } else {
            &self.drawings
        }
    }

    pub fn set_basis(&mut self, basis: data::chart::Basis) {
        let tick_mode = !basis.is_time();
        if tick_mode != self.tick_mode {
            std::mem::swap(&mut self.drawings, &mut self.inactive_drawings);
            self.tick_mode = tick_mode;
            self.clear_tick_drawings();
        }
        self.clear_interaction();
    }

    pub fn sync_tick_sequence(&mut self, sequence_id: Option<uuid::Uuid>) -> bool {
        if self.tick_sequence_id == sequence_id {
            return false;
        }
        self.clear_tick_drawings();
        self.tick_sequence_id = sequence_id;
        true
    }

    fn clear_tick_drawings(&mut self) {
        if self.tick_mode {
            self.drawings.clear();
            self.clear_interaction();
        } else {
            self.inactive_drawings.clear();
        }
    }

    fn clear_interaction(&mut self) {
        self.selected_drawings.clear();
        self.interaction = DrawingInteraction::Idle;
        self.measure = None;
        self.magnet_modifier_active = false;
    }

    fn accepts_anchor(&self, anchor: DrawingAnchor) -> bool {
        anchor.x.is_valid() && anchor.x.is_time() != self.tick_mode
    }

    fn selected_editable_indices(&self) -> Vec<usize> {
        self.selected_drawings
            .iter()
            .filter_map(|id| self.drawing_index(*id))
            .filter(|index| !self.drawings[*index].locked)
            .collect()
    }

    fn selection_capabilities(&self) -> SelectionCapabilities {
        let editable_indices = self.selected_editable_indices();
        let editable_box_count = editable_indices
            .iter()
            .filter(|index| matches!(self.drawings[**index].object, DrawingObject::Box { .. }))
            .count();
        let editable_horizontal_line_count = editable_indices
            .iter()
            .filter(|index| {
                matches!(
                    self.drawings[**index].object,
                    DrawingObject::HorizontalLine { .. }
                )
            })
            .count();

        SelectionCapabilities {
            editable_count: editable_indices.len(),
            editable_box_count,
            editable_horizontal_line_count,
        }
    }

    fn selection_summary(&self) -> Option<DrawingSelectionSummary<'_>> {
        let selected = self
            .selected_drawings
            .iter()
            .filter_map(|id| self.drawings.iter().find(|drawing| drawing.id == *id))
            .collect::<Vec<_>>();

        if selected.is_empty() {
            return None;
        }

        let capabilities = self.selection_capabilities();
        let editable = selected
            .iter()
            .copied()
            .filter(|drawing| !drawing.locked)
            .collect::<Vec<_>>();
        let boxes = editable
            .iter()
            .copied()
            .filter(|drawing| matches!(drawing.object, DrawingObject::Box { .. }))
            .collect::<Vec<_>>();
        let horizontal_lines = editable
            .iter()
            .copied()
            .filter(|drawing| matches!(drawing.object, DrawingObject::HorizontalLine { .. }))
            .collect::<Vec<_>>();

        let stroke_color = shared_value(editable.iter().map(|drawing| drawing.style.stroke_color));
        let stroke_shade = shared_value(
            editable
                .iter()
                .map(|drawing| drawing.style.stroke_color.shade),
        );
        let stroke_width = shared_value(
            editable
                .iter()
                .map(|drawing| drawing.style.normalized_stroke_width()),
        );
        let fill_color = shared_value(boxes.iter().map(|drawing| drawing.style.fill_color));
        let fill_shade = shared_value(boxes.iter().map(|drawing| {
            drawing
                .style
                .fill_color
                .map(|color| color.shade)
                .unwrap_or_default()
        }));
        let box_extend_right =
            shared_value(boxes.iter().filter_map(|drawing| match drawing.object {
                DrawingObject::Box { extend_right, .. } => Some(extend_right),
                _ => None,
            }));
        let horizontal_line_ray =
            shared_value(
                horizontal_lines
                    .iter()
                    .filter_map(|drawing| match drawing.object {
                        DrawingObject::HorizontalLine { ray, .. } => Some(ray),
                        _ => None,
                    }),
            );

        Some(DrawingSelectionSummary {
            selected,
            capabilities,
            stroke_color,
            stroke_shade,
            stroke_width,
            fill_color,
            fill_shade,
            box_extend_right,
            horizontal_line_ray,
        })
    }

    pub fn update(
        &mut self,
        message: DrawingMessage,
        settings: &mut DrawingSettings,
    ) -> DrawingUpdate {
        match message {
            DrawingMessage::SelectTool(tool) => {
                if self.select_tool(tool) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::ToggleMagnet => {
                self.magnet_enabled = !self.magnet_enabled;
                DrawingUpdate::Visual
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
            DrawingMessage::OpenDrawingToolbar(id) => {
                if self.open_drawing_toolbar(id) {
                    DrawingUpdate::Visual
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingStrokeColor(color) => {
                if self.update_selected_style(DrawingStyleChange::StrokeColor(color), settings) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingStrokeShade(shade) => {
                if self.update_selected_style(DrawingStyleChange::StrokeShade(shade), settings) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingFillColor(color) => {
                if self.update_selected_style(DrawingStyleChange::FillColor(color), settings) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingFillShade(shade) => {
                if self.update_selected_style(DrawingStyleChange::FillShade(shade), settings) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetSelectedDrawingStrokeWidth(width) => {
                if self.update_selected_style(DrawingStyleChange::StrokeWidth(width), settings) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SelectBoxBorderStyle => {
                if self.box_style_target != BoxStyleTarget::Border {
                    self.box_style_target = BoxStyleTarget::Border;
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SelectBoxFillStyle => {
                if self.box_style_target != BoxStyleTarget::Fill {
                    self.box_style_target = BoxStyleTarget::Fill;
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetHorizontalLineRay(ray) => {
                if self
                    .update_selected_object(DrawingObjectChange::HorizontalLineRay(ray), settings)
                {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::SetBoxExtendRight(extend) => {
                if self
                    .update_selected_object(DrawingObjectChange::BoxExtendRight(extend), settings)
                {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteSelectedDrawing => {
                if self.remove_selected_drawing() {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteDrawing(id) => {
                if self.remove_drawing(id) {
                    DrawingUpdate::AtomicEdit
                } else {
                    DrawingUpdate::None
                }
            }
            DrawingMessage::DeleteAllDrawings => {
                if self.remove_all_drawings() {
                    DrawingUpdate::AtomicEdit
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
        theme_accents: ThemeAccents,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> Option<Element<'a, Message>>
    where
        Message: Clone + 'a + 'static,
    {
        self.selection_summary().map(|selection| {
            self.view_selected_toolbar(theme_accents, selection, on_sidebar_message)
                .into()
        })
    }

    fn view_selected_toolbar<'a, Message>(
        &'a self,
        theme_accents: ThemeAccents,
        selection: DrawingSelectionSummary<'a>,
        on_sidebar_message: fn(DrawingMessage) -> Message,
    ) -> iced::widget::Container<'a, Message>
    where
        Message: Clone + 'a + 'static,
    {
        let mut controls = iced::widget::row![
            iced::widget::text(selection.title()).size(13),
            drawing_toolbar_divider(),
        ]
        .padding([2, 0])
        .spacing(8)
        .align_y(iced::Alignment::Center);

        if selection.capabilities.has_editable() {
            if selection.capabilities.has_boxes() {
                let style_controls = match self.box_style_target {
                    BoxStyleTarget::Border => drawing_stroke_controls(
                        theme_accents,
                        selection.stroke_color,
                        selection.stroke_shade,
                        selection.stroke_width,
                        on_sidebar_message,
                    ),
                    BoxStyleTarget::Fill => iced::widget::row![
                        iced::widget::container(drawing_color_picker(
                            theme_accents,
                            selection.fill_color,
                            selection.fill_shade,
                            DRAWING_FILL_ALPHA,
                            DrawingMessage::SetSelectedDrawingFillColor,
                            on_sidebar_message,
                        ))
                        .width(Length::Shrink),
                        iced::widget::container(drawing_shade_picker(
                            selection.fill_shade,
                            DrawingMessage::SetSelectedDrawingFillShade,
                            on_sidebar_message,
                        ))
                        .width(DRAWING_TOOLBAR_SHADE_WIDTH),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .into(),
                };

                let style_section = iced::widget::row![
                    drawing_box_style_target(self.box_style_target, on_sidebar_message),
                    style_controls,
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center);

                controls = controls
                    .push(style_section)
                    .push(drawing_toolbar_divider())
                    .push(drawing_extend_checkbox(
                        selection.box_extend_right,
                        "Extend right",
                        DrawingMessage::SetBoxExtendRight,
                        on_sidebar_message,
                    ));
            } else {
                controls = controls.push(drawing_stroke_controls(
                    theme_accents,
                    selection.stroke_color,
                    selection.stroke_shade,
                    selection.stroke_width,
                    on_sidebar_message,
                ));
            }

            if selection.capabilities.has_horizontal_lines() {
                controls = controls
                    .push(drawing_toolbar_divider())
                    .push(drawing_extend_checkbox(
                        selection.horizontal_line_ray,
                        "Ray",
                        DrawingMessage::SetHorizontalLineRay,
                        on_sidebar_message,
                    ));
            }
        }

        controls = controls.push(drawing_toolbar_divider());

        if selection.capabilities.has_editable() {
            controls = controls.push(
                iced::widget::button(style::icon_text(style::Icon::TrashBin, 12))
                    .style(|theme, status| style::button::cancel(theme, status, false))
                    .padding([4, 6])
                    .on_press(on_sidebar_message(DrawingMessage::DeleteSelectedDrawing)),
            );
        }

        controls = controls.push(
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

        let toolbar_button = iced::widget::button(
            iced::widget::row![
                iced::widget::text(object_label).size(12),
                iced::widget::space::horizontal(),
                style::icon_text(style::Icon::Cog, 12),
            ]
            .align_y(iced::Alignment::Center),
        )
        .style(move |theme, status| style::button::transparent(theme, status, selected))
        .width(iced::Length::Fill)
        .padding([2, 6])
        .on_press(on_sidebar_message(DrawingMessage::OpenDrawingToolbar(
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

        iced::widget::row![toolbar_button, remove_button]
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
            .padding(4)
            .spacing(4)
            .push(header_nav)
            .push(iced::widget::rule::horizontal(1.0));

        if self.tick_mode {
            content = content.push(
                iced::widget::text(
                    "Tick drawings last until you leave tick mode or reset its data.",
                )
                .size(style::text_size::SMALL),
            );
        }
        if self.drawings.is_empty() {
            content =
                content.push(iced::widget::text("No drawings").size(style::text_size::SECTION));
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
                    iced::widget::text(Self::panel_group_title(composition, panel_id))
                        .size(style::text_size::SECTION),
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

        if !self.drawings.is_empty() {
            content = content.push(row![
                iced::widget::space::horizontal(),
                remove_all_button.on_press(on_sidebar_message(DrawingMessage::DeleteAllDrawings))
            ])
        };

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

            let button = if selected {
                btn
            } else {
                btn.on_press(on_sidebar_message(DrawingMessage::SelectTool(tool)))
            };
            tools = tools.push(tooltip(button, Some(tool.label()), TooltipPosition::Right));
        }

        tools = tools.push(iced::widget::space::vertical());

        let magnet_enabled = self.magnet_enabled;
        let magnet_active = self.magnet_active();
        let magnet_button =
            iced::widget::button(iced::widget::text("M").align_x(iced::Alignment::Center))
                .style(move |theme, status| {
                    style::button::magnet(theme, status, magnet_enabled, magnet_active)
                })
                .width(iced::Length::Fill)
                .on_press(on_sidebar_message(DrawingMessage::ToggleMagnet));
        tools = tools.push(tooltip(
            magnet_button,
            Some("Magnet"),
            TooltipPosition::Right,
        ));

        let objects_button =
            iced::widget::button(iced::widget::text("O").align_x(iced::Alignment::Center))
                .style(|theme, status| style::button::transparent(theme, status, false))
                .width(iced::Length::Fill)
                .on_press(on_sidebar_message(DrawingMessage::ShowObjectListPage));
        tools = tools.push(tooltip(
            objects_button,
            Some("Drawings"),
            TooltipPosition::Right,
        ));

        iced::widget::container(tools)
            .style(style::chart_sidebar_container)
            .width(SIDEBAR_WIDTH)
            .height(iced::Length::Fill)
            .padding([4, 2])
    }

    pub fn snapshot(&self) -> DrawingSnapshot<'_> {
        DrawingSnapshot::new(
            self.active_tool,
            self.magnet_enabled,
            &self.drawings,
            &self.selected_drawings,
            self.drawing_draft(),
            self.measure,
        )
    }

    pub fn handle_widget_drawing_event(
        &mut self,
        event: KlineWidgetDrawingEvent,
        settings: &DrawingSettings,
    ) -> DrawingUpdate {
        match event {
            KlineWidgetDrawingEvent::MagnetModifierChanged(active) => {
                if self.magnet_modifier_active != active {
                    self.magnet_modifier_active = active;
                    DrawingUpdate::StateOnly
                } else {
                    DrawingUpdate::None
                }
            }
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
                        DrawingUpdate::AtomicEdit
                    } else {
                        DrawingUpdate::None
                    }
                } else {
                    let started = self.start_drawing_from_anchor(anchor, settings);
                    let committed = started && self.drawing_draft().is_none();
                    if committed {
                        self.active_tool = DrawingTool::Cursor;
                    }

                    if committed {
                        DrawingUpdate::AtomicEdit
                    } else if started {
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
                    DrawingUpdate::GestureStarted {
                        visual_changed: matches!(operation, DrawingDragOperation::Copy),
                    }
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DragMoved { id, target, anchor } => {
                if self.update_drawing_drag(id, target, anchor) {
                    DrawingUpdate::GestureUpdated
                } else {
                    DrawingUpdate::None
                }
            }
            KlineWidgetDrawingEvent::DragFinished { id } => {
                if self.finish_drawing_drag(id) {
                    DrawingUpdate::GestureFinished {
                        visual_changed: false,
                    }
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

    pub fn handle_escape(&mut self) -> DrawingUpdate {
        if self.set_sidebar_page(SidebarPage::Tools) {
            return DrawingUpdate::Visual;
        }

        if self.active_drag_any().is_some() {
            self.selected_drawings.clear();
            self.interaction = DrawingInteraction::Idle;
            return DrawingUpdate::GestureFinished {
                visual_changed: true,
            };
        }

        if self.measure.take().is_some() {
            return DrawingUpdate::Visual;
        }

        if self.cancel_drawing_draft() {
            return DrawingUpdate::Visual;
        }

        if self.select_drawing(None) {
            return DrawingUpdate::Visual;
        }

        if self.select_tool(DrawingTool::Cursor) {
            return DrawingUpdate::Visual;
        }

        DrawingUpdate::None
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
        let ids = self.selected_drawings.clone();
        if ids.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut retained = Vec::new();
        for id in ids {
            if self.remove_drawing(id) {
                changed = true;
            } else if self.drawing_index(id).is_some() {
                retained.push(id);
            }
        }

        self.selected_drawings = retained;
        changed
    }

    fn update_selected_style(
        &mut self,
        change: DrawingStyleChange,
        settings: &mut DrawingSettings,
    ) -> bool {
        let capabilities = self.selection_capabilities();
        if !capabilities.has_editable() {
            return false;
        }

        let mut changed_tools = Vec::new();
        let mut changed = false;

        for index in self.selected_editable_indices() {
            if !change.applies_to(&self.drawings[index].object) {
                continue;
            }

            let tool = self.drawings[index].object.tool();
            if change.apply(&mut self.drawings[index].style) {
                changed = true;
                if !changed_tools.contains(&tool) {
                    changed_tools.push(tool);
                }
            }
        }

        for tool in changed_tools {
            let mut default_style = settings.style_defaults.style_for(tool);
            if change.apply(&mut default_style) {
                settings.style_defaults.set_style(tool, default_style);
            }
        }

        changed
    }

    fn update_selected_object(
        &mut self,
        change: DrawingObjectChange,
        settings: &mut DrawingSettings,
    ) -> bool {
        let mut has_compatible_drawing = false;
        let mut object_changed = false;

        for index in self.selected_editable_indices() {
            let current_changed = match change {
                DrawingObjectChange::HorizontalLineRay(ray) => {
                    if let DrawingObject::HorizontalLine { ray: current, .. } =
                        &mut self.drawings[index].object
                    {
                        has_compatible_drawing = true;
                        let changed = *current != ray;
                        *current = ray;
                        changed
                    } else {
                        false
                    }
                }
                DrawingObjectChange::BoxExtendRight(extend_right) => {
                    if let DrawingObject::Box {
                        extend_right: current,
                        ..
                    } = &mut self.drawings[index].object
                    {
                        has_compatible_drawing = true;
                        let changed = *current != extend_right;
                        *current = extend_right;
                        changed
                    } else {
                        false
                    }
                }
            };

            object_changed |= current_changed;
        }

        if !has_compatible_drawing {
            return false;
        }

        let default_changed = match change {
            DrawingObjectChange::HorizontalLineRay(ray) => {
                let changed = settings.horizontal_line_ray != ray;
                settings.horizontal_line_ray = ray;
                changed
            }
            DrawingObjectChange::BoxExtendRight(extend_right) => {
                let changed = settings.box_extend_right != extend_right;
                settings.box_extend_right = extend_right;
                changed
            }
        };

        object_changed || default_changed
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

    pub fn start_drawing_from_anchor(
        &mut self,
        anchor: DrawingAnchor,
        settings: &DrawingSettings,
    ) -> bool {
        if !self.accepts_anchor(anchor) {
            return false;
        }
        match self.active_tool {
            DrawingTool::Cursor | DrawingTool::Measure => false,
            DrawingTool::Trendline => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Trendline {
                    start: anchor,
                    current: anchor,
                    style: settings.style_defaults.style_for(DrawingTool::Trendline),
                });
                self.selected_drawings.clear();
                true
            }
            DrawingTool::Box => {
                self.interaction = DrawingInteraction::Drafting(DrawingDraft::Box {
                    start: anchor,
                    current: anchor,
                    style: settings.style_defaults.style_for(DrawingTool::Box),
                    extend_right: settings.box_extend_right,
                });
                self.selected_drawings.clear();
                true
            }
            DrawingTool::HorizontalLine => {
                let object = DrawingObject::HorizontalLine {
                    panel_id: anchor.panel_id,
                    y_unit: anchor.y_unit,
                    x: anchor.x,
                    ray: settings.horizontal_line_ray,
                };
                self.push_drawing(
                    object,
                    settings
                        .style_defaults
                        .style_for(DrawingTool::HorizontalLine),
                );
                true
            }
            DrawingTool::VerticalLine => {
                let object = DrawingObject::VerticalLine { x: anchor.x };
                self.push_drawing(
                    object,
                    settings.style_defaults.style_for(DrawingTool::VerticalLine),
                );
                true
            }
        }
    }

    pub fn update_drawing_draft_anchor(&mut self, anchor: DrawingAnchor) -> bool {
        if !self.accepts_anchor(anchor) {
            return false;
        }
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
        if !self.accepts_anchor(end) {
            return None;
        }
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
        let before = self.drawings.len() + self.inactive_drawings.len();
        self.drawings
            .retain(|drawing| !drawing.belongs_to_panel(panel_id));
        self.inactive_drawings
            .retain(|drawing| !drawing.belongs_to_panel(panel_id));
        let drawings_pruned = before != self.drawings.len() + self.inactive_drawings.len();

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
        if !self.accepts_anchor(anchor) {
            return false;
        }
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
        if !self.accepts_anchor(anchor) {
            return false;
        }
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

    fn open_drawing_toolbar(&mut self, id: DrawingId) -> bool {
        self.select_drawing(Some(id))
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
        if !self.accepts_anchor(anchor) {
            return false;
        }
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
        if !self.accepts_anchor(anchor) {
            return false;
        }
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
                    let Some(moved) = dragged
                        .origin_object
                        .translated(drag_state.origin_anchor, anchor)
                    else {
                        return false;
                    };
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
                let Some(edited) =
                    primary
                        .origin_object
                        .handle_dragged(handle, drag_state.origin_anchor, anchor)
                else {
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
