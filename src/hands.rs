//! The user's hands during a run: any movement of the mouse, any button, any key ends it, and the
//! key or click that did is swallowed, its release with it, so it reaches nothing else. Low-level
//! hooks see every input before any app or hotkey does; they are up only from the pointer's
//! parking until the run has ended and what ended it is up again.

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::GetAsyncKeyState,
    UI::WindowsAndMessaging::{
        CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
        WM_XBUTTONUP,
    },
};

use crate::win32;

/// What Zilum puts in the extra information of the input it makes itself, the run's wheel, so the
/// hooks tell it from the user's.
pub const MARK: usize = 0x5A49_4C55;

/// Posted to the tray window once the run is over and nothing pressed to end it is still down, so
/// the hooks come down. Clear of the other messages the tray window takes.
pub const WM_UNHOOK: u32 = WM_APP + 7;

/// The tray window's timer that takes the hooks down at the latest [`LET_GO_MS`] after the run
/// has ended. Clear of the hotkey ids, which name the hotkey retry timers, of the menu fade's
/// timer, the timed capture's and the run's.
pub const TIMER: usize = 103;
/// How long after the run has ended the hooks wait at most for the release of what ended it. A
/// release can go unseen, as Ctrl+Alt+Del's does on the secure desktop, and the hooks would then
/// swallow every key; a hand holds a key this long only by leaning on it.
const LET_GO_MS: u32 = 3000;

thread_local! {
    /// The hands of the run being watched, while the hooks are up.
    static HANDS: Cell<Option<Hands>> = const { Cell::new(None) };
    /// The mouse hook and the keyboard hook, while they are up.
    static HOOKS: Cell<[HHOOK; 2]> = const { Cell::new([HHOOK(std::ptr::null_mut()); 2]) };
    /// The tray window, told when the hooks can come down.
    static TRAY: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    /// Set when the hands end the run, which the run's thread watches.
    static STOPPED: RefCell<Arc<AtomicBool>> = RefCell::default();
}

/// Watches the user's hands from now on, for a run whose pointer Zilum has just parked at
/// `parked`, setting `stopped` if they end it, and telling `tray` when the hooks can come down.
pub fn watch(parked: POINT, stopped: Arc<AtomicBool>, tray: HWND) {
    unwatch();
    HANDS.set(Some(Hands::new(parked)));
    TRAY.set(tray);
    STOPPED.set(stopped);
    // SAFETY: `mouse` and `keyboard` are HOOKPROCs, and the hooks are this thread's own, which
    // runs a message loop.
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(Into::into);
        HOOKS.set([
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), instance, 0).unwrap_or_default(),
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), instance, 0).unwrap_or_default(),
        ]);
    }
}

/// The run has ended: the hooks go now, or once what the hands pressed to end it is up, and at
/// the latest [`LET_GO_MS`] from now.
pub fn end() {
    if let Some(mut hands) = HANDS.get() {
        hands.end();
        HANDS.set(Some(hands));
        if !hands.done() {
            win32::set_timer(TRAY.get(), TIMER, LET_GO_MS);
        }
    }
    tidy();
}

/// Takes the hooks down if there is nothing left to watch.
pub fn tidy() {
    if HANDS.get().is_some_and(|hands| hands.done()) {
        unwatch();
    }
}

/// Takes the hooks down, whatever is still held.
pub fn unwatch() {
    win32::kill_timer(TRAY.get(), TIMER);
    HANDS.set(None);
    for hook in HOOKS.replace([HHOOK::default(); 2]) {
        if !hook.is_invalid() {
            // SAFETY: Takes a hook this thread set.
            let _ = unsafe { UnhookWindowsHookEx(hook) };
        }
    }
}

/// The hands take `input`, Zilum's own if `ours`: whether it is swallowed. Every input on the
/// system waits for this, so it only updates the hands, sets a flag and posts.
fn take(input: Input, ours: bool) -> bool {
    let Some(mut hands) = HANDS.get() else {
        return false;
    };
    let seen = hands.see(input, ours);
    HANDS.set(Some(hands));
    if seen.stops {
        STOPPED.with_borrow(|stopped| stopped.store(true, Ordering::Relaxed));
    }
    if hands.done() {
        win32::post(TRAY.get(), WM_UNHOOK, WPARAM(0), LPARAM(0));
    }
    seen.swallowed
}

