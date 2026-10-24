//! Which window a window capture takes, and the size it is captured at, from the window hotkey
//! or from a click on the overlay.

use std::cell::Cell;

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
        System::Threading::GetCurrentProcessId,
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook},
            WindowsAndMessaging::{
                EVENT_SYSTEM_FOREGROUND, EnumWindows, GA_ROOT, GWL_EXSTYLE, GetAncestor,
                GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
                IsWindowVisible, SetForegroundWindow, WINDOW_EX_STYLE, WINEVENT_OUTOFCONTEXT,
                WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
            },
        },
    },
    core::{BOOL, Result},
};

use crate::selection::Point;
use crate::win32;

/// Window classes of the taskbar, on each screen, and of the tray icons it hides.
const TASKBAR: [&str; 4] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
];

/// Window classes of the desktop: the icons, and the wallpaper layer that takes the front when
/// the desktop is clicked.
const DESKTOP: [&str; 2] = ["Progman", "WorkerW"];

thread_local! {
    /// The last window other than the taskbar to come to the front.
    static BEFORE_TASKBAR: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// Starts remembering which window was in front before the taskbar was, so a capture from the
/// tray hands the keyboard back to it. Call once, on the UI thread.
pub fn remember_front() {
    // SAFETY: `foreground_changed` is a WINEVENTPROC, called on this thread.
    unsafe {
        came_to_front(GetForegroundWindow());
        // Never unhooked: Zilum remembers for as long as it runs.
        let _ = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
    }
}

/// The window that was in front before the taskbar. Clicking the tray icon brings the taskbar to
/// the front, so this is what the user was in when they clicked it.
pub fn before_taskbar() -> HWND {
    BEFORE_TASKBAR.get()
}

/// Called on the UI thread, from its message loop, some time after `hwnd` came to the front.
unsafe extern "system" fn foreground_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    came_to_front(hwnd);
}

fn came_to_front(hwnd: HWND) {
    // An editor or the overlay is remembered: with one in front, there is no window to capture.
    // The hidden tray window is not: the tray menu brings it to the front, and it stays there
    // when the menu is dismissed, though the user is still in the window before.
    let root = root(hwnd);
    if !is_taskbar(root) && class(root) != "Zilum.Tray" {
        BEFORE_TASKBAR.set(hwnd);
    }
}

/// `window`, if a window capture can take it: a visible, uncloaked, unminimised window that is
/// its own root, has a frame, and is not Zilum's, the desktop, the taskbar, click-through or a
/// tool window.
pub fn capturable(window: HWND) -> Option<HWND> {
    (!window.is_invalid()
        && root(window) == window
        && on_screen(window).is_some_and(|shown| shown.kind == Kind::Capturable))
    .then_some(window)
}

/// The top-level windows on the screens now, front to back. Hidden, minimised and cloaked
/// windows are left out: no screen shows them.
pub fn shown() -> Vec<Shown> {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe extern "system" fn add(window: HWND, list: LPARAM) -> BOOL {
        // SAFETY: `list` is `windows` below, used by nothing else during EnumWindows.
        unsafe { (*(list.0 as *mut Vec<HWND>)).push(window) };
        true.into()
    }
    // Front to back is the order EnumWindows gives.
    // SAFETY: `add` is a WNDENUMPROC, and `windows` outlives the enumeration it is passed to.
    let _ = unsafe { EnumWindows(Some(add), LPARAM(&mut windows as *mut Vec<HWND> as isize)) };
    windows.into_iter().filter_map(on_screen).collect()
}

/// `window` as the screen shows it, with an empty frame passed over, or none if the screen does
/// not show it, hidden, minimised or cloaked, or its frame cannot be read.
fn on_screen(window: HWND) -> Option<Shown> {
    let shown_as = kind(window)?;
    let frame = frame(window).ok()?;
    let kind = match shown_as {
        Kind::Capturable if frame.right <= frame.left || frame.bottom <= frame.top => {
            Kind::PassedOver
        }
        kind => kind,
    };
    Some(Shown {
        window,
        frame,
        kind,
    })
}

/// What `window` is to a capture, or none if the screen does not show it: hidden, minimised or
/// cloaked.
fn kind(window: HWND) -> Option<Kind> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
            return None;
        }
        let mut cloaked = 0u32;
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
        .ok()?;
        if cloaked != 0 {
            return None;
        }
    }
    Some(if is_zilum(window) {
        Kind::Zilum
    } else if is_taskbar(window) || DESKTOP.contains(&class(window).as_str()) {
        Kind::Shell
    } else {
        styled(WINDOW_EX_STYLE(
            win32::window_long(window, GWL_EXSTYLE) as u32
        ))
    })
}

