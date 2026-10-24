//! One Zilum at a time. A copy opened while another runs asks the running one its version: a
//! newer copy closes the running one and takes its place, so updating is opening the new exe;
//! any other copy leaves, and the running one says why nothing new appeared.

use std::time::Duration;

use windows::Win32::{
    Foundation::{HANDLE, LPARAM, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM},
    System::Threading::WaitForSingleObject,
    UI::WindowsAndMessaging::{
        FindWindowW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_APP, WM_CLOSE,
    },
};
use windows::core::w;

use crate::win32;

/// Tells the running copy that a copy left, its version in wparam. Copies made before copies
/// told their version send it too, with 0, so its number never changes.
pub const WM_COPY_LEFT: u32 = WM_APP + 1;
/// Asks the running copy its version, which it answers. A copy made before copies told their
/// version leaves it to Windows, which answers 0.
pub const WM_VERSION: u32 = WM_APP + 4;
/// How long the running copy has to answer, and to close once asked.
const ANSWER_MS: u32 = 2000;
const CLOSE_MS: u32 = 5000;

/// This copy's version, packed so a later version is a larger number.
pub const VERSION: u32 = pack(
    parse(env!("CARGO_PKG_VERSION_MAJOR")),
    parse(env!("CARGO_PKG_VERSION_MINOR")),
    parse(env!("CARGO_PKG_VERSION_PATCH")),
);

// 0 is the answer of a copy made before copies told their version.
const _: () = assert!(VERSION > 0);

/// `major.minor.patch` as one number, each part below 1024.
const fn pack(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 20) | (minor << 10) | patch
}

const fn parse(digits: &str) -> u32 {
    let bytes = digits.as_bytes();
    let mut value = 0;
    let mut at = 0;
    while at < bytes.len() {
        value = value * 10 + (bytes[at] - b'0') as u32;
        at += 1;
    }
    value
}

/// What a copy opened while another runs does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Meeting {
    /// Closes the running copy, which is older, and starts in its place.
    TakeOver,
    /// Leaves the running copy as it is and exits.
    Leave,
}

/// What a copy of version `own` does when the running copy answered `running`: its version, 0
/// from a copy made before copies told their version, or `None` when it did not answer in time.
pub fn meet(own: u32, running: Option<u32>) -> Meeting {
    match running {
        Some(running) if running < own => Meeting::TakeOver,
        _ => Meeting::Leave,
    }
}

/// What the running copy, of version `own`, says when a copy of version `opened` left: 0 from a
/// copy made before copies told their version, which is older than any that tells it.
pub fn left_message(own: u32, opened: u32) -> &'static str {
    if opened < own {
        "A newer Zilum is already running"
    } else {
        "Zilum is already running"
    }
}

/// Meets the running copy, when another copy holds `mutex`, the single-instance mutex this copy
/// opened. True when this copy took over: the running copy, older, closed, and this copy holds
/// `mutex` now. False when it leaves, the running copy told why.
pub fn arrive(mutex: HANDLE) -> bool {
    let Some(tray) = running_tray() else {
        return false;
    };
    let mut answer = 0;
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let answered = unsafe {
        SendMessageTimeoutW(
            tray,
            WM_VERSION,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            ANSWER_MS,
            Some(&mut answer),
        )
    }
    .0 != 0;
    match meet(VERSION, answered.then_some(answer as u32)) {
        Meeting::Leave => {
            win32::post(tray, WM_COPY_LEFT, WPARAM(VERSION as usize), LPARAM(0));
            false
        }
        Meeting::TakeOver => {
            // Every copy closes on WM_CLOSE to its tray window, as Quit closes it, those made
            // before copies told their version included: Windows' own handling destroys it.
            win32::post(tray, WM_CLOSE, WPARAM(0), LPARAM(0));
            // The mutex is the running copy's until its process ends, abandoning it.
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let waited = unsafe { WaitForSingleObject(mutex, CLOSE_MS) };
            waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED
        }
    }
}

/// The running copy's tray window. It may still be making it, so look for about a second.
fn running_tray() -> Option<windows::Win32::Foundation::HWND> {
    for _ in 0..20 {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        if let Ok(hwnd) = unsafe { FindWindowW(w!("Zilum.Tray"), None) } {
            return Some(hwnd);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_version_packs_to_a_larger_number() {
        assert!(pack(0, 1, 2) > pack(0, 1, 1));
        assert!(pack(0, 2, 0) > pack(0, 1, 999));
        assert!(pack(1, 0, 0) > pack(0, 999, 999));
    }

    #[test]
    fn a_newer_copy_takes_over_from_an_older_one() {
        assert_eq!(meet(pack(0, 1, 2), Some(pack(0, 1, 1))), Meeting::TakeOver);
        assert_eq!(meet(pack(1, 0, 0), Some(pack(0, 9, 9))), Meeting::TakeOver);
    }

    #[test]
    fn a_newer_copy_takes_over_from_one_made_before_copies_told_their_version() {
        assert_eq!(meet(pack(0, 1, 2), Some(0)), Meeting::TakeOver);
    }

    #[test]
    fn the_same_or_an_older_copy_leaves() {
        assert_eq!(meet(pack(0, 1, 2), Some(pack(0, 1, 2))), Meeting::Leave);
        assert_eq!(meet(pack(0, 1, 1), Some(pack(0, 1, 2))), Meeting::Leave);
    }

    #[test]
    fn a_copy_leaves_when_the_running_one_does_not_answer() {
        assert_eq!(meet(pack(0, 1, 2), None), Meeting::Leave);
    }

    #[test]
    fn the_running_copy_says_which_kind_of_copy_left() {
        let own = pack(0, 1, 2);
        assert_eq!(left_message(own, own), "Zilum is already running");
        assert_eq!(
            left_message(own, pack(0, 1, 1)),
            "A newer Zilum is already running"
        );
        assert_eq!(left_message(own, 0), "A newer Zilum is already running");
    }
}
