//! The Win32 calls every window in Zilum makes, in safe form. Each takes only handles and
//! integers, which Windows validates and fails on rather than faults, or a pointer to a local
//! this module owns for the length of the call, so none can be misused from safe code.

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{ClientToScreen, ScreenToClient},
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, VIRTUAL_KEY},
        WindowsAndMessaging::{
            DefWindowProcW, DestroyWindow, GetWindowLongPtrW, KillTimer, PostMessageW, SetTimer,
            WINDOW_LONG_PTR_INDEX,
        },
    },
};

/// What Windows does with a message the window procedure leaves to it.
pub fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Takes only handles and integers, which Windows validates.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Destroys `hwnd`, if it is still a window.
pub fn destroy(hwnd: HWND) {
    // SAFETY: Takes only a handle, which Windows validates.
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// Starts, or restarts, timer `id` on `hwnd`, firing WM_TIMER every `ms` milliseconds. False if
/// Windows refused it.
pub fn set_timer(hwnd: HWND, id: usize, ms: u32) -> bool {
    // SAFETY: Takes only handles and integers, which Windows validates, and no callback.
    unsafe { SetTimer(Some(hwnd), id, ms, None) != 0 }
}

/// Stops timer `id` on `hwnd`, if it is running.
pub fn kill_timer(hwnd: HWND, id: usize) {
    // SAFETY: Takes only handles and integers, which Windows validates.
    let _ = unsafe { KillTimer(Some(hwnd), id) };
}

/// Whether `key` is down in this thread's view of the keyboard, as of the message being handled.
pub fn key_down(key: VIRTUAL_KEY) -> bool {
    // SAFETY: Takes only an integer.
    unsafe { GetKeyState(key.0 as i32) < 0 }
}

/// Queues `msg` on `hwnd`'s thread and returns at once.
pub fn post(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) {
    // SAFETY: Takes only handles and integers, which Windows validates; a pointer carried in
    // `lparam` is the receiver's to account for.
    let _ = unsafe { PostMessageW(Some(hwnd), msg, wparam, lparam) };
}

/// Sends the mouse to `hwnd` until the capture is released, even outside the window.
pub fn capture(hwnd: HWND) {
    // SAFETY: Takes only a handle, which Windows validates.
    unsafe { SetCapture(hwnd) };
}

/// Lets the mouse go wherever the pointer is again.
pub fn release_capture() {
    // SAFETY: Takes nothing.
    let _ = unsafe { ReleaseCapture() };
}

/// The value at `index` in `hwnd`'s window data, such as its style, or 0 if there is no such
/// window.
pub fn window_long(hwnd: HWND, index: WINDOW_LONG_PTR_INDEX) -> isize {
    // SAFETY: Takes only handles and integers, which Windows validates.
    unsafe { GetWindowLongPtrW(hwnd, index) }
}

/// `point`, in `hwnd`'s client pixels, as screen pixels. Unchanged if `hwnd` is not a window.
pub fn to_screen(hwnd: HWND, mut point: POINT) -> POINT {
    // SAFETY: `point` is a local that outlives the call.
    let _ = unsafe { ClientToScreen(hwnd, &mut point) };
    point
}

/// `point`, in screen pixels, as `hwnd`'s client pixels. Unchanged if `hwnd` is not a window.
pub fn to_client(hwnd: HWND, mut point: POINT) -> POINT {
    // SAFETY: `point` is a local that outlives the call.
    let _ = unsafe { ScreenToClient(hwnd, &mut point) };
    point
}
