use super::helpers::ChartCoordinates;
use super::scene::Scene;
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
pub(super) enum CacheInvalidation {
    None,
    Overlay,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavigationFallback {
    Pan,
    PanOrMarquee,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum LayerGesture {
    Click {
        selection: super::SelectionDisposition,
        captures_outside_release: bool,
    },
    Drag {
        selection: super::SelectionDisposition,
        active_outside_bounds: bool,
        cursor: Interaction,
    },
    ClickOrNavigate {
        selection: super::SelectionDisposition,
        captures_outside_release: bool,
        fallback: NavigationFallback,
    },
}

pub(super) struct LayerEffects<E> {
    pub(super) events: Vec<E>,
    pub(super) invalidation: CacheInvalidation,
    pub(super) status: Status,
    pub(super) gesture: Option<LayerGesture>,
}

impl<E> Default for LayerEffects<E> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            invalidation: CacheInvalidation::None,
            status: Status::Ignored,
            gesture: None,
        }
    }
}

impl<E> LayerEffects<E> {
    pub(super) fn captured(invalidation: CacheInvalidation) -> Self {
        Self {
            invalidation,
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
            invalidation: self.invalidation,
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
