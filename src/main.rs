#![windows_subsystem = "windows"]

mod annotation;
mod background;
mod bar;
mod capture;
mod caret;
mod clipboard;
mod command_bar;
mod copy_text;
mod editor;
mod entry;
mod flyout;
mod frame;
mod graphics;
mod history;
mod hotkey;
mod image;
mod instance;
mod lifetime;
mod notice;
mod overlay;
mod picker;
mod pixels;
mod save;
mod selection;
mod settings;
mod settings_layout;
mod settings_window;
mod slide;
mod startup;
mod style;
mod theme;
mod tooltip;
mod view;
mod wallpaper;
mod win32;
mod window;

use std::{cell::Cell, sync::Arc, time::Duration};

use windows::{
    Graphics::Capture::GraphicsCaptureItem,
    Win32::{
        Foundation::{
            ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, SYSTEMTIME, WPARAM,
        },
        Graphics::{
            Direct3D11::ID3D11Texture2D,
            Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromWindow},
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
            SystemInformation::GetLocalTime,
            Threading::{CreateMutexW, GetCurrentThreadId},
        },
        UI::{
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem,
                GetSystemMetricsForDpi, SetProcessDpiAwarenessContext,
            },
            Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey},
            Shell::{
                NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
                NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DestroyMenu,
                DispatchMessageW, GetForegroundWindow, GetMessageW, HICON, IMAGE_ICON,
                LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, MSG, MSGF_MENU, PM_REMOVE,
                PeekMessageW, PostQuitMessage, RegisterClassW, SM_CXSMICON, SPI_GETSELECTIONFADE,
                SPI_GETUIEFFECTS, SYSTEM_METRICS_INDEX, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                SetForegroundWindow, SetWindowsHookExW, SystemParametersInfoW, TPM_RETURNCMD,
                TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx,
                WH_MSGFILTER, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY,
                WM_ENDSESSION, WM_HOTKEY, WM_NULL, WM_QUERYENDSESSION, WM_QUIT, WM_TIMER,
                WNDCLASSW, WS_OVERLAPPED,
            },
        },
    },
    core::{BOOL, PCWSTR, Result, w},
};

use crate::{
    hotkey::{Hotkey, Mode, Refusal, in_use},
    selection::Selection,
};

const TRAY_ID: u32 = 1;
const WM_TRAY: u32 = WM_APP;
const ID_CAPTURE: usize = 1;
const ID_OPEN_SAVE_FOLDER: usize = 2;
const ID_QUIT: usize = 3;
const ID_CAPTURE_SCREEN: usize = 4;
const ID_CAPTURE_WINDOW: usize = 5;
const ID_SETTINGS: usize = 6;
const ID_COPY_TEXT: usize = 7;
const HOTKEY_RETRY_MS: u32 = 500;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
/// The timer that runs a capture or Copy text chosen from the tray menu once the menu has faded.
/// Clear of the hotkey ids, which name the hotkey retry timers.
const MENU_FADE_TIMER: usize = 100;
/// How long Windows takes to fade a chosen menu item out. The compositor draws the fade after the
/// menu window is destroyed and reports nothing when it ends, so this is measured: on the
/// development machine a capture 250 ms after the menu closed sometimes held a trace of the item
/// and one 260 ms after never did; 20 ms more is the margin.
const MENU_FADE_MS: u32 = 280;

thread_local! {
    /// A capture or Copy text chosen from the tray menu, waiting out the menu's fade, and the
    /// window the user was in.
    static WAITING: Cell<Option<(Mode, Option<HWND>)>> = const { Cell::new(None) };
}

/// What a selection on the overlay is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A capture, on the clipboard and in an editor.
    Capture,
    /// Copy text: the text read in it, on the clipboard.
    CopyText,
}

