//! What undo and redo bring back: the editor as it was before each edit, and as it was before
//! each undo, as whatever state the editor keeps of itself.

pub struct History<S> {
    /// The state before each edit, oldest first; undoing restores the last.
    past: Vec<S>,
    /// The state before each undo, the latest last; redoing restores the last.
    undone: Vec<S>,
    /// Whether the last thing done was a nudge, which the next nudge joins.
    nudging: bool,
}

impl<S> History<S> {
    pub fn new() -> History<S> {
        History {
            past: Vec::new(),
            undone: Vec::new(),
            nudging: false,
        }
    }

    /// An edit is about to be made from `now`. What could have been redone is gone.
    pub fn record(&mut self, now: S) {
        self.past.push(now);
        self.undone.clear();
        self.nudging = false;
    }

    /// A nudge is about to be made from `now`. A run of nudges with nothing else between them is
    /// one edit, made from where the run began.
    pub fn nudge(&mut self, now: S) {
        if !self.nudging {
            self.record(now);
            self.nudging = true;
        }
    }

    /// Something other than a nudge happened, so the next nudge starts a new edit.
    pub fn interrupt(&mut self) {
        self.nudging = false;
    }

    /// The state to go back to from `now`, if any edit is left to undo.
    pub fn undo(&mut self, now: S) -> Option<S> {
        self.nudging = false;
        let before = self.past.pop()?;
        self.undone.push(now);
        Some(before)
    }

    /// The state to go back to from the last edit, if any, taking it back as if it had never
    /// been made, so there is nothing to redo.
    pub fn take_back(&mut self) -> Option<S> {
        self.nudging = false;
        self.past.pop()
    }

    /// The state an undo left, to go forward to from `now`, if any.
    pub fn redo(&mut self, now: S) -> Option<S> {
        self.nudging = false;
        let after = self.undone.pop()?;
        self.past.push(now);
        Some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redo_brings_back_what_undo_took_away() {
        let mut history = History::new();
        history.record("empty");
        let undone = history.undo("arrow");
        assert_eq!(undone, Some("empty"));
        assert_eq!(history.redo("empty"), Some("arrow"));
    }

    #[test]
    fn redo_after_redo_walks_forward_through_every_undone_step() {
        let mut history = History::new();
        history.record("empty");
        history.record("one arrow");
        assert_eq!(history.undo("two arrows"), Some("one arrow"));
        assert_eq!(history.undo("one arrow"), Some("empty"));
        assert_eq!(history.redo("empty"), Some("one arrow"));
        assert_eq!(history.redo("one arrow"), Some("two arrows"));
        assert_eq!(history.redo("two arrows"), None);
        assert_eq!(history.undo("two arrows"), Some("one arrow"));
    }

    #[test]
    fn a_new_edit_after_an_undo_leaves_nothing_to_redo() {
        let mut history = History::new();
        history.record("empty");
        history.undo("arrow");
        history.record("empty");
        assert_eq!(history.redo("rectangle"), None);
        assert_eq!(history.undo("rectangle"), Some("empty"));
        assert_eq!(history.undo("empty"), None);
    }

    #[test]
    fn a_run_of_nudges_is_one_step() {
        let mut history = History::new();
        history.nudge(0);
        history.nudge(1);
        history.nudge(2);
        assert_eq!(history.undo(3), Some(0));
        assert_eq!(history.undo(0), None);
        assert_eq!(history.redo(0), Some(3));
    }

    #[test]
    fn anything_else_between_nudges_starts_a_new_step() {
        let mut history = History::new();
        history.nudge(0);
        history.nudge(1);
        history.interrupt();
        history.nudge(2);
        assert_eq!(history.undo(3), Some(2));
        assert_eq!(history.undo(2), Some(0));
    }

    #[test]
    fn an_edit_ends_a_run_of_nudges() {
        let mut history = History::new();
        history.nudge(0);
        history.record(1);
        history.nudge(2);
        assert_eq!(history.undo(3), Some(2));
        assert_eq!(history.undo(2), Some(1));
        assert_eq!(history.undo(1), Some(0));
    }

    #[test]
    fn undo_and_redo_end_a_run_of_nudges() {
        let mut history = History::new();
        history.nudge(0);
        history.undo(1);
        history.nudge(0);
        history.nudge(1);
        assert_eq!(history.undo(2), Some(0));
        history.redo(0);
        history.nudge(2);
        assert_eq!(history.undo(3), Some(2));
    }

    #[test]
    fn a_nudge_after_an_undo_leaves_nothing_to_redo() {
        let mut history = History::new();
        history.record(0);
        history.undo(1);
        history.nudge(0);
        assert_eq!(history.redo(1), None);
    }

    #[test]
    fn an_edit_taken_back_leaves_nothing_to_redo() {
        let mut history = History::new();
        history.record("empty");
        history.record("text");
        assert_eq!(history.take_back(), Some("text"));
        assert_eq!(history.redo("text"), None);
        assert_eq!(history.undo("text"), Some("empty"));
        assert_eq!(history.take_back(), None);
    }

    #[test]
    fn a_resize_is_one_step_undone_to_the_full_capture_exactly_and_redone() {
        use crate::{
            annotation::{Annotation, Arrow},
            image::Resize,
            style::{Colour, Sizes, Style},
        };
        use windows_numerics::Vector2;

        let arrow = Annotation::Arrow(
            Arrow::straight(
                Vector2 { X: 100.5, Y: 50.5 },
                Vector2 { X: 900.5, Y: 650.5 },
            ),
            Style {
                colour: Colour::Red,
                size: Sizes::FIRST_RUN.arrow,
            },
        );
        let full = ((2560, 1440), vec![arrow.clone()]);
        // What the editor does: keeps the state, then shrinks the capture and every annotation.
        let half = Resize::Half;
        let resized = (
            half.dimensions(2560, 1440),
            vec![arrow.resized(half.fraction())],
        );
        let mut history = History::new();
        history.record(full.clone());

        let undone = history.undo(resized.clone());
        assert_eq!(undone, Some(full.clone()));
        let redone = history.redo(full);
        assert_eq!(redone, Some(resized.clone()));
        assert_eq!(redone.unwrap().0, (1280, 720));
        // Twice compounds, and two undos go back to the full capture.
        history.record(resized.clone());
        let quarter = (
            half.dimensions(1280, 720),
            vec![arrow.resized(0.5).resized(0.5)],
        );
        assert_eq!(quarter.0, (640, 360));
        assert_eq!(
            history.undo(quarter).map(|state| state.0),
            Some((1280, 720))
        );
        assert_eq!(
            history.undo(resized).map(|state| state.0),
            Some((2560, 1440))
        );
    }

    #[test]
    fn redo_with_nothing_undone_does_nothing() {
        let mut history = History::new();
        assert_eq!(history.redo("empty"), None);
        history.record("empty");
        assert_eq!(history.redo("arrow"), None);
        assert_eq!(history.undo("arrow"), Some("empty"));
    }
}
