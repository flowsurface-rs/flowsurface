use crate::widget::chart::kline::PanelYViewport;
use crate::widget::chart::kline::composition::PanelId;

#[derive(Default)]
pub(super) struct PanelViewportStore {
    values: Vec<(PanelId, PanelYViewport)>,
}

impl PanelViewportStore {
    pub(super) fn replace(&mut self, values: impl IntoIterator<Item = (PanelId, PanelYViewport)>) {
        self.values.clear();
        for (panel_id, viewport) in values {
            let _ = self.set(panel_id, viewport);
        }
    }

    pub(super) fn as_slice(&self) -> &[(PanelId, PanelYViewport)] {
        &self.values
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &(PanelId, PanelYViewport)> {
        self.values.iter()
    }

    pub(super) fn set(&mut self, panel_id: PanelId, viewport: PanelYViewport) -> bool {
        if let Some((_, current)) = self
            .values
            .iter_mut()
            .find(|(current_id, _)| *current_id == panel_id)
        {
            if *current == viewport {
                return false;
            }

            *current = viewport;
            return true;
        }

        self.values.push((panel_id, viewport));
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