/// Every mouse input on the system waits for this while a run is watched.
unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: At HC_ACTION, the lparam of a WH_MOUSE_LL hook points to an MSLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        // A side button's is the high word of the mouse data: 1 for the back one.
        let side = || match info.mouseData >> 16 {
            1 => Press::Button(Button::Back),
            _ => Press::Button(Button::Forward),
        };
        let input = match wparam.0 as u32 {
            WM_MOUSEMOVE => Some(Input::Moved(info.pt)),
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => Some(Input::Wheel),
            WM_LBUTTONDOWN => Some(Input::Down(Press::Button(Button::Left))),
            WM_LBUTTONUP => Some(Input::Up(Press::Button(Button::Left))),
            WM_RBUTTONDOWN => Some(Input::Down(Press::Button(Button::Right))),
            WM_RBUTTONUP => Some(Input::Up(Press::Button(Button::Right))),
            WM_MBUTTONDOWN => Some(Input::Down(Press::Button(Button::Middle))),
            WM_MBUTTONUP => Some(Input::Up(Press::Button(Button::Middle))),
            WM_XBUTTONDOWN => Some(Input::Down(side())),
            WM_XBUTTONUP => Some(Input::Up(side())),
            _ => None,
        };
        if let Some(input) = input
            && take(input, info.dwExtraInfo == MARK)
        {
            return LRESULT(1);
        }
    }
    // SAFETY: Passes the hook's own arguments on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Every key on the system waits for this while a run is watched.
unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: At HC_ACTION, the lparam of a WH_KEYBOARD_LL hook points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let key = Press::Key(info.vkCode as u8);
        // A key swallowed here is never down to Windows, so one that is was pressed before the
        // run, and this is its repeat.
        // SAFETY: Takes only an integer.
        let before = unsafe { GetAsyncKeyState(info.vkCode as i32) } < 0;
        let input = match wparam.0 as u32 {
            WM_KEYDOWN | WM_SYSKEYDOWN if before => Some(Input::Repeat),
            WM_KEYDOWN | WM_SYSKEYDOWN => Some(Input::Down(key)),
            WM_KEYUP | WM_SYSKEYUP => Some(Input::Up(key)),
            _ => None,
        };
        if let Some(input) = input
            && take(input, info.dwExtraInfo == MARK)
        {
            return LRESULT(1);
        }
    }
    // SAFETY: Passes the hook's own arguments on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// One input the run's hooks see.
#[derive(Clone, Copy)]
enum Input {
    /// The pointer moved to a point on the desktop.
    Moved(POINT),
    /// A key or a mouse button went down, or repeated as it was held.
    Down(Press),
    /// A key held down since before the run repeated: whatever was in front saw its press.
    Repeat,
    /// A key or a mouse button came up.
    Up(Press),
    /// The wheel turned, either way.
    Wheel,
}

/// A key or a mouse button.
#[derive(Clone, Copy)]
enum Press {
    /// A key, by its virtual key.
    Key(u8),
    Button(Button),
}

/// The five mouse buttons Windows knows.
#[derive(Clone, Copy)]
enum Button {
    Left,
    Right,
    Middle,
    /// The side buttons.
    Back,
    Forward,
}

impl Press {
    /// Its place in a [`Held`]: the keys first, then the buttons.
    fn index(self) -> usize {
        match self {
            Press::Key(key) => key as usize,
            Press::Button(button) => 256 + button as usize,
        }
    }
}

/// The keys and buttons down, one bit each, so the hooks keep them without allocating.
#[derive(Clone, Copy, Default)]
struct Held([u64; 5]);

impl Held {
    /// Whether anything is down.
    fn any(&self) -> bool {
        self.0.iter().any(|bits| *bits != 0)
    }

    /// Whether `press` is down.
    fn holds(&self, press: Press) -> bool {
        let at = press.index();
        self.0[at / 64] & (1 << (at % 64)) != 0
    }

    /// Notes `press` as `down` or up.
    fn set(&mut self, press: Press, down: bool) {
        let at = press.index();
        if down {
            self.0[at / 64] |= 1 << (at % 64);
        } else {
            self.0[at / 64] &= !(1 << (at % 64));
        }
    }
}

/// What became of one input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    /// It ended the run.
    stops: bool,
    /// It goes no further: the app under the pointer, Zilum's hotkeys and Windows never see it.
    swallowed: bool,
}

