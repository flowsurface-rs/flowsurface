use super::drawing::{DrawingEditState, DrawingTools, DrawingUpdate};
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
    pub(super) fn apply_update(
        &mut self,
        update: DrawingUpdate,
        before: DrawingEditState,
        tools: &DrawingTools,
    ) {
        match update {
            DrawingUpdate::AtomicEdit => {
                self.finish_drawing(|| before.clone());
                let after = tools.edit_state();
                if before != after {
                    self.record(before, after);
                }
            }
            DrawingUpdate::GestureStarted { .. } => {
                self.finish_drawing(|| before.clone());
                self.begin_drawing(before);
            }
            DrawingUpdate::GestureUpdated => {}
            DrawingUpdate::GestureFinished { .. } => {
                self.finish_drawing(|| tools.edit_state());
            }
            DrawingUpdate::None | DrawingUpdate::StateOnly | DrawingUpdate::Visual => {
                self.finish_if_idle(tools);
            }
        }
    }

    pub(super) fn record(&mut self, before: DrawingEditState, after: DrawingEditState) {
        self.pending_drawing = None;
        self.redo.clear();
        if self.undo.len() == HISTORY_LIMIT {
            let _ = self.undo.pop_front();
        }
        self.undo.push_back(DrawingEdit { before, after });
    }

    fn begin_drawing(&mut self, before: DrawingEditState) {
        self.pending_drawing = Some(before);
    }

    pub(super) fn finish_drawing(&mut self, after: impl FnOnce() -> DrawingEditState) {
        let Some(before) = self.pending_drawing.take() else {
            return;
        };
        let after = after();
        if before != after {
            self.record(before, after);
        }
    }

    pub(super) fn finish_if_idle(&mut self, tools: &DrawingTools) {
        if !tools.is_dragging() {
            self.finish_drawing(|| tools.edit_state());
        }
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