/// What a window that is neither Zilum's nor the shell's is to a capture, from its extended
/// styles. Clicks pass through a click-through window, and a tool window is a helper such as a
/// palette or a launcher's, so the user is never pointing at either. Real windows are often
/// layered or topmost, so neither of those counts.
fn styled(styles: WINDOW_EX_STYLE) -> Kind {
    if styles.contains(WS_EX_TRANSPARENT) || styles.contains(WS_EX_TOOLWINDOW) {
        Kind::PassedOver
    } else {
        Kind::Capturable
    }
}

/// Gives the front back to `previous`, the window the user was in, or leaves it there if it already
/// has it. Returns it if it has the front now; none if it is gone, hidden or minimised.
pub fn give_front(previous: Option<HWND>) -> Option<HWND> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        previous
            .filter(|&previous| {
                IsWindow(Some(previous)).as_bool()
                    && IsWindowVisible(previous).as_bool()
                    && !IsIconic(previous).as_bool()
            })
            .filter(|&previous| {
                GetForegroundWindow() == previous || SetForegroundWindow(previous).as_bool()
            })
    }
}

/// `window`, unless it is null: none.
pub fn valid(window: HWND) -> Option<HWND> {
    (!window.is_invalid()).then_some(window)
}

/// The window's visible frame in physical pixels, without the invisible resize borders Windows
/// keeps around it. A window capture is this size.
pub fn frame(window: HWND) -> Result<RECT> {
    let mut frame = RECT::default();
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut _,
            size_of::<RECT>() as u32,
        )?;
    }
    Ok(frame)
}

/// The top-level window `hwnd` belongs to, or `hwnd` itself when it has none.
pub fn root(hwnd: HWND) -> HWND {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.is_invalid() { hwnd } else { root }
}

/// Whether `hwnd` is one of Zilum's own windows, such as an editor, the overlay or the tray menu.
pub fn is_zilum(hwnd: HWND) -> bool {
    let mut process = 0;
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    process == unsafe { GetCurrentProcessId() }
}

/// Whether `hwnd` is the taskbar on any screen, or the flyout of tray icons it hides.
pub fn is_taskbar(hwnd: HWND) -> bool {
    TASKBAR.contains(&class(hwnd).as_str())
}

fn class(hwnd: HWND) -> String {
    let mut class = [0u16; 64];
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let length = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    String::from_utf16_lossy(&class[..length])
}

/// What a window on the frozen picture is to the overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A window a window capture can take.
    Capturable,
    /// The taskbar or the desktop: in the picture, covering what is behind, never captured.
    Shell,
    /// One of Zilum's own, which captures leave out, so the picture shows what is behind it.
    Zilum,
    /// A click-through or tool window, or one with an empty frame: often drawing nothing, as a
    /// game launcher's hidden in-game window does, so never highlighted or captured, and passed over to
    /// the window behind.
    PassedOver,
}

/// A top-level window shown on the screen when the overlay opened.
#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub window: HWND,
    /// The visible frame, in physical pixels of the desktop.
    pub frame: RECT,
    pub kind: Kind,
}

