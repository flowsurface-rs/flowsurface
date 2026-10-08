use super::painting::PaintInputs;
mod big_trades;
mod tpo;
mod volume_profile;

use super::interaction::{LayerContext, LayerEffects};
use super::painting::PixelGeometry;
pub(crate) use big_trades::BigTradesObject;
use data::chart::kline::KlineStudyId;
use data::config::theme::ACCENT_COUNT;
use iced::advanced::mouse::Interaction;
use iced::theme::palette::Extended;
use rustc_hash::FxHashMap;
pub(crate) use tpo::{TpoInteraction, TpoObject};
pub(crate) use volume_profile::{ProfileBarSource, VolumeProfileObject};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StudyObjectKey {
    Study(KlineStudyId),
    VolumeProfiles,
    BigTrades,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerHit {
    pub(super) key: StudyObjectKey,
    pub(super) priority: i16,
    pub(super) precedes_tools: bool,
    pub(super) readouts_visible: bool,
    pub(super) cursor: Interaction,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum LayerPointerInput {
    /// Clear selection outside the target after a confirmed plot click.
    PlotClicked {
        target: Option<StudyObjectKey>,
    },
    Pressed,
    Moved,
    Clicked {
        confirmed: bool,
    },
    DragReleased,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayerPaintPhase {
    Underlay,
    BeforeIndicators,
    AfterIndicators,
    Overlay,
}

#[derive(Debug, Clone, Copy)]
pub enum StudyObjectEvent {
    Tpo(TpoInteraction),
}

pub(crate) enum StudyObject<'a> {
    Tpo(TpoObject<'a>),
    VolumeProfiles(VolumeProfileObject<'a>),
    BigTrades(BigTradesObject),
}

pub(super) enum StudyObjectState {
    Tpo(tpo::TpoState),
    VolumeProfiles(volume_profile::VolumeProfileState),
}

impl StudyObject<'_> {
    fn key(&self) -> StudyObjectKey {
        match self {
            Self::Tpo(object) => object.key(),
            Self::VolumeProfiles(_) => StudyObjectKey::VolumeProfiles,
            Self::BigTrades(_) => StudyObjectKey::BigTrades,
        }
    }

    fn new_state(&self) -> Option<StudyObjectState> {
        match self {
            Self::Tpo(_) => Some(StudyObjectState::Tpo(tpo::TpoState::default())),
            Self::VolumeProfiles(_) => Some(StudyObjectState::VolumeProfiles(
                volume_profile::VolumeProfileState::default(),
            )),
            Self::BigTrades(_) => None,
        }
    }

    pub(super) fn matches_state(&self, state: &StudyObjectState) -> bool {
        matches!(
            (self, state),
            (Self::Tpo(_), StudyObjectState::Tpo(_))
                | (Self::VolumeProfiles(_), StudyObjectState::VolumeProfiles(_))
        )
    }

    fn hit_test(
        &self,
        context: LayerContext<'_>,
        state: Option<&StudyObjectState>,
    ) -> Option<LayerHit> {
        match (self, state) {
            (Self::Tpo(object), Some(StudyObjectState::Tpo(state))) => {
                object.hit_test(context, state)
            }
            _ => None,
        }
    }

    fn pointer(
        &self,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        state: Option<&mut StudyObjectState>,
    ) -> LayerEffects<StudyObjectEvent> {
        match (self, state) {
            (Self::Tpo(object), Some(StudyObjectState::Tpo(state))) => object
                .pointer(input, context, state)
                .map_events(StudyObjectEvent::Tpo),
            _ => LayerEffects::default(),
        }
    }

    fn paint(
        &self,
        phase: LayerPaintPhase,
        context: LayerContext<'_>,
        state: Option<&StudyObjectState>,
        frame: &mut PaintInputs,
        palette: &Extended,
        accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        match self {
            Self::Tpo(object) => {
                if let Some(StudyObjectState::Tpo(state)) = state {
                    object.paint(phase, context, state, frame, palette, accents, pixels);
                }
            }
            Self::VolumeProfiles(object) => {
                if let Some(StudyObjectState::VolumeProfiles(state)) = state {
                    object.paint(phase, context, state, frame, palette, accents, pixels);
                }
            }
            Self::BigTrades(object) => {
                object.paint(phase, context, frame, palette, accents, pixels);
            }
        }
    }
}

pub(super) type StudyStates = FxHashMap<StudyObjectKey, StudyObjectState>;

#[derive(Default)]
pub(super) struct StudyObjects<'a> {
    objects: Vec<StudyObject<'a>>,
}

impl<'a> StudyObjects<'a> {
    pub(super) fn register(&mut self, object: StudyObject<'a>) {
        let key = object.key();
        assert!(
            self.objects
                .iter()
                .all(|registered| registered.key() != key)
        );
        self.objects.push(object);
    }

    pub(super) fn get(&self, key: StudyObjectKey) -> Option<&StudyObject<'a>> {
        self.objects.iter().find(|object| object.key() == key)
    }

    pub(super) fn hit_test(
        &self,
        context: LayerContext<'_>,
        states: &StudyStates,
    ) -> Option<LayerHit> {
        self.objects
            .iter()
            .filter_map(|object| object.hit_test(context, states.get(&object.key())))
            .max_by_key(|hit| hit.priority)
    }

    pub(super) fn pointer(
        &self,
        key: StudyObjectKey,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        states: &mut StudyStates,
    ) -> Option<LayerEffects<StudyObjectEvent>> {
        let object = self.get(key)?;
        Some(object.pointer(input, context, states.get_mut(&key)))
    }

    pub(super) fn pointer_all(
        &self,
        input: LayerPointerInput,
        context: LayerContext<'_>,
        states: &mut StudyStates,
    ) -> Vec<LayerEffects<StudyObjectEvent>> {
        self.objects
            .iter()
            .map(|object| object.pointer(input, context, states.get_mut(&object.key())))
            .collect()
    }

    pub(super) fn paint(
        &self,
        phase: LayerPaintPhase,
        context: LayerContext<'_>,
        states: &StudyStates,
        frame: &mut PaintInputs,
        palette: &Extended,
        accents: &[iced::Color; ACCENT_COUNT],
        pixels: PixelGeometry,
    ) {
        for object in &self.objects {
            object.paint(
                phase,
                context,
                states.get(&object.key()),
                frame,
                palette,
                accents,
                pixels,
            );
        }
    }

    pub(super) fn prepare_controls(
        &self,
        context: LayerContext<'_>,
        states: &StudyStates,
        controls: &mut super::chrome::PreparedChrome,
        palette: &Extended,
        pixels: PixelGeometry,
    ) {
        for object in &self.objects {
            if let StudyObject::Tpo(object) = object
                && let Some(StudyObjectState::Tpo(state)) = states.get(&object.key())
            {
                object.prepare_controls(context, state, controls, palette, pixels);
            }
        }
    }

    pub(super) fn reconcile_states(&self, states: &mut StudyStates) {
        states.retain(|key, state| {
            self.objects
                .iter()
                .find(|object| object.key() == *key)
                .is_some_and(|object| object.matches_state(state))
        });
        for object in &self.objects {
            if let Some(new_state) = object.new_state() {
                states.entry(object.key()).or_insert(new_state);
            }
        }
    }
}