/// What the user's hands have done since a run's pointer was parked.
#[derive(Clone, Copy)]
struct Hands {
    /// Where the pointer was parked.
    parked: POINT,
    /// Whether the run is over, ended by them or on its own.
    over: bool,
    /// The keys and buttons pressed to end it that are still down, whose releases are swallowed.
    held: Held,
}

impl Hands {
    /// The hands of a run whose pointer is parked at `parked`, touching nothing yet.
    fn new(parked: POINT) -> Hands {
        Hands {
            parked,
            over: false,
            held: Held::default(),
        }
    }

    /// Takes `input`, which Zilum made itself if `ours`.
    fn see(&mut self, input: Input, ours: bool) -> Seen {
        let running = !self.over;
        let (touched, swallowed) = match input {
            // Zilum's own steps scroll the content and are not a hand.
            _ if ours => (false, false),
            // The wheel has no release to wait for.
            Input::Wheel => (true, running),
            // Nothing but a hand moves the pointer off where Zilum parked it.
            Input::Moved(at) => (at != self.parked, false),
            // A press while what ended the run is still down is part of that keystroke, as
            // PrintScreen is of Ctrl+PrintScreen, and goes with it.
            Input::Down(press) if running || self.held.any() => {
                self.held.set(press, true);
                (true, true)
            }
            Input::Down(_) => (false, false),
            // Its release is the app's, which saw its press, so it is not held for.
            Input::Repeat => (true, running || self.held.any()),
            Input::Up(press) => {
                let held = self.held.holds(press);
                self.held.set(press, false);
                (true, held)
            }
        };
        let stops = touched && running;
        self.over |= stops;
        Seen { stops, swallowed }
    }

    /// The run has ended on its own.
    fn end(&mut self) {
        self.over = true;
    }

