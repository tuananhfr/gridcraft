//! When a host document's edits are due for AutoSave (rule 3): once editing pauses, and as a
//! checkpoint while edits keep coming without a pause.

pub(crate) const IDLE_MS: f64 = 3_000.0;
pub(crate) const CHECKPOINT_MS: f64 = 30_000.0;

/// Edits made since the last save was started (or queued behind the one in flight).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Timing {
    last_edit: Option<f64>,
    first_edit: Option<f64>,
}

impl Timing {
    pub(crate) fn edited(&mut self, now: f64) {
        self.last_edit = Some(now);
        self.first_edit.get_or_insert(now);
    }

    pub(crate) fn started(&mut self) {
        *self = Timing::default();
    }

    /// Milliseconds until a save is due; `Some(0.0)` when it is due now, `None` without edits.
    pub(crate) fn due_in(&self, now: f64) -> Option<f64> {
        let (last, first) = (self.last_edit?, self.first_edit?);
        Some((last + IDLE_MS).min(first + CHECKPOINT_MS) - now).map(|ms| ms.max(0.0))
    }

    pub(crate) fn due(&self, now: f64) -> bool {
        self.due_in(now) == Some(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_three_seconds_after_the_last_edit() {
        let mut t = Timing::default();
        assert_eq!(t.due_in(0.0), None);
        t.edited(1_000.0);
        assert!(!t.due(3_999.0));
        t.edited(3_000.0);
        assert!(!t.due(5_999.0), "a later edit restarts the pause");
        assert!(t.due(6_000.0));
        t.started();
        assert_eq!(t.due_in(10_000.0), None);
    }

    #[test]
    fn continuous_editing_saves_every_thirty_seconds() {
        let mut t = Timing::default();
        let mut now = 0.0;
        while now < CHECKPOINT_MS {
            t.edited(now);
            assert!(!t.due(now), "no pause yet at {now}");
            now += 1_000.0;
        }
        t.edited(now);
        assert!(t.due(now));
    }
}
