use super::{DEFAULT_HORIZONTAL_OFFSET_UNITS, DEFAULT_HORIZONTAL_SPACING_PX};
use crate::widget::chart::kline::{PrimaryViewportState, YUnitRange};
use data::chart::kline::{
    HorizontalWorld,
    composition::{ChartComposition, PanelId, PanelScaleMode},
};

pub(super) struct ViewportState {
    pub(super) horizontal_world: HorizontalWorld,
    pub(super) primary: PrimaryViewportState,
    pub(super) primary_scale_inverted: bool,
    pub(super) indicator_y_ranges: PanelViewportStore,
    pub(super) plot_width: Option<f32>,
}

impl ViewportState {
    pub(super) fn allows_panel_range(
        &self,
        composition: &ChartComposition,
        panel_id: PanelId,
    ) -> bool {
        composition.primary_panel_id() != Some(panel_id)
            || composition.panel_effective_scale_mode(panel_id)
                != Some(PanelScaleMode::PercentFromBase)
    }

    pub(super) fn set_panel_range(
        &mut self,
        composition: &ChartComposition,
        panel_id: PanelId,
        range: YUnitRange,
    ) -> bool {
        if !self.allows_panel_range(composition, panel_id) {
            return false;
        }
        if composition.primary_panel_id() == Some(panel_id) {
            self.primary.set_range(range)
        } else {
            self.indicator_y_ranges.set(panel_id, range)
        }
    }

    pub(super) fn reset_panel_range(
        &mut self,
        composition: &ChartComposition,
        panel_id: PanelId,
    ) -> bool {
        if !self.allows_panel_range(composition, panel_id) {
            return false;
        }
        if composition.primary_panel_id() == Some(panel_id) {
            self.primary.reset()
        } else {
            self.indicator_y_ranges.reset(panel_id)
        }
    }

    pub(super) fn toggle_primary_autoscale(&mut self, composition: &ChartComposition) -> bool {
        composition.primary_panel_id().is_some_and(|panel_id| {
            self.allows_panel_range(composition, panel_id) && self.primary.toggle()
        })
    }

    pub(super) fn reset_autoscale(&mut self) {
        self.primary.reset();
        self.indicator_y_ranges = PanelViewportStore::default();
    }
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            horizontal_world: HorizontalWorld::new(
                DEFAULT_HORIZONTAL_SPACING_PX,
                DEFAULT_HORIZONTAL_OFFSET_UNITS,
            ),
            primary: PrimaryViewportState::Auto,
            primary_scale_inverted: false,
            indicator_y_ranges: PanelViewportStore::default(),
            plot_width: None,
        }
    }
}

#[derive(Default)]
pub(super) struct PanelViewportStore {
    values: Vec<(PanelId, YUnitRange)>,
}

impl PanelViewportStore {
    pub(super) fn as_slice(&self) -> &[(PanelId, YUnitRange)] {
        &self.values
    }

    pub(super) fn set(&mut self, panel_id: PanelId, range: YUnitRange) -> bool {
        if let Some((_, current)) = self
            .values
            .iter_mut()
            .find(|(current_id, _)| *current_id == panel_id)
        {
            if *current == range {
                return false;
            }

            *current = range;
            return true;
        }

        self.values.push((panel_id, range));
        true
    }

    pub(super) fn reset(&mut self, panel_id: PanelId) -> bool {
        let Some(index) = self
            .values
            .iter()
            .position(|(current_id, _)| *current_id == panel_id)
        else {
            return false;
        };

        self.values.remove(index);
        true
    }

    pub(super) fn retain(&mut self, mut should_retain: impl FnMut(PanelId) -> bool) {
        self.values.retain(|(panel_id, _)| should_retain(*panel_id));
    }
}
