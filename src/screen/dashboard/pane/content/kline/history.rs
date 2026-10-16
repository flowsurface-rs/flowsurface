use super::drawing::DrawingEditState;
use std::collections::VecDeque;

const HISTORY_LIMIT: usize = 100;

pub(super) struct DrawingEdit {
    pub(super) before: DrawingEditState,
    pub(super) after: DrawingEditState,
}

#[derive(Default)]
pub(super) struct DrawingHistory {
    undo: VecDeque<DrawingEdit>,
    redo: Vec<DrawingEdit>,
    pending_drawing: Option<DrawingEditState>,
}

impl DrawingHistory {
    pub(super) fn record(&mut self, before: DrawingEditState, after: DrawingEditState) {
        self.pending_drawing = None;
        self.redo.clear();
        if self.undo.len() == HISTORY_LIMIT {
            let _ = self.undo.pop_front();
        }
        self.undo.push_back(DrawingEdit { before, after });
    }

    pub(super) fn begin_drawing(&mut self, before: DrawingEditState) {
        self.pending_drawing = Some(before);
    }

    pub(super) fn finish_drawing(&mut self, after: DrawingEditState) {
        let Some(before) = self.pending_drawing.take() else {
            return;
        };
        if before != after {
            self.record(before, after);
        }
    }

    pub(super) fn has_pending_drawing(&self) -> bool {
        self.pending_drawing.is_some()
    }

    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending_drawing = None;
    }

    pub(super) fn pop_undo(&mut self) -> Option<DrawingEdit> {
        self.undo.pop_back()
    }

    pub(super) fn push_redo(&mut self, entry: DrawingEdit) {
        self.redo.push(entry);
    }

    pub(super) fn pop_redo(&mut self) -> Option<DrawingEdit> {
        self.redo.pop()
    }

    pub(super) fn push_undo(&mut self, entry: DrawingEdit) {
        if self.undo.len() == HISTORY_LIMIT {
            let _ = self.undo.pop_front();
        }
        self.undo.push_back(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::super::drawing::DrawingTools;
    use super::*;

    #[test]
    fn drawing_history_skips_unchanged_gestures_and_clears_redo_on_edit() {
        let state = DrawingTools::default().edit_state();
        let mut history = DrawingHistory::default();

        history.begin_drawing(state.clone());
        history.finish_drawing(state.clone());
        assert!(history.pop_undo().is_none());

        history.record(state.clone(), state.clone());
        let entry = history.pop_undo().unwrap();
        history.push_redo(entry);
        history.record(state.clone(), state);
        assert!(history.pop_redo().is_none());
        assert!(history.pop_undo().is_some());
    }
}
