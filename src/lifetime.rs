//! How long an editor stays. An untouched editor leaves on its own: on a click outside it, on the
//! next capture, and when its clock runs out, unless the pointer has come onto it. It opens with
//! the keyboard and is still untouched; entering it, with a click, a wheel turn or a key, keeps it
//! until the user closes it.

/// Where an editor is in its lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// Untouched, with the clock running. `away` once the pointer has been off the editor since it
    /// opened, so the pointer coming onto it next has moved onto it.
    Open { away: bool },
    /// Untouched, and the pointer has come onto it, or a key is copying, saving or closing it: the
    /// clock has stopped for good.
    Seen,
    /// Untouched, and the clock ran out at the tick count `since`: it is fading out.
    Fading { since: u32 },
    /// Entered: it stays until the user closes it.
    Entered,
}

/// What can close an untouched editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    ClickOutside,
    NewCapture,
    Clock,
}

/// A key going down on an editor, as its lifetime sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Ctrl, Shift, Alt or Windows on its own, which does nothing yet.
    Modifier,
    /// A key that copies, saves or closes the editor, which then closes it untouched or not.
    Closes,
    /// Any other key.
    Other,
}

impl Lifetime {
    /// Just opened, with the pointer on the editor or off it. It has the keyboard, and is still
    /// untouched.
    pub fn opened(pointer_on: bool) -> Lifetime {
        Lifetime::Open { away: !pointer_on }
    }

    /// The pointer is now on the editor or off it. True if that stops the clock.
    pub fn pointer(&mut self, on: bool) -> bool {
        match *self {
            Lifetime::Open { away: true } if on => {
                *self = Lifetime::Seen;
                true
            }
            Lifetime::Open { .. } if !on => {
                *self = Lifetime::Open { away: true };
                false
            }
            _ => false,
        }
    }

    /// The clock ran out at the tick count `now`. True if the editor starts fading out.
    pub fn clock_out(&mut self, now: u32) -> bool {
        if !self.closes_on(Cause::Clock) {
            return false;
        }
        *self = Lifetime::Fading { since: now };
        true
    }

    /// The user clicked or turned the wheel on the editor, fading or not. True if that entered it.
    pub fn enter(&mut self) -> bool {
        let untouched = !self.entered();
        *self = Lifetime::Entered;
        untouched
    }

    /// `key` went down on the editor. A key is as deliberate as a click, so it enters it, except a
    /// modifier on its own, and a key that copies, saves or closes it, which closes it without
    /// entering it first: that one only stops the clock, so the editor waits for it. True if that
    /// entered it.
    pub fn key(&mut self, key: Key) -> bool {
        match key {
            Key::Modifier => false,
            Key::Closes => {
                if let Lifetime::Open { .. } = self {
                    *self = Lifetime::Seen;
                }
                false
            }
            Key::Other => self.enter(),
        }
    }

    /// Whether the user has entered the editor.
    pub fn entered(self) -> bool {
        self == Lifetime::Entered
    }

    /// Whether the clock is running, so the editor needs to know when the pointer leaves it.
    pub fn clock_running(self) -> bool {
        matches!(self, Lifetime::Open { .. })
    }