fn main() -> Result<()> {
    // SAFETY: Win32 and Direct3D calls on live handles, interfaces and locals.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;

        // A newer copy opened while an older one runs closes it and starts in its place, so
        // updating is opening the new exe. Any other copy leaves the running one as it is.
        let single_instance = CreateMutexW(None, true, w!(r"Local\Zilum"))?;
        let took_over = GetLastError() == ERROR_ALREADY_EXISTS;
        if took_over && !instance::arrive(single_instance) {
            return Ok(());
        }

        // A hotkey press then finds the graphics device ready. If this fails, the first
        // capture tries again and reports its own failure.
        graphics::init_apartment();
        let _ = graphics::get();
        capture::request_borderless_access();

        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Zilum.Tray"),
            ..Default::default()
        };
        RegisterClassW(&class);
        overlay::register(instance.into())?;
        editor::register(instance.into())?;
        notice::register(instance.into())?;
        entry::register(instance.into())?;
        settings_window::register(instance.into())?;
        tooltip::register(instance.into())?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Zilum.Tray"),
            w!("Zilum"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        add_tray_icon(hwnd)?;
        window::remember_front();

        // Read once: a hand edit takes effect at the next start. An invalid value stays in the
        // file until the next choice rewrites it.
        let (settings, invalid) = settings::load();
        editor::set_colour(settings.colour);
        editor::set_sizes(settings.sizes);
        editor::set_text_size(settings.text_size);
        editor::set_head(settings.head);
        editor::set_fill(settings.fill);
        editor::set_level(settings.blur);
        // A start the user made says Zilum is there, as it has no window. Any problem below
        // follows and replaces it. A start at sign-in says nothing.
        if !startup::at_sign_in() {
            notify(
                hwnd,
                &running(settings.hotkeys[Mode::Capture as usize], took_over),
            );
        }
        if invalid {
            notify(hwnd, "Some settings were invalid and reset");
        }
        // The entry holds the exe's path, so a moved or renamed exe still starts at sign-in.
        if settings.start_with_windows {
            startup::set(true);
        }

        // Each hotkey is claimed, retried and reported on its own, so one another app owns
        // leaves the others working. Another app may hold one only because it started a moment
        // earlier.
        for mode in Mode::ALL {
            let Some(hotkey) = settings.hotkeys[mode as usize] else {
                continue;
            };
            if !claim(hwnd, mode, hotkey)
                && !win32::set_timer(hwnd, hotkey_id(mode), HOTKEY_RETRY_MS)
            {
                notify(hwnd, &in_use(hotkey));
            }
        }

        let mut msg = MSG::default();
        loop {
            // While the overlay's highlight slides, its frames are drawn whenever no message is
            // waiting, so a move or a press mid-slide is never kept waiting. Otherwise nothing
            // runs until a message comes.
            if overlay::sliding() {
                if !PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    overlay::slide_frame();
                    continue;
                }
                if msg.message == WM_QUIT {
                    break;
                }
            } else if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                break;
            }
            if settings_window::handles(&msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAY => {
            // Version 4 callbacks: the event is the low word of lparam.
            match lparam.0 as u32 & 0xFFFF {
                // A left-click, or Enter or Space on the focused icon.
                NIN_SELECT | NIN_KEYSELECT => {
                    cancel_waiting(hwnd);
                    open_overlay(
                        hwnd,
                        window::valid(window::before_taskbar()),
                        Purpose::Capture,
                    )
                }
                WM_CONTEXTMENU => {
                    // The click position in screen coordinates is packed into wparam.
                    let x = (wparam.0 & 0xFFFF) as i16 as i32;
                    let y = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                    // Taken now, at the click, because showing the menu brings the tray window
                    // to the front.
                    let previous = window::before_taskbar();
                    show_menu(hwnd, x, y, previous);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            // The window the user was in, which gets the keyboard back when the capture is
            // cancelled, or when its editor leaves untouched.
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let previous = window::valid(unsafe { GetForegroundWindow() });
            if let Some(mode) = mode_of(wparam.0) {
                cancel_waiting(hwnd);
                start(hwnd, mode, previous);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == MENU_FADE_TIMER => {
            win32::kill_timer(hwnd, MENU_FADE_TIMER);
            if let Some((chosen, previous)) = WAITING.take() {
                start(hwnd, chosen, previous);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(mode) = mode_of(wparam.0) {
                win32::kill_timer(hwnd, wparam.0);
                if let Some(hotkey) = settings::current().hotkeys[mode as usize]
                    && !claim(hwnd, mode, hotkey)
                {
                    notify(hwnd, &in_use(hotkey));
                }
            }
            LRESULT(0)
        }
        copy_text::WM_TEXT_COPIED | copy_text::WM_NO_TEXT => {
            copy_text::finished(msg, wparam);
            LRESULT(0)
        }
        instance::WM_COPY_LEFT => {
            notify(
                hwnd,
                instance::left_message(instance::VERSION, wparam.0 as u32),
            );
            LRESULT(0)
        }
        instance::WM_VERSION => LRESULT(instance::VERSION as isize),
        // A newer copy closes this one so, to start in its place.
        WM_CLOSE => {
            win32::destroy(hwnd);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                win32::destroy(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let nid = tray_data(hwnd);
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => win32::default_proc(hwnd, msg, wparam, lparam),
    }
}

/// The id of `mode`'s hotkey, and of the timer that retries claiming it.
fn hotkey_id(mode: Mode) -> usize {
    mode as usize + 1
}

/// The mode whose hotkey or retry timer has the id `id`.
fn mode_of(id: usize) -> Option<Mode> {
    Mode::ALL.into_iter().find(|mode| hotkey_id(*mode) == id)
}

/// True when `hotkey` will reach Zilum as `mode`'s. PrintScreen stays registered even when
/// Snipping Tool takes it, so it starts working if the user later turns Snipping Tool's setting
/// off.
fn claim(hwnd: HWND, mode: Mode, hotkey: Hotkey) -> bool {
    let id = hotkey_id(mode) as i32;
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        // Re-registering our own hotkey would fail, so the retry starts clean.
        let _ = UnregisterHotKey(Some(hwnd), id);
        let registered =
            RegisterHotKey(Some(hwnd), id, hotkey.modifiers(), hotkey.virtual_key()).is_ok();
        registered && (hotkey != Hotkey::PRINTSCREEN || !snipping_tool_owns_printscreen())
    }
}

/// Moves each mode's hotkey from `from` to `to` for the tray window `tray`, retrying a failed
/// claim once after the startup retry's half second, waited out here rather than on a timer so
/// Settings knows the outcome. A mode whose new hotkey cannot be claimed keeps its old one,
/// working throughout, and so does a mode whose new hotkey is the old one of a mode that keeps it;
/// their entries say why. Every other mode has its new hotkey from now on.
pub fn rebind(
    tray: HWND,
    from: [Option<Hotkey>; 4],
    to: [Option<Hotkey>; 4],
) -> [Option<Refusal>; 4] {
    let changing: Vec<Mode> = Mode::ALL
        .into_iter()
        .filter(|mode| from[*mode as usize] != to[*mode as usize])
        .collect();
    let mut refused = [None; 4];
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        // All released first, so two modes can swap hotkeys.
        for mode in &changing {
            let _ = UnregisterHotKey(Some(tray), hotkey_id(*mode) as i32);
        }
        // Claiming releases whatever the mode held, the old hotkey included.
        let claims_new =
            |mode: &Mode| to[*mode as usize].is_none_or(|hotkey| claim(tray, *mode, hotkey));
        let mut failed: Vec<Mode> = changing
            .iter()
            .copied()
            .filter(|m| !claims_new(m))
            .collect();
        if !failed.is_empty() {
            // The old hotkeys keep working while the retry waits.
            for mode in &failed {
                if let Some(old) = from[*mode as usize] {
                    claim(tray, *mode, old);
                }
            }
            std::thread::sleep(Duration::from_millis(HOTKEY_RETRY_MS.into()));
            failed.retain(|mode| !claims_new(mode));
        }
        for mode in failed {
            refused[mode as usize] = Some(Refusal::InUse);
        }
        // A mode keeping its old hotkey takes it back from a mode that has just claimed it.
        while let Some((keeper, taker)) = Mode::ALL.into_iter().find_map(|keeper| {
            refused[keeper as usize]?;
            let old = from[keeper as usize]?;
            let taker = changing
                .iter()
                .copied()
                .find(|t| refused[*t as usize].is_none() && to[*t as usize] == Some(old))?;
            Some((keeper, taker))
        }) {
            let _ = UnregisterHotKey(Some(tray), hotkey_id(taker) as i32);
            refused[taker as usize] = Some(Refusal::UsedFor(keeper));
        }
        for mode in Mode::ALL
            .into_iter()
            .filter(|m| refused[*m as usize].is_some())
        {
            // Snipping Tool's PrintScreen is registered though not claimed.
            let _ = UnregisterHotKey(Some(tray), hotkey_id(mode) as i32);
            if let Some(old) = from[mode as usize] {
                claim(tray, mode, old);
            }
        }
    }
    refused
}

/// Snipping Tool's PrintScreen setting takes the key before it reaches registered hotkeys,
/// yet RegisterHotKey still succeeds, so the setting has to be read. Zilum never writes it.
fn snipping_tool_owns_printscreen() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a u32, the size `size` gives.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Control Panel\Keyboard"),
            w!("PrintScreenKeyForSnippingEnabled"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok() && value != 0
}

/// Captures or copies text as `mode` asks. `previous` is the window the user was in, which gets
/// the keyboard back when no editor takes it, or when the editor leaves untouched.
fn start(tray: HWND, mode: Mode, previous: Option<HWND>) {
    match mode {
        Mode::Capture => open_overlay(tray, previous, Purpose::Capture),
        Mode::Screen => capture_screen(tray, previous),
        Mode::Window => capture_window(
            tray,
            editor::looking_past(previous).and_then(window::capturable),
            previous,
        ),
        Mode::CopyText => open_overlay(tray, previous, Purpose::CopyText),
    }
}

/// Runs `chosen` from the tray menu once Windows has faded the chosen item out, so the capture
/// does not hold it. The pointer is then still about where the item was clicked, so the capture
/// is of that screen.
fn after_fade(tray: HWND, chosen: Mode, previous: Option<HWND>) {
    if menu_fades() && win32::set_timer(tray, MENU_FADE_TIMER, MENU_FADE_MS) {
        WAITING.set(Some((chosen, previous)));
    } else {
        start(tray, chosen, previous);
    }
}

/// Drops a capture or Copy text chosen from the tray menu that is still waiting out the menu's
/// fade, because the user has asked for something else since.
fn cancel_waiting(tray: HWND) {
    if WAITING.take().is_some() {
        win32::kill_timer(tray, MENU_FADE_TIMER);
    }
}

/// Opens the overlay for `purpose`. `previous` is the window the user was in, if any, which gets
/// the keyboard back when the capture is cancelled, or when its editor leaves untouched.
fn open_overlay(hwnd: HWND, previous: Option<HWND>, purpose: Purpose) {
    if overlay::open(hwnd, previous, purpose).is_err() {
        notify(hwnd, "Capture failed");
    }
}

/// Captures the whole current screen with no overlay. `previous` is as for `open_overlay`. Does
/// nothing while an overlay is open.
fn capture_screen(hwnd: HWND, previous: Option<HWND>) {
    if overlay::is_open() {
        return;
    }
    let grabbed = capture::current_screen().and_then(|(monitor, screen)| {
        // Zilum's editors are excluded from capture, so what is behind them is in the frame.
        let frame = capture::grab(monitor)?;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        Ok((monitor, screen, frame, unsafe { GetLocalTime() }))
    });
    match grabbed {
        Ok((monitor, screen, frame, taken)) => {
            let whole = Selection {
                left: 0,
                top: 0,
                width: screen.right - screen.left,
                height: screen.bottom - screen.top,
            };
            deliver(
                hwnd,
                &frame,
                monitor,
                taken,
                whole,
                None,
                monitor,
                previous,
                Purpose::Capture,
            );
        }
        Err(_) => notify(hwnd, "Capture failed"),
    }
}

/// Captures the whole of `window`, the one in front when the capture was asked for, or the one
/// before the editor in front, if it was capturable then, with no overlay. A window gone since
/// then is a failed capture. `previous` is as for `open_overlay`. Does nothing while an overlay is
/// open.
fn capture_window(tray: HWND, window: Option<HWND>, previous: Option<HWND>) {
    if overlay::is_open() {
        return;
    }
    let Some(window) = window else {
        notify(tray, "No window to capture");
        return;
    };
    // The item first: from then on it is this window, whatever happens to the handle.
    let item = capture::window_item(window);
    match capture::current_screen() {
        Ok((screen, _)) => deliver_window(tray, item, window, screen, previous, Purpose::Capture),
        Err(_) => notify(tray, "Capture failed"),
    }
}

/// Delivers the whole of the window `item` was made for, `window`, for `purpose`, as `deliver`
/// does. No item, or a window gone by now, is a failed capture.
pub fn deliver_window(
    tray: HWND,
    item: Result<GraphicsCaptureItem>,
    window: HWND,
    screen: HMONITOR,
    previous: Option<HWND>,
    purpose: Purpose,
) {
    let grabbed = item.and_then(|item| {
        let bounds = window::frame(window)?;
        // The frame's colours are relative to the white of the screen the window is mostly on.
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let shown_on = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
        let frame = capture::grab_item(&item)?;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        Ok((bounds, shown_on, frame, unsafe { GetLocalTime() }))
    });
    match grabbed {
        Ok((bounds, shown_on, frame, taken)) => {
            // The frame starts at the visible frame's top-left; past its size is anything.
            let whole = Selection {
                left: 0,
                top: 0,
                width: bounds.right - bounds.left,
                height: bounds.bottom - bounds.top,
            };
            deliver(
                tray,
                &frame,
                shown_on,
                taken,
                whole,
                Some(bounds),
                screen,
                previous,
                purpose,
            );
        }
        Err(_) => notify(tray, "Capture failed"),
    }
}

/// Takes `selection` of `frame`, captured at `taken` with its colours relative to the white of
/// `shown_on`, for `purpose`: on the clipboard and in an editor on the primary screen, or its
/// text read to the clipboard with a Notice on `screen`. `window` is where the window's frame was
/// on the screen when the capture is a window's: its transparent corners and regions are kept,
/// and a capture gets the window background around it. `previous` is the window the user was in,
/// which the editor's keyboard goes back to when it leaves untouched, or which gets it back at
/// once when no editor opens. Failures are notified from the tray window `tray`.
#[allow(clippy::too_many_arguments)]
pub fn deliver(
    tray: HWND,
    frame: &ID3D11Texture2D,
    shown_on: HMONITOR,
    taken: SYSTEMTIME,
    selection: Selection,
    window: Option<RECT>,
    screen: HMONITOR,
    previous: Option<HWND>,
    purpose: Purpose,
) {
    // A window that draws nothing comes back with every pixel transparent: an editor of it
    // would show nothing, and the clipboard would hold a blank image.
    let read = pixels::read(frame, shown_on, taken, selection, window.is_some())
        .ok()
        .filter(|image| window.is_none() || !image.is_blank());
    match read {
        Some(image) => match purpose {
            Purpose::Capture => {
                // A window capture gets its background from the moment it is taken.
                let image = match window {
                    Some(bounds) => settings::current()
                        .window_background
                        .around(image, bounds, shown_on),
                    None => image,
                };
                let image = Arc::new(image);
                // Without an editor the capture is still copied.
                let editor = editor::open(image.clone(), tray, previous).ok();
                clipboard::copy(image, tray, editor);
            }
            Purpose::CopyText => {
                // No editor opens to hand the front back, so it goes back from here.
                window::give_front(previous);
                copy_text::start(image, screen, tray);
            }
        },
        None => notify(tray, "Capture failed"),
    }
}

/// Drops hotkey messages while the tray menu is modal, so no overlay opens under it.
unsafe extern "system" fn menu_filter(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: For a WH_MSGFILTER hook, lparam points to the MSG being filtered.
    if code == MSGF_MENU as i32 && unsafe { (*(lparam.0 as *const MSG)).message } == WM_HOTKEY {
        return LRESULT(1);
    }
    // SAFETY: Passes the hook's own arguments on unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Shows the tray menu at `x`, `y`. `previous` is the window that was in front before the tray icon
/// was clicked, which Capture window captures if it is capturable, and which every capture hands
/// the keyboard back to.
fn show_menu(hwnd: HWND, x: i32, y: i32, previous: HWND) {
    let previous = window::valid(previous);
    // Its timer would fire under this menu, and the capture would hold the menu.
    cancel_waiting(hwnd);
    // SAFETY: `menu_filter` is a HOOKPROC on this thread; the rest take handles and integers.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, ID_CAPTURE, w!("Capture"));
        let _ = AppendMenuW(menu, MF_STRING, ID_CAPTURE_WINDOW, w!("Capture window"));
        let _ = AppendMenuW(menu, MF_STRING, ID_CAPTURE_SCREEN, w!("Capture screen"));
        let _ = AppendMenuW(menu, MF_STRING, ID_COPY_TEXT, w!("Copy text"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_SAVE_FOLDER, w!("Open save folder"));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));
        // Without this the menu does not close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let hook = SetWindowsHookExW(WH_MSGFILTER, Some(menu_filter), None, GetCurrentThreadId());
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            None,
            hwnd,
            None,
        );
        if let Ok(hook) = hook {
            let _ = UnhookWindowsHookEx(hook);
        }
        win32::post(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match chosen.0 as usize {
            ID_CAPTURE => after_fade(hwnd, Mode::Capture, previous),
            ID_CAPTURE_WINDOW => after_fade(hwnd, Mode::Window, previous),
            ID_CAPTURE_SCREEN => after_fade(hwnd, Mode::Screen, previous),
            ID_COPY_TEXT => after_fade(hwnd, Mode::CopyText, previous),
            ID_OPEN_SAVE_FOLDER => save::open_folder(),
            ID_SETTINGS => settings_window::open(hwnd),
            ID_QUIT => {
                win32::destroy(hwnd);
            }
            _ => {}
        }
    }
}

/// Whether Windows fades a chosen menu item out, its "Fade out menu items after clicking" effect.
fn menu_fades() -> bool {
    let setting = |action| {
        let mut on = BOOL(1);
        // SAFETY: Each action asked for here writes one BOOL, to `on`.
        let _ = unsafe {
            SystemParametersInfoW(
                action,
                0,
                Some(&mut on as *mut BOOL as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        on.as_bool()
    };
    // With UI effects off, every effect is off whatever its own setting says.
    setting(SPI_GETUIEFFECTS) && setting(SPI_GETSELECTIONFADE)
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

fn copy_wide<const N: usize>(text: &str, into: &mut [u16; N]) {
    for (slot, unit) in into.iter_mut().zip(text.encode_utf16().take(N - 1)) {
        *slot = unit;
    }
}

fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = load_icon(SM_CXSMICON)?;
    copy_wide("Zilum", &mut nid.szTip);
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        Shell_NotifyIconW(NIM_ADD, &nid).ok()?;
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        Shell_NotifyIconW(NIM_SETVERSION, &nid).ok()?;
    }
    Ok(())
}

/// The Zilum icon at the size the system metric `metric` gives, from the exe's own icon, which
/// the build script embeds as resource 1.
fn load_icon(metric: SYSTEM_METRICS_INDEX) -> Result<HICON> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals; resource 1 is a
    // name the call reads as an integer ID.
    unsafe {
        let size = GetSystemMetricsForDpi(metric, GetDpiForSystem());
        let instance = GetModuleHandleW(None)?;
        let icon = LoadImageW(
            Some(instance.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )?;
        Ok(HICON(icon.0))
    }
}

/// What a start the user made says, naming the capture hotkey if there is one: that Zilum is
/// in the tray, or that it is the new version when it took over from an older copy.
fn running(capture: Option<Hotkey>, took_over: bool) -> String {
    let start = if took_over {
        concat!(
            "Zilum is updated to version ",
            env!("CARGO_PKG_VERSION"),
            "."
        )
    } else {
        "Zilum is running in the tray."
    };
    match capture {
        Some(hotkey) => format!("{start} Press {hotkey} to capture."),
        None => start.into(),
    }
}

fn notify(hwnd: HWND, text: &str) {
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_INFO;
    copy_wide("Zilum", &mut nid.szInfoTitle);
    copy_wide(text, &mut nid.szInfo);
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}