/// The window a click at `point` captures, from `shown` in front-to-back order: the frontmost
/// one under it that the picture shows there, if a window capture can take that one. Zilum's
/// windows and those passed over are looked through.
pub fn pick(point: Point, shown: &[Shown]) -> Option<&Shown> {
    let inside = |frame: RECT| {
        (frame.left..frame.right).contains(&point.x) && (frame.top..frame.bottom).contains(&point.y)
    };
    shown
        .iter()
        .filter(|shown| !matches!(shown.kind, Kind::Zilum | Kind::PassedOver))
        .find(|shown| inside(shown.frame))
        .filter(|shown| shown.kind == Kind::Capturable)
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_OVERLAPPEDWINDOW,
        WS_EX_TOPMOST, WS_EX_WINDOWEDGE,
    };

    use super::*;

    fn shown(id: usize, kind: Kind, left: i32, top: i32, right: i32, bottom: i32) -> Shown {
        Shown {
            window: HWND(id as *mut _),
            frame: RECT {
                left,
                top,
                right,
                bottom,
            },
            kind,
        }
    }

    fn picked(x: i32, y: i32, shown: &[Shown]) -> Option<usize> {
        pick(Point { x, y }, shown).map(|shown| shown.window.0 as usize)
    }

    #[test]
    fn a_point_picks_the_frontmost_window_under_it() {
        // A notepad at 100,100 - 900,700, partly covered by an explorer at 600,400 - 1400,1000.
        let windows = [
            shown(1, Kind::Capturable, 600, 400, 1400, 1000),
            shown(2, Kind::Capturable, 100, 100, 900, 700),
        ];
        assert_eq!(picked(300, 300, &windows), Some(2));
        assert_eq!(picked(700, 500, &windows), Some(1));
        assert_eq!(picked(1200, 900, &windows), Some(1));
    }

    #[test]
    fn a_point_picks_what_the_picture_shows_there_whichever_window_is_in_front() {
        // Window 1 at 600,300 - 1000,600 and window 2 over its lower right, each in front in
        // turn, over the desktop.
        let window = |id| match id {
            1 => shown(1, Kind::Capturable, 600, 300, 1000, 600),
            _ => shown(2, Kind::Capturable, 850, 450, 1350, 800),
        };
        for (front, back) in [(1, 2), (2, 1)] {
            let windows = [
                shown(3, Kind::Shell, 0, 1392, 2560, 1440),
                window(front),
                window(back),
                shown(4, Kind::Shell, 0, 0, 2560, 1440),
            ];
            assert_eq!(picked(700, 350, &windows), Some(1));
            assert_eq!(picked(1250, 750, &windows), Some(2));
            assert_eq!(picked(950, 550, &windows), Some(front));
            assert_eq!(picked(1500, 300, &windows), None);
        }
    }

    #[test]
    fn a_point_in_no_window_picks_none() {
        let windows = [shown(1, Kind::Capturable, 100, 100, 900, 700)];
        assert_eq!(picked(50, 50, &windows), None);
        assert_eq!(picked(2000, 1000, &windows), None);
        assert_eq!(picked(0, 0, &[]), None);
    }

    #[test]
    fn a_frame_takes_its_left_and_top_edges_and_not_its_right_and_bottom() {
        let windows = [shown(1, Kind::Capturable, 100, 100, 900, 700)];
        assert_eq!(picked(100, 100, &windows), Some(1));
        assert_eq!(picked(899, 699, &windows), Some(1));
        assert_eq!(picked(900, 300, &windows), None);
        assert_eq!(picked(300, 700, &windows), None);
    }

    #[test]
    fn zilum_windows_are_passed_through_to_the_window_behind() {
        // An editor over part of a notepad: the picture shows the notepad there.
        let windows = [
            shown(1, Kind::Zilum, 500, 300, 1500, 900),
            shown(2, Kind::Capturable, 100, 100, 900, 700),
        ];
        assert_eq!(picked(600, 400, &windows), Some(2));
        assert_eq!(picked(1200, 800, &windows), None);
    }

    #[test]
    fn a_point_passes_over_a_window_that_draws_nothing_to_the_window_behind() {
        // MapleStory, with the Nexon overlay inside its frame and Explorer's island across the
        // top of the screen, in front of it.
        let windows = [
            shown(1, Kind::PassedOver, 0, 0, 2560, 360),
            shown(2, Kind::PassedOver, 880, 546, 1680, 846),
            shown(3, Kind::Capturable, 241, 234, 2163, 1346),
            shown(4, Kind::Shell, 0, 0, 2560, 1440),
        ];
        assert_eq!(picked(1000, 600, &windows), Some(3));
        assert_eq!(picked(300, 300, &windows), Some(3));
        assert_eq!(picked(500, 1000, &windows), Some(3));
        assert_eq!(picked(100, 100, &windows), None);
    }

    #[test]
    fn a_point_over_only_windows_that_draw_nothing_picks_none() {
        let windows = [
            shown(1, Kind::PassedOver, 0, 0, 2560, 360),
            shown(2, Kind::PassedOver, 100, 100, 900, 700),
        ];
        assert_eq!(picked(200, 200, &windows), None);
        assert_eq!(picked(500, 500, &windows), None);
    }

    #[test]
    fn click_through_and_tool_windows_are_passed_over() {
        // The Nexon overlay, Explorer's island across the top, and a plain tool window.
        for styles in [
            WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOPMOST,
            WS_EX_TOOLWINDOW,
        ] {
            assert_eq!(styled(styles), Kind::PassedOver);
        }
    }

    #[test]
    fn plain_layered_and_topmost_windows_are_capturable() {
        // A plain window, a Chromium window and a window pinned on top.
        for styles in [
            WS_EX_OVERLAPPEDWINDOW,
            WS_EX_LAYERED | WS_EX_NOREDIRECTIONBITMAP,
            WS_EX_TOPMOST | WS_EX_WINDOWEDGE,
        ] {
            assert_eq!(styled(styles), Kind::Capturable);
        }
    }

    #[test]
    fn the_taskbar_and_the_desktop_pick_nothing_and_cover_what_is_behind() {
        // A notepad reaching under the taskbar, over the desktop.
        let windows = [
            shown(1, Kind::Shell, 0, 1392, 2560, 1440),
            shown(2, Kind::Capturable, 100, 100, 900, 1420),
            shown(3, Kind::Shell, 0, 0, 2560, 1440),
        ];
        assert_eq!(picked(300, 1400, &windows), None);
        assert_eq!(picked(1500, 500, &windows), None);
        assert_eq!(picked(300, 1000, &windows), Some(2));
    }
}