    /// Whether there is nothing left to watch: the run is over, and nothing pressed to end it is
    /// still down.
    fn done(&self) -> bool {
        self.over && !self.held.any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the run's pointer is parked.
    const PARKED: POINT = POINT { x: 1192, y: 675 };
    /// An input that goes on as if no run were up.
    const PASSES: Seen = Seen {
        stops: false,
        swallowed: false,
    };
    /// An input that ends the run and reaches nothing else.
    const STOPS_AND_SWALLOWED: Seen = Seen {
        stops: true,
        swallowed: true,
    };
    /// An input that reaches nothing, the run already over.
    const SWALLOWED: Seen = Seen {
        stops: false,
        swallowed: true,
    };
    /// Esc's virtual key.
    const ESC: u8 = 0x1B;
    /// A's.
    const A: u8 = 0x41;

    #[test]
    fn the_first_pixel_the_mouse_moves_ends_the_run_and_the_movement_goes_on() {
        let mut hands = Hands::new(PARKED);
        let moved = hands.see(Input::Moved(POINT { x: 1193, y: 675 }), false);
        assert_eq!(
            moved,
            Seen {
                stops: true,
                swallowed: false
            }
        );
        // The next movement is the user's.
        assert_eq!(
            hands.see(Input::Moved(POINT { x: 1200, y: 680 }), false),
            PASSES
        );
    }

    #[test]
    fn a_key_ends_the_run_and_it_and_its_release_are_swallowed_and_the_next_key_is_the_users() {
        let mut hands = Hands::new(PARKED);
        assert_eq!(
            hands.see(Input::Down(Press::Key(ESC)), false),
            STOPS_AND_SWALLOWED
        );
        assert_eq!(hands.see(Input::Up(Press::Key(ESC)), false), SWALLOWED);
        assert_eq!(hands.see(Input::Down(Press::Key(A)), false), PASSES);
        assert_eq!(hands.see(Input::Up(Press::Key(A)), false), PASSES);
        assert_eq!(hands.see(Input::Down(Press::Key(ESC)), false), PASSES);
        assert_eq!(hands.see(Input::Up(Press::Key(ESC)), false), PASSES);
    }

    #[test]
    fn a_combination_that_ends_the_run_is_swallowed_whole_its_repeats_included() {
        // Ctrl+PrintScreen, the Capture scrolling hotkey, its Ctrl going down first and repeating
        // while PrintScreen is pressed.
        const CTRL: u8 = 0x11;
        const PRINT_SCREEN: u8 = 0x2C;
        let mut hands = Hands::new(PARKED);
        assert_eq!(
            hands.see(Input::Down(Press::Key(CTRL)), false),
            STOPS_AND_SWALLOWED
        );
        assert_eq!(hands.see(Input::Down(Press::Key(CTRL)), false), SWALLOWED);
        assert_eq!(
            hands.see(Input::Down(Press::Key(PRINT_SCREEN)), false),
            SWALLOWED
        );
        assert_eq!(hands.see(Input::Down(Press::Key(CTRL)), false), SWALLOWED);
        assert_eq!(
            hands.see(Input::Up(Press::Key(PRINT_SCREEN)), false),
            SWALLOWED
        );
        assert_eq!(hands.see(Input::Up(Press::Key(CTRL)), false), SWALLOWED);
        // All of it is up: the next PrintScreen is the user's, and opens an overlay as it would.
        assert_eq!(
            hands.see(Input::Down(Press::Key(PRINT_SCREEN)), false),
            PASSES
        );
        assert_eq!(
            hands.see(Input::Up(Press::Key(PRINT_SCREEN)), false),
            PASSES
        );
    }

    #[test]
    fn zilums_own_wheel_is_not_the_users_and_the_users_wheel_ends_the_run() {
        let mut hands = Hands::new(PARKED);
        for _ in 0..3 {
            assert_eq!(hands.see(Input::Wheel, true), PASSES);
        }
        assert_eq!(hands.see(Input::Wheel, false), STOPS_AND_SWALLOWED);
        assert_eq!(hands.see(Input::Wheel, false), PASSES);
    }

    #[test]
    fn a_click_ends_the_run_and_lands_nowhere_and_the_next_click_is_the_users() {
        const LEFT: Press = Press::Button(Button::Left);
        let mut hands = Hands::new(PARKED);
        assert_eq!(hands.see(Input::Down(LEFT), false), STOPS_AND_SWALLOWED);
        assert_eq!(hands.see(Input::Up(LEFT), false), SWALLOWED);
        assert_eq!(hands.see(Input::Down(LEFT), false), PASSES);
        assert_eq!(hands.see(Input::Up(LEFT), false), PASSES);
    }

    #[test]
    fn a_release_of_a_key_pressed_before_the_run_ends_it_and_reaches_the_app_that_saw_the_press() {
        let mut hands = Hands::new(PARKED);
        assert_eq!(
            hands.see(Input::Up(Press::Key(A)), false),
            Seen {
                stops: true,
                swallowed: false
            }
        );
        assert_eq!(hands.see(Input::Down(Press::Key(A)), false), PASSES);
    }

    #[test]
    fn the_hands_are_watched_until_the_run_ends_and_what_ended_it_is_up() {
        // A run that ends on its own.
        let mut hands = Hands::new(PARKED);
        assert!(!hands.done());
        hands.end();
        assert!(hands.done());
        // Ended by a movement.
        let mut hands = Hands::new(PARKED);
        hands.see(Input::Moved(POINT { x: 0, y: 0 }), false);
        assert!(hands.done());
        // Ended by a key, still watched until the key is up, even once the run has ended.
        let mut hands = Hands::new(PARKED);
        hands.see(Input::Down(Press::Key(A)), false);
        hands.end();
        assert!(!hands.done());
        hands.see(Input::Up(Press::Key(A)), false);
        assert!(hands.done());
    }

    #[test]
    fn a_key_held_from_before_the_run_ends_it_on_its_repeat_and_its_release_reaches_the_app() {
        // The app saw the press before the run; it sees the release, never half a keystroke.
        const SHIFT: u8 = 0x10;
        let mut hands = Hands::new(PARKED);
        assert_eq!(hands.see(Input::Repeat, false), STOPS_AND_SWALLOWED);
        assert_eq!(hands.see(Input::Repeat, false), PASSES);
        assert!(hands.done());
        assert_eq!(hands.see(Input::Up(Press::Key(SHIFT)), false), PASSES);
    }

    #[test]
    fn parking_the_pointer_is_not_the_users_movement() {
        let mut hands = Hands::new(PARKED);
        assert_eq!(hands.see(Input::Moved(PARKED), false), PASSES);
        assert_eq!(hands.see(Input::Moved(PARKED), false), PASSES);
    }
}
