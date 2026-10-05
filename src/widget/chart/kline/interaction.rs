pub(super) mod navigation;

use super::layout::LayoutHitZone;
use super::painting::RenderRequest;
use super::scene::ChartCoordinates;
use super::scene::{CursorTarget, Scene};
use super::studies::StudyObjectKey;
use data::chart::kline::HorizontalWorld;
use data::chart::kline::drawing::{DrawingMeasure, DrawingSnapshot};
use iced::advanced::mouse::Interaction;
use iced::event::Status;
use iced::{Point, Vector, keyboard};

#[derive(Clone, Copy)]
pub(super) struct LayerContext<'a> {
    pub(super) scene: &'a Scene,
    pub(super) coordinates: ChartCoordinates<'a>,
    pub(super) modifiers: keyboard::Modifiers,
    pub(super) position: Option<Point>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavigationFallback {
    Pan,
    PanOrMarquee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionDisposition {
    Preserve,
    Clear,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum LayerGesture {
    Click {
        selection: SelectionDisposition,
        captures_outside_release: bool,
    },
    Drag {
        selection: SelectionDisposition,
        active_outside_bounds: bool,
        cursor: Interaction,
    },
    ClickOrNavigate {
        selection: SelectionDisposition,
        captures_outside_release: bool,
        fallback: NavigationFallback,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PressAction {
    Ordinary,
    DrawingPlacement,
    Drawing {
        selection: SelectionDisposition,
        captures_outside_release: bool,
    },
    Layer {
        key: StudyObjectKey,
        selection: SelectionDisposition,
        captures_outside_release: bool,
    },
}

impl PressAction {
    pub(super) fn selection_disposition(self) -> SelectionDisposition {
        match self {
            Self::DrawingPlacement => SelectionDisposition::Preserve,
            Self::Drawing { selection, .. } => selection,
            Self::Ordinary => SelectionDisposition::Clear,
            Self::Layer { selection, .. } => selection,
        }
    }

    pub(super) fn captures_outside_release(self) -> bool {
        match self {
            Self::DrawingPlacement => true,
            Self::Drawing {
                captures_outside_release,
                ..
            } => captures_outside_release,
            Self::Layer {
                captures_outside_release,
                ..
            } => captures_outside_release,
            Self::Ordinary => false,
        }
    }

    pub(super) fn layer_key(self) -> Option<StudyObjectKey> {
        match self {
            Self::Layer { key, .. } => Some(key),
            Self::Ordinary | Self::DrawingPlacement | Self::Drawing { .. } => None,
        }
    }

    pub(super) fn is_drawing(self) -> bool {
        matches!(self, Self::Drawing { .. })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DragAction {
    Split(usize),
    Layer {
        key: StudyObjectKey,
        selection: SelectionDisposition,
        active_outside_bounds: bool,
        cursor: Interaction,
    },
    Drawing {
        selection: SelectionDisposition,
        active_outside_bounds: bool,
        cursor: Interaction,
    },
    YAxisScale {
        panel_index: usize,
        anchor_y: f32,
    },
    XAxisScale {
        initial_world: HorizontalWorld,
        initial_right_unit: f32,
    },
    Pan {
        panel_index: usize,
        source_layer: Option<StudyObjectKey>,
    },
    Marquee {
        panel_index: usize,
        source_layer: Option<StudyObjectKey>,
    },
}

impl DragAction {
    pub(super) fn scene_activity(self) -> SceneActivity {
        match self {
            Self::Pan { .. } => SceneActivity::Pan,
            _ => SceneActivity::Drag,
        }
    }

    pub(super) fn active_outside_bounds(self) -> bool {
        match self {
            Self::Layer {
                active_outside_bounds,
                ..
            }
            | Self::Drawing {
                active_outside_bounds,
                ..
            } => active_outside_bounds,
            _ => matches!(
                self,
                Self::YAxisScale { .. } | Self::XAxisScale { .. } | Self::Marquee { .. }
            ),
        }
    }

    pub(super) fn selection_disposition(self) -> SelectionDisposition {
        match self {
            Self::Layer { selection, .. } | Self::Drawing { selection, .. } => selection,
            Self::Pan { .. } | Self::Marquee { .. } => SelectionDisposition::Preserve,
            _ => SelectionDisposition::Clear,
        }
    }

    pub(super) fn cursor_appearance(self) -> Option<Interaction> {
        match self {
            Self::Split(_) | Self::YAxisScale { .. } => Some(Interaction::ResizingVertically),
            Self::Layer { cursor, .. } | Self::Drawing { cursor, .. } => Some(cursor),
            Self::XAxisScale { .. } => Some(Interaction::ResizingHorizontally),
            Self::Pan { .. } => Some(Interaction::Grabbing),
            Self::Marquee { .. } => None,
        }
    }

    pub(super) fn layer_key(self) -> Option<StudyObjectKey> {
        match self {
            Self::Layer { key, .. } => Some(key),
            Self::Pan { source_layer, .. } | Self::Marquee { source_layer, .. } => source_layer,
            _ => None,
        }
    }

    pub(super) fn is_drawing(self) -> bool {
        matches!(self, Self::Drawing { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneActivity {
    Idle,
    Pan,
    Drag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HorizontalWheelZoomMode {
    LiveEdge,
    Cursor,
}

impl HorizontalWheelZoomMode {
    pub(super) fn from_modifiers(modifiers: keyboard::Modifiers) -> Self {
        if modifiers.intersects(keyboard::Modifiers::COMMAND) {
            Self::Cursor
        } else {
            Self::LiveEdge
        }
    }
}

pub(super) struct LayerEffects<E> {
    pub(super) events: Vec<E>,
    pub(super) render_request: RenderRequest,
    pub(super) status: Status,
    pub(super) gesture: Option<LayerGesture>,
}

impl<E> Default for LayerEffects<E> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            render_request: RenderRequest::None,
            status: Status::Ignored,
            gesture: None,
        }
    }
}

impl<E> LayerEffects<E> {
    pub(super) fn captured(render_request: RenderRequest) -> Self {
        Self {
            render_request,
            status: Status::Captured,
            ..Self::default()
        }
    }

    pub(super) fn publish(&mut self, event: E) {
        self.events.push(event);
    }

    pub(super) fn map_events<T>(self, mut map: impl FnMut(E) -> T) -> LayerEffects<T> {
        LayerEffects {
            events: self.events.into_iter().map(&mut map).collect(),
            render_request: self.render_request,
            status: self.status,
            gesture: self.gesture,
        }
    }
}

const DRAG_THRESHOLD_PX: f32 = 5.0;

#[derive(Debug, Clone, Copy)]
enum PressIntent<C, D> {
    Click(C),
    Drag(D),
    ClickOrDrag { click: C, drag: D },
}

impl<C, D: Copy> PressIntent<C, D> {
    fn drag_action(&self) -> Option<D> {
        match self {
            Self::Drag(action) | Self::ClickOrDrag { drag: action, .. } => Some(*action),
            Self::Click(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PointerMotion {
    pub(super) origin: Point,
    pub(super) current: Point,
}

impl PointerMotion {
    fn new(origin: Point) -> Self {
        Self {
            origin,
            current: origin,
        }
    }

    fn delta(&mut self, current: Point) -> Vector {
        let delta = Vector::new(
            (current.x - self.origin.x).round() - (self.current.x - self.origin.x).round(),
            (current.y - self.origin.y).round() - (self.current.y - self.origin.y).round(),
        );
        self.current = current;
        delta
    }

    fn drag_threshold_reached(&self, current: Point) -> bool {
        (self.origin.x - current.x).abs() + (self.origin.y - current.y).abs() >= DRAG_THRESHOLD_PX
    }
}

#[derive(Debug, Clone, Copy)]
struct PressGesture<C, D> {
    motion: PointerMotion,
    intent: PressIntent<C, D>,
}

#[derive(Debug, Clone, Copy)]
struct DragGesture<C, D> {
    motion: PointerMotion,
    action: D,
    press_action: Option<C>,
}

#[derive(Debug, Clone, Copy)]
enum PointerGesture<C, D> {
    Idle,
    Pressed(PressGesture<C, D>),
    Dragging(DragGesture<C, D>),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum PointerMove {
    Idle,
    Pending,
    Moving { delta: Vector },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClickOutcome {
    Confirmed,
    Cancelled,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ReleasedGesture<C, D> {
    Idle,
    Pressed {
        action: C,
        click: ClickOutcome,
    },
    Drag {
        action: D,
        origin: Point,
        position: Option<Point>,
        press_action: Option<C>,
    },
}

pub(super) struct UserInteraction<C, D> {
    pointer: PointerGesture<C, D>,
    previous_click: Option<iced_core::mouse::Click>,
    modifiers: keyboard::Modifiers,
}

impl<C, D> Default for UserInteraction<C, D> {
    fn default() -> Self {
        Self {
            pointer: PointerGesture::Idle,
            previous_click: None,
            modifiers: keyboard::Modifiers::empty(),
        }
    }
}

impl<C: Copy, D: Copy> UserInteraction<C, D> {
    pub(super) fn begin_press(
        &mut self,
        position: Point,
        global_position: Option<Point>,
        action: C,
    ) -> Option<iced_core::mouse::click::Kind> {
        self.begin_click(position, action);

        let click = global_position.map(|position| {
            iced_core::mouse::Click::new(
                position,
                iced_core::mouse::Button::Left,
                self.previous_click,
            )
        });
        let kind = click.as_ref().map(iced_core::mouse::Click::kind);
        self.previous_click = click;
        kind
    }

    pub(super) fn begin_click(&mut self, position: Point, action: C) {
        self.press(position, PressIntent::Click(action));
    }

    pub(super) fn begin_drag_candidate(&mut self, position: Point, action: D) {
        self.press(position, PressIntent::Drag(action));
    }

    pub(super) fn begin_click_or_drag(&mut self, position: Point, click: C, drag: D) {
        self.press(position, PressIntent::ClickOrDrag { click, drag });
    }

    fn press(&mut self, position: Point, intent: PressIntent<C, D>) {
        self.pointer = PointerGesture::Pressed(PressGesture {
            motion: PointerMotion::new(position),
            intent,
        });
    }

    pub(super) fn pointer_moved(&mut self, position: Point) -> PointerMove {
        let press = match &mut self.pointer {
            PointerGesture::Pressed(press) => *press,
            PointerGesture::Dragging(drag) => {
                return PointerMove::Moving {
                    delta: drag.motion.delta(position),
                };
            }
            PointerGesture::Idle => return PointerMove::Idle,
        };
        let Some(action) = press.intent.drag_action() else {
            return PointerMove::Idle;
        };
        if !press.motion.drag_threshold_reached(position) {
            return PointerMove::Pending;
        }

        let mut motion = press.motion;
        let delta = motion.delta(position);
        let press_action = match press.intent {
            PressIntent::Click(action) | PressIntent::ClickOrDrag { click: action, .. } => {
                Some(action)
            }
            PressIntent::Drag(_) => None,
        };
        self.pointer = PointerGesture::Dragging(DragGesture {
            motion,
            action,
            press_action,
        });
        PointerMove::Moving { delta }
    }

    pub(super) fn release(&mut self, position: Option<Point>) -> ReleasedGesture<C, D> {
        match std::mem::replace(&mut self.pointer, PointerGesture::Idle) {
            PointerGesture::Idle => ReleasedGesture::Idle,
            PointerGesture::Pressed(press) => match press.intent {
                PressIntent::Click(action) | PressIntent::ClickOrDrag { click: action, .. } => {
                    let click = if position
                        .is_some_and(|current| !press.motion.drag_threshold_reached(current))
                    {
                        ClickOutcome::Confirmed
                    } else {
                        ClickOutcome::Cancelled
                    };
                    ReleasedGesture::Pressed { action, click }
                }
                PressIntent::Drag(action) => ReleasedGesture::Drag {
                    action,
                    origin: press.motion.origin,
                    position,
                    press_action: None,
                },
            },
            PointerGesture::Dragging(drag) => ReleasedGesture::Drag {
                action: drag.action,
                origin: drag.motion.origin,
                position,
                press_action: drag.press_action,
            },
        }
    }

    pub(super) fn cancel_pointer(&mut self) -> Option<C> {
        match std::mem::replace(&mut self.pointer, PointerGesture::Idle) {
            PointerGesture::Idle => None,
            PointerGesture::Pressed(press) => match press.intent {
                PressIntent::Click(action) | PressIntent::ClickOrDrag { click: action, .. } => {
                    Some(action)
                }
                PressIntent::Drag(_) => None,
            },
            PointerGesture::Dragging(drag) => drag.press_action,
        }
    }

    pub(super) fn press_action(&self) -> Option<C> {
        match self.pointer {
            PointerGesture::Pressed(press) => match press.intent {
                PressIntent::Click(action) | PressIntent::ClickOrDrag { click: action, .. } => {
                    Some(action)
                }
                PressIntent::Drag(_) => None,
            },
            PointerGesture::Dragging(drag) => drag.press_action,
            PointerGesture::Idle => None,
        }
    }

    pub(super) fn modifiers(&self) -> keyboard::Modifiers {
        self.modifiers
    }

    pub(super) fn set_modifiers(&mut self, modifiers: keyboard::Modifiers) {
        self.modifiers = modifiers;
    }

    pub(super) fn drag_action(&self) -> Option<D> {
        match self.pointer {
            PointerGesture::Pressed(press) => press.intent.drag_action(),
            PointerGesture::Dragging(drag) => Some(drag.action),
            PointerGesture::Idle => None,
        }
    }

    pub(super) fn motion(&self) -> Option<PointerMotion> {
        match self.pointer {
            PointerGesture::Pressed(press) => Some(press.motion),
            PointerGesture::Dragging(drag) => Some(drag.motion),
            PointerGesture::Idle => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum GestureSource {
    Drawing,
    Study(StudyObjectKey),
}

impl UserInteraction<PressAction, DragAction> {
    pub(super) fn resolve_cursor_target(
        &self,
        target: CursorTarget,
        zone: LayoutHitZone,
        drawings: &DrawingSnapshot<'_>,
    ) -> CursorTarget {
        if (self.motion().is_some()
            || !drawings.allows_panning()
            || matches!(drawings.measure, Some(DrawingMeasure::Pending(_)))
            || self
                .modifiers
                .intersects(keyboard::Modifiers::SHIFT | keyboard::Modifiers::COMMAND))
            && matches!(
                target,
                CursorTarget::Legend(_)
                    | CursorTarget::CornerControl(_)
                    | CursorTarget::PanelControl { .. }
            )
            && let LayoutHitZone::PanelPlot(panel_index) = zone
        {
            CursorTarget::Plot(panel_index)
        } else {
            target
        }
    }

    pub(super) fn begin_layer_gesture(
        &mut self,
        gesture: LayerGesture,
        source: GestureSource,
        position: Point,
        zone: LayoutHitZone,
    ) -> bool {
        let press_action = |selection, captures_outside_release| match source {
            GestureSource::Drawing => PressAction::Drawing {
                selection,
                captures_outside_release,
            },
            GestureSource::Study(key) => PressAction::Layer {
                key,
                selection,
                captures_outside_release,
            },
        };
        match gesture {
            LayerGesture::Click {
                selection,
                captures_outside_release,
            } => {
                self.begin_click(position, press_action(selection, captures_outside_release));
            }
            LayerGesture::Drag {
                selection,
                active_outside_bounds,
                cursor,
            } => {
                let action = match source {
                    GestureSource::Drawing => DragAction::Drawing {
                        selection,
                        active_outside_bounds,
                        cursor,
                    },
                    GestureSource::Study(key) => DragAction::Layer {
                        key,
                        selection,
                        active_outside_bounds,
                        cursor,
                    },
                };
                self.begin_drag_candidate(position, action);
            }
            LayerGesture::ClickOrNavigate {
                selection,
                captures_outside_release,
                fallback,
            } => {
                let LayoutHitZone::PanelPlot(panel_index) = zone else {
                    return false;
                };
                let source_layer = match source {
                    GestureSource::Drawing => None,
                    GestureSource::Study(key) => Some(key),
                };
                let drag = match fallback {
                    NavigationFallback::PanOrMarquee
                        if self.modifiers().intersects(keyboard::Modifiers::COMMAND) =>
                    {
                        DragAction::Marquee {
                            panel_index,
                            source_layer,
                        }
                    }
                    _ => DragAction::Pan {
                        panel_index,
                        source_layer,
                    },
                };
                self.begin_click_or_drag(
                    position,
                    press_action(selection, captures_outside_release),
                    drag,
                );
            }
        }
        true
    }
}
