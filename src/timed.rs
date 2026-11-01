//! Capture screen in: a screen capture taken a chosen time after the click, from the tray menu's
//! submenu of three times.

/// A time under Capture screen in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Time {
    /// The id the tray menu's item carries.
    pub menu_id: usize,
    /// What the item reads.
    pub text: &'static str,
    /// How long after the click the capture is taken.
    pub ms: u32,
}

/// The submenu's items, in order.
pub const TIMES: [Time; 3] = [
    Time {
        menu_id: 9,
        text: "3 seconds",
        ms: 3000,
    },
    Time {
        menu_id: 10,
        text: "5 seconds",
        ms: 5000,
    },
    Time {
        menu_id: 11,
        text: "10 seconds",
        ms: 10000,
    },
];

/// The tray window's timer a timed capture waits on, one for every time, so choosing again
/// restarts it rather than adding a second capture.
/// Clear of the hotkey ids, which name the hotkey retry timers, and of the menu fade's timer.
pub const TIMER: usize = 101;

/// How long after the click the menu item `menu_id` takes its capture, if it is one of the times.
pub fn ms_for(menu_id: usize) -> Option<u32> {
    TIMES
        .iter()
        .find(|time| time.menu_id == menu_id)
        .map(|time| time.ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_submenu_reads_3_5_and_10_seconds() {
        let texts: Vec<_> = TIMES.iter().map(|time| time.text).collect();
        assert_eq!(texts, ["3 seconds", "5 seconds", "10 seconds"]);
        let ms: Vec<_> = TIMES.iter().map(|time| time.ms).collect();
        assert_eq!(ms, [3000, 5000, 10000]);
        let ids: Vec<_> = TIMES.iter().map(|time| time.menu_id).collect();
        assert_eq!(ids, [9, 10, 11]);
    }

    #[test]
    fn each_time_has_a_menu_id_of_its_own_among_the_tray_entries() {
        let mut ids: Vec<_> = TIMES.iter().map(|time| time.menu_id).collect();
        ids.extend([
            crate::ID_CAPTURE,
            crate::ID_OPEN_SAVE_FOLDER,
            crate::ID_QUIT,
            crate::ID_CAPTURE_SCREEN,
            crate::ID_CAPTURE_WINDOW,
            crate::ID_SETTINGS,
            crate::ID_COPY_TEXT,
            crate::ID_RECORD,
        ]);
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
        // TrackPopupMenu returns 0 when nothing was chosen.
        assert!(!ids.contains(&0));
    }

    #[test]
    fn each_item_takes_its_capture_after_its_own_time() {
        assert_eq!(ms_for(9), Some(3000));
        assert_eq!(ms_for(10), Some(5000));
        assert_eq!(ms_for(11), Some(10000));
        assert_eq!(ms_for(crate::ID_CAPTURE_SCREEN), None);
    }

    #[test]
    fn the_timer_is_clear_of_the_tray_windows_other_timers() {
        assert_ne!(TIMER, crate::MENU_FADE_TIMER);
        assert!(
            crate::hotkey::Mode::ALL
                .into_iter()
                .all(|mode| crate::hotkey_id(mode) != TIMER)
        );
    }
}