    /// Whether `cause` closes the editor now.
    pub fn closes_on(self, cause: Cause) -> bool {
        if self.entered() {
            return false;
        }
        match cause {
            Cause::ClickOutside | Cause::NewCapture => true,
            Cause::Clock => self.clock_running(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opened_editor_has_the_keyboard_and_is_untouched() {
        for pointer_on in [false, true] {
            let opened = Lifetime::opened(pointer_on);
            assert!(!opened.entered());
            assert!(opened.closes_on(Cause::ClickOutside));
            assert!(opened.closes_on(Cause::NewCapture));
            assert!(opened.closes_on(Cause::Clock));
        }
    }

    #[test]
    fn a_key_enters_an_untouched_editor() {
        let open = Lifetime::opened(false);
        let seen = Lifetime::Seen;
        let fading = Lifetime::Fading { since: 8000 };
        for mut lifetime in [open, seen, fading] {
            assert!(lifetime.key(Key::Other));
            assert_eq!(lifetime, Lifetime::Entered);
            assert!(!lifetime.key(Key::Other));
        }
    }

    #[test]
    fn a_modifier_on_its_own_changes_nothing() {
        let open = Lifetime::opened(false);
        let seen = Lifetime::Seen;
        let fading = Lifetime::Fading { since: 8000 };
        for before in [open, seen, fading, Lifetime::Entered] {
            let mut lifetime = before;
            assert!(!lifetime.key(Key::Modifier));
            assert_eq!(lifetime, before);
        }
    }

    #[test]
    fn a_key_that_copies_saves_or_closes_stops_the_clock_and_leaves_the_editor_untouched() {
        let mut lifetime = Lifetime::opened(false);
        assert!(!lifetime.key(Key::Closes));
        assert_eq!(lifetime, Lifetime::Seen);
        assert!(!lifetime.closes_on(Cause::Clock));
        assert!(lifetime.closes_on(Cause::ClickOutside));
        assert!(lifetime.closes_on(Cause::NewCapture));

        let mut entered = Lifetime::Entered;
        assert!(!entered.key(Key::Closes));
        assert_eq!(entered, Lifetime::Entered);
    }

    #[test]
    fn the_pointer_moving_onto_an_editor_stops_its_clock_for_good() {
        let mut lifetime = Lifetime::opened(false);
        assert!(lifetime.pointer(true));
        assert!(!lifetime.pointer(false));
        assert!(!lifetime.pointer(true));
        assert!(!lifetime.closes_on(Cause::Clock));
        assert!(lifetime.closes_on(Cause::ClickOutside));
        assert!(lifetime.closes_on(Cause::NewCapture));
    }

    #[test]
    fn an_editor_opened_under_the_pointer_keeps_its_clock_until_the_pointer_leaves_and_comes_back()
    {
        let mut lifetime = Lifetime::opened(true);
        assert!(!lifetime.pointer(true));
        assert!(lifetime.closes_on(Cause::Clock));
        assert!(!lifetime.pointer(false));
        assert!(lifetime.closes_on(Cause::Clock));
        assert!(lifetime.pointer(true));
        assert!(!lifetime.closes_on(Cause::Clock));
    }

    #[test]
    fn the_clock_running_out_fades_an_editor_that_a_click_outside_or_a_new_capture_closes_at_once()
    {
        let mut lifetime = Lifetime::opened(false);
        assert!(lifetime.clock_out(8000));
        assert_eq!(lifetime, Lifetime::Fading { since: 8000 });
        assert!(!lifetime.clock_out(8100));
        assert!(!lifetime.pointer(true));
        assert!(!lifetime.key(Key::Closes));
        assert_eq!(lifetime, Lifetime::Fading { since: 8000 });
        assert!(!lifetime.closes_on(Cause::Clock));
        assert!(lifetime.closes_on(Cause::ClickOutside));
        assert!(lifetime.closes_on(Cause::NewCapture));
    }

    #[test]
    fn the_clock_does_nothing_once_the_pointer_has_been_seen() {
        let mut lifetime = Lifetime::opened(false);
        lifetime.pointer(true);
        assert!(!lifetime.clock_out(8000));
        assert_eq!(lifetime, Lifetime::Seen);
    }

    #[test]
    fn an_entered_editor_stays_whatever_happens() {
        let open = Lifetime::opened(true);
        let seen = Lifetime::Seen;
        let fading = Lifetime::Fading { since: 8000 };
        for mut lifetime in [open, seen, fading] {
            assert!(lifetime.enter());
            assert_eq!(lifetime, Lifetime::Entered);
            assert!(!lifetime.enter());
            assert!(!lifetime.pointer(false));
            assert!(!lifetime.pointer(true));
            assert!(!lifetime.clock_out(8000));
            assert_eq!(lifetime, Lifetime::Entered);
            for cause in [Cause::ClickOutside, Cause::NewCapture, Cause::Clock] {
                assert!(!lifetime.closes_on(cause));
            }
        }
    }
}
