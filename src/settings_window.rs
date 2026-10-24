//! The Settings window: drawn by Zilum in the theme and frameless like the editor, a title row
//! over four groups in one column, where the user chooses the four hotkeys, the save folder,
//! the theme and whether Zilum starts with Windows. There is no Apply: each choice takes effect,
//! and is written to the settings file, the moment it is made and valid. A hotkey field, once
//! clicked, listens for the next combination pressed and takes it at once. The folder field is a
//! native control, so typing, paste and input methods come from Windows.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT},
                D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
                D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, ID2D1DeviceContext,
            },
            DirectWrite::IDWriteTextLayout,
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN},
                DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG, IDXGISurface, IDXGISwapChain1,
            },
            Gdi::{
                CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DC_BRUSH, DEFAULT_CHARSET,
                DeleteObject, FW_NORMAL, GetDC, GetMonitorInfoW, GetStockObject, GetTextMetricsW,
                HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
                MonitorFromPoint, OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkColor,
                SetDCBrushColor, SetTextColor, TEXTMETRICW, ValidateRect,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::EM_SETSEL,
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetFocus, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
                SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_BACK,
                VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT,
                VK_SPACE, VK_TAB,
            },
            WindowsAndMessaging::{
                CallNextHookEx, CreateWindowExW, ES_AUTOHSCROLL, GetCursorPos, GetDlgItem,
                GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, HC_ACTION, HHOOK, HMENU,
                HTCAPTION, HTCLIENT, HTNOWHERE, IDC_ARROW, IsChild, KBDLLHOOKSTRUCT, LoadCursorW,
                MSG, RegisterClassExW, SC_KEYMENU, SM_CXICON, SM_CXSMICON, SW_SHOW, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
                SetWindowPos, SetWindowTextW, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx,
                WA_INACTIVE, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WINDOWPOS, WM_ACTIVATE,
                WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED,
                WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
                WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_PAINT,
                WM_RBUTTONDOWN, WM_SETFONT, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
                WM_WINDOWPOSCHANGING, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_EX_APPWINDOW,
                WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
            },
        },
    },
    core::{Error, HSTRING, PCWSTR, Result, w},
};
use windows_numerics::Vector2;

use crate::{
    annotation::color,
    background::Background,
    bar::{self, Paint, Tint, Tone},
    editor, frame, graphics,
    hotkey::{self, Hotkey, Key, Mode, Refusal},
    save,
    settings::{self, Settings},
    settings_layout::{self, Layout, Space, Stop, Target},
    startup,
    theme::{self, Palette, Theme},
    win32,
};

/// Posted to the window when the folder picker closes: `lparam` is the folder chosen, boxed, or
/// 0 when it was cancelled.
pub const WM_FOLDER_CHOSEN: u32 = WM_APP + 4;

/// Posted to the window by the keyboard hook for a key pressed while a hotkey field listens:
/// `wparam` is its virtual key, `lparam` the modifiers held, as `MOD_CONTROL`, `MOD_ALT` and
/// `MOD_SHIFT`.
const WM_FIELD_KEY: u32 = WM_APP + 5;

/// Posted to the window after a press on one of its fields, once the field has the keyboard.
const WM_FOCUS_MOVED: u32 = WM_APP + 6;

/// Only the `UI_Controls` feature of the windows crate has it.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// The folder field.
const ID_FOLDER: i32 = 10;

/// No title bar, fixed in size. The thick frame only gives it Windows' shadow, corners and
/// border; nothing resizes it. The system menu lets Alt+F4 and the taskbar close it.
const STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_POPUP.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0);

const TITLE: &str = "Zilum Settings";
const HEADINGS: [&str; 4] = ["Hotkeys", "Saving", "Appearance", "Startup"];
/// Each row's name, in the order of `Layout::labels`.
const LABELS: [&str; 8] = [
    "Capture",
    "Screen",
    "Window",
    "Copy text",
    "Save folder",
    "Theme",
    "Window background",
    "Start with Windows",
];
/// In the order of `Theme::ALL`.
const THEME_NAMES: [&str; 3] = ["System", "Light", "Dark"];
/// In the order of `Background::ALL`.
const BACKGROUND_NAMES: [&str; 3] = ["Trim shadow", "Shadow", "Wallpaper"];
/// Close's icon, as the editor's command bar has it.
const CLOSE_ICON: (char, &str) = ('\u{E8BB}', "\u{00D7}");
/// The folder field's text, in logical pixels, and the room either side of a field's text inside
/// the field.
const FIELD_FONT: f32 = 13.0;
const FIELD_PADDING: f32 = 10.0;
/// How far a clear control's tint sits inside the field's edge.
const CLEAR_INSET: f32 = 4.0;
/// The folder's message, after the hotkey fields' in `State::messages`.
const FOLDER_MESSAGE: usize = 4;
const NOT_A_FOLDER: &str = "Not a valid folder";

thread_local! {
    /// The window while it is open. There is never a second. The keyboard hook reads it, so it is
    /// kept apart from the rest.
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// The tray window, which claims the hotkeys.
    static TRAY: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    /// The mode whose hotkey field shows "Press a key…" and takes the next combination pressed.
    /// The keyboard hook reads it.
    static LISTENING: Cell<Option<Mode>> = const { Cell::new(None) };
    /// The keyboard hook while a field listens.
    static HOOK: Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
    /// The folder field's text and fill as GDI takes them, which it asks for whenever it paints.
    static FIELD_COLOURS: Cell<(COLORREF, COLORREF)> = const { Cell::new((COLORREF(0), COLORREF(0))) };
    /// Set while the window closes, when nothing typed and not yet taken is taken.
    static CLOSING: Cell<bool> = const { Cell::new(false) };
    /// The size the window keeps, as `window_size` last worked it out.
    static SIZE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

/// The open window.
struct State {
    hwnd: HWND,
    swap_chain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    palette: Palette,
    paint: Paint,
    text: bar::Text,
    dpi: u32,
    layout: Layout,
    /// Under each hotkey field and the folder field, empty when there is nothing to say.
    messages: [String; 5],
    /// The control the keyboard is on, if any.
    focus: Option<Stop>,
    /// Whether that control shows the focus ring: once Tab has moved the keyboard, until a click.
    ring: bool,
    /// What the pointer is on, and what the left button went down on while it is held.
    hover: Option<Target>,
    pressed: Option<Target>,
    /// The folder field's font, with the height of a line in it.
    field_font: HFONT,
    field_line: i32,
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    // SAFETY: `window_proc` is a WNDPROC, and the class names and cursors are static.
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Settings"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            // The taskbar and Alt+Tab show these.
            hIcon: crate::load_icon(SM_CXICON)?,
            hIconSm: crate::load_icon(SM_CXSMICON)?,
            // No background brush: the swap chain covers every pixel.
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

/// Opens the window showing the settings as they are, centred on the screen under the pointer,
/// or brings it to the front if it is already open. `tray` claims the hotkeys it changes.
pub fn open(tray: HWND) {
    TRAY.set(tray);
    let open = WINDOW.get();
    if open.is_invalid() {
        let _ = create();
    } else {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let _ = unsafe { SetForegroundWindow(open) };
    }
}

/// Takes the keys the window answers itself before `msg` is dispatched: Tab and Shift+Tab move
/// between the controls, Esc closes the window, Enter takes what the folder field holds or
/// presses Browse, and Space presses the control the keyboard is on. While a hotkey field
/// listens, every key is the field's: the keyboard hook takes the combinations, Esc puts back
/// what the field held, and the rest do nothing. True when it took `msg`.
pub fn handles(msg: &MSG) -> bool {
    let window = WINDOW.get();
    if window.is_invalid()
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        || !(msg.hwnd == window || unsafe { IsChild(window, msg.hwnd) }.as_bool())
    {
        return false;
    }
    let listening = LISTENING.get().is_some();
    let key = VIRTUAL_KEY(msg.wParam.0 as u16);
    match msg.message {
        // Kept from the window too, where releasing Alt would open its menu.
        WM_KEYDOWN | WM_SYSKEYDOWN if listening => {
            if key == VK_ESCAPE {
                with(State::stop_listening);
            }
            true
        }
        WM_KEYUP | WM_SYSKEYUP => listening,
        WM_KEYDOWN | WM_SYSKEYDOWN => match key {
            VK_TAB => {
                let back = win32::key_down(VK_SHIFT);
                with(|state| {
                    state.ring = true;
                    state.move_focus(Some(settings_layout::next(state.focus, back)), true)
                });
                true
            }
            VK_ESCAPE => {
                close(window);
                true
            }
            VK_RETURN => {
                with(State::enter);
                true
            }
            VK_SPACE => with(State::space) == Some(true),
            _ => false,
        },
        // A press on the folder field gives it the keyboard once it is dispatched.
        WM_LBUTTONDOWN | WM_RBUTTONDOWN if msg.hwnd != window => {
            win32::post(window, WM_FOCUS_MOVED, WPARAM(0), LPARAM(0));
            false
        }
        _ => false,
    }
}

/// Runs `f` on the window, if it is open and not already in use further up the stack.
fn with<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|state| state.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Closes the window, first taking what the field the keyboard is in holds, as leaving it would:
/// a folder that cannot be one is not taken, and its text goes with the window.
fn close(window: HWND) {
    with(|state| {
        let focus = state.focus;
        state.leave(focus);
    });
    CLOSING.set(true);
    win32::destroy(window);
}

/// The native field of the window `window` for `stop`, the folder field, or the window itself
/// for a stop it draws and for no stop.
fn native(window: HWND, stop: Option<Stop>) -> HWND {
    match stop {
        Some(Stop::Folder) => control(window, ID_FOLDER),
        _ => window,
    }
}

/// The control of the window `hwnd` with the id `id`.
fn control(hwnd: HWND, id: i32) -> HWND {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe { GetDlgItem(Some(hwnd), id) }.unwrap_or_default()
}

/// Makes `mode`'s hotkey field the one listening, or none, and installs the keyboard hook while
/// one does. A combination another app has registered, or Zilum itself, goes to that app before
/// any window sees its keys; only a low-level hook sees it first, so it can be pressed in a field.
/// Every key on the system waits for the hook, so it is only there while a field listens.
fn set_listening(mode: Option<Mode>) {
    LISTENING.set(mode);
    hook_keyboard(mode.is_some());
}

/// Installs the keyboard hook, or removes it.
fn hook_keyboard(on: bool) {
    let hook = HOOK.get();
    if on != hook.is_invalid() {
        return;
    }
    // SAFETY: `keyboard_hook` is a HOOKPROC, and the hook is this thread's own.
    unsafe {
        if on {
            let instance = GetModuleHandleW(None).ok().map(HINSTANCE::from);
            if let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), instance, 0) {
                HOOK.set(hook);
            }
        } else {
            let _ = UnhookWindowsHookEx(hook);
            HOOK.set(HHOOK::default());
        }
    }
}

/// Takes each press of a key a hotkey can end in, Backspace and Delete, while a field listens and
/// the window is in front, and posts it to the window. Every key pressed anywhere waits for this,
/// so it only asks Windows a few questions that never block, posts and returns.
unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: At HC_ACTION, the lparam of a WH_KEYBOARD_LL hook points to a KBDLLHOOKSTRUCT.
    unsafe {
        let window = WINDOW.get();
        let press = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if code == HC_ACTION as i32
            && press
            && LISTENING.get().is_some()
            && GetForegroundWindow() == window
        {
            let vk = VIRTUAL_KEY((*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode as u16);
            let held = |key: VIRTUAL_KEY| GetAsyncKeyState(key.0 as i32) < 0;
            let usable =
                vk == VK_BACK || vk == VK_DELETE || Key::from_virtual_key(vk.0 as u32).is_some();
            // The Win key is not offered, and its own shortcuts keep working.
            if usable && !held(VK_LWIN) && !held(VK_RWIN) {
                let modifiers = [
                    (VK_CONTROL, MOD_CONTROL),
                    (VK_MENU, MOD_ALT),
                    (VK_SHIFT, MOD_SHIFT),
                ]
                .into_iter()
                .filter(|(key, _)| held(*key))
                .fold(0, |modifiers, (_, modifier)| modifiers | modifier.0);
                let lparam = LPARAM(modifiers as isize);
                win32::post(window, WM_FIELD_KEY, WPARAM(vk.0 as usize), lparam);
                return LRESULT(1);
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }
}

/// `scale`, physical pixels per logical pixel, at `dpi`.
fn scale(dpi: u32) -> f32 {
    dpi as f32 / 96.0
}

/// The size of the window whose client area is laid out as `layout` at `dpi`: the client area
/// with the invisible borders beside and below it. The window keeps this size whatever asks it to
/// change, such as snapping it to a side of the screen.
fn window_size(layout: &Layout, dpi: u32) -> Result<(i32, i32)> {
    let borders = frame::borders(STYLE, dpi)?;
    let size = (
        layout.width + borders.right - borders.left,
        layout.height + borders.bottom,
    );
    SIZE.set(size);
    Ok(size)
}

/// Creates the window centred on the screen under the pointer, at that screen's DPI, and shows
/// it in front.
fn create() -> Result<()> {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        let mut pointer = POINT::default();
        GetCursorPos(&mut pointer)?;
        let monitor = MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).ok()?;
        let (mut dpi, mut _dpi_y) = (96, 96);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
        let layout = Layout::new(scale(dpi), [1; 5]);
        let borders = frame::borders(STYLE, dpi)?;
        let (width, height) = window_size(&layout, dpi)?;
        // The client area, which is all that shows, centred in the work area.
        let work = info.rcWork;
        let left = work.left + (work.right - work.left - layout.width) / 2;
        let top = work.top + (work.bottom - work.top - layout.height) / 2;
        let instance = GetModuleHandleW(None)?;
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            w!("Zilum.Settings"),
            &HSTRING::from(TITLE),
            STYLE,
            left + borders.left,
            top,
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let made = State::new(hwnd, dpi, layout);
        let state = match made {
            Ok(state) => state,
            Err(error) => {
                win32::destroy(hwnd);
                return Err(error);
            }
        };
        frame::round(hwnd);
        frame::border(hwnd, state.palette.dark);
        STATE.with(|slot| *slot.borrow_mut() = Some(state));
        WINDOW.set(hwnd);
        CLOSING.set(false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

impl State {
    /// The window `hwnd` at `dpi`, laid out as `layout`, with its fields made and showing the
    /// settings as they are.
    fn new(hwnd: HWND, dpi: u32, layout: Layout) -> Result<State> {
        let graphics = graphics::get()?;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let instance = unsafe { GetModuleHandleW(None)? };
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let child = |id: i32, class: PCWSTR, style: u32, ex_style: WINDOW_EX_STYLE| unsafe {
            CreateWindowExW(
                ex_style,
                class,
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                0,
                0,
                0,
                0,
                Some(hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(instance.into()),
                None,
            )
        };
        // No border of its own: the window draws the field around it.
        child(
            ID_FOLDER,
            w!("EDIT"),
            ES_AUTOHSCROLL as u32,
            WINDOW_EX_STYLE(0),
        )?;

        let palette = theme::palette(settings::current().theme);
        set_field_colours(&palette);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let context = unsafe {
            graphics
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?
        };
        // One unit is one physical pixel.
        // SAFETY: Direct2D calls on live interfaces and pointers to live locals.
        unsafe { context.SetDpi(96.0, 96.0) };
        let swap_chain =
            graphics.window_swap_chain(hwnd, layout.width as u32, layout.height as u32)?;
        let settings = settings::current();
        let mut state = State {
            hwnd,
            swap_chain,
            paint: Paint::new(&context, &palette)?,
            context,
            palette,
            text: bar::Text::new(scale(dpi))?,
            dpi,
            layout,
            messages: Default::default(),
            focus: None,
            ring: false,
            hover: None,
            pressed: None,
            field_font: HFONT::default(),
            field_line: 0,
        };
        state.make_fonts();
        state.place_controls();
        set_text(
            control(hwnd, ID_FOLDER),
            &settings.save_folder.to_string_lossy(),
        );
        Ok(state)
    }

    /// Gives the folder field Segoe UI for the window's DPI.
    fn make_fonts(&mut self) {
        let size = (FIELD_FONT * scale(self.dpi)).round() as i32;
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            let font = CreateFontW(
                -size,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );
            let field = control(self.hwnd, ID_FOLDER);
            SendMessageW(
                field,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let dc = GetDC(Some(field));
            let before = SelectObject(dc, font.into());
            let mut text = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut text);
            SelectObject(dc, before);
            ReleaseDC(Some(field), dc);
            self.field_line = text.tmHeight.max(1);
            self.drop_font();
            self.field_font = font;
        }
    }

    /// Deletes the folder field's font, which the window made.
    fn drop_font(&mut self) {
        if !self.field_font.is_invalid() {
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let _ = unsafe { DeleteObject(self.field_font.into()) };
        }
    }

    /// Puts the folder field's text inside the field the window draws, centred down it.
    fn place_controls(&self) {
        let folder = self.layout.folder;
        let padding = (FIELD_PADDING * scale(self.dpi)).round();
        let top =
            (folder.top + (folder.bottom - folder.top - self.field_line as f32) / 2.0).round();
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            let _ = SetWindowPos(
                control(self.hwnd, ID_FOLDER),
                None,
                (folder.left + padding) as i32,
                top as i32,
                (folder.right - folder.left - 2.0 * padding) as i32,
                self.field_line,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    /// Lays the window out again for its messages and DPI, resizing it if it changed size.
    fn relayout(&mut self) -> Result<()> {
        let scale = scale(self.dpi);
        // A message's width is the same however long the messages are.
        let room = Layout::new(scale, [1; 5]).messages;
        let mut lines = [1; 5];
        for (at, message) in self.messages.iter().enumerate() {
            if !message.is_empty() {
                let width = room[at].right - room[at].left;
                lines[at] = bar::lines(&self.text.message(message, width)?)?;
            }
        }
        let layout = Layout::new(scale, lines);
        let resized = (layout.width, layout.height) != (self.layout.width, self.layout.height);
        self.layout = layout;
        if resized {
            let (width, height) = window_size(&self.layout, self.dpi)?;
            // SAFETY: Win32 and Direct3D calls on live handles, interfaces and locals.
            unsafe {
                self.swap_chain.ResizeBuffers(
                    0,
                    self.layout.width as u32,
                    self.layout.height as u32,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
        self.place_controls();
        self.present()
    }

    /// Lays the window out again at `dpi`, at its new size, with its top-left corner at `at`, where
    /// Windows suggests.
    fn rescale(&mut self, dpi: u32, at: POINT) -> Result<()> {
        self.dpi = dpi;
        self.text = bar::Text::new(scale(dpi))?;
        self.make_fonts();
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                at.x,
                at.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        // Every length differs at another DPI, so this resizes the window and the swap chain.
        self.relayout()
    }

    /// Draws the window in the palette as it is now, after the theme or Windows' accent changed.
    fn retheme(&mut self) -> Result<()> {
        self.palette = theme::palette(settings::current().theme);
        self.paint = Paint::new(&self.context, &self.palette)?;
        set_field_colours(&self.palette);
        frame::border(self.hwnd, self.palette.dark);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let _ = unsafe { InvalidateRect(Some(control(self.hwnd, ID_FOLDER)), None, true) };
        self.present()
    }

    /// Draws the whole window and shows it.
    fn present(&mut self) -> Result<()> {
        // SAFETY: COM calls on live interfaces and pointers to live locals.
        unsafe {
            let buffer: IDXGISurface = self.swap_chain.GetBuffer(0)?;
            let target = self.context.CreateBitmapFromDxgiSurface(
                &buffer,
                Some(&D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                }),
            )?;
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            self.context.Clear(Some(&color(self.palette.bar)));
            let drawn = self.draw();
            let ended = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn?;
            ended?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    /// Draws everything but the folder field's text. Only between `BeginDraw` and `EndDraw`.
    fn draw(&self) -> Result<()> {
        let (layout, paint, text) = (&self.layout, &self.paint, &self.text);
        let scale = scale(self.dpi);
        let radius = bar::RADIUS * scale;
        let settings = settings::current();

        paint.bar(&D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: layout.width as f32,
            bottom: layout.title.bottom,
        });
        self.beside(&text.label(TITLE)?, &layout.title, Tone::Text)?;
        let (glyph, fallback) = CLOSE_ICON;
        paint.close_button(
            &layout.close,
            self.tint(Target::Close),
            &text.icon(glyph, fallback)?,
        )?;
        for (heading, area) in HEADINGS.iter().zip(&layout.headings) {
            self.beside(&text.heading(heading)?, area, Tone::Text)?;
        }
        for (label, area) in LABELS.iter().zip(&layout.labels) {
            self.beside(&text.label(label)?, area, Tone::Text)?;
        }
        for (message, area) in self.messages.iter().zip(&layout.messages) {
            if !message.is_empty() {
                let width = area.right - area.left;
                let at = Vector2 {
                    X: area.left,
                    Y: area.top,
                };
                paint.text(&text.message(message, width)?, at, Tone::Failed);
            }
        }

        let hotkeys = settings.hotkeys;
        for mode in Mode::ALL {
            let at = mode as usize;
            let (field, clear) = (layout.hotkeys[at], layout.clears[at]);
            paint.field(&field, radius);
            let listening = LISTENING.get() == Some(mode);
            let shown = text.label(&hotkey::field_text(hotkeys[at], listening))?;
            let padding = (FIELD_PADDING * scale).round();
            let inside = D2D_RECT_F {
                left: field.left + padding,
                ..field
            };
            let tone = if listening {
                Tone::Secondary
            } else {
                Tone::Text
            };
            self.beside(&shown, &inside, tone)?;
            // Dimmed, and still, when there is nothing to clear.
            let set = hotkeys[at].is_some();
            if let Some(tint) = self.tint(Target::Clear(mode)).filter(|_| set) {
                let by = (CLEAR_INSET * scale).round();
                let area = D2D_RECT_F {
                    left: clear.left + by,
                    top: clear.top + by,
                    right: clear.right - by,
                    bottom: clear.bottom - by,
                };
                paint.tint(&area, tint, radius);
            }
            let (glyph, fallback) = CLOSE_ICON;
            let tone = if set { Tone::Text } else { Tone::Disabled };
            paint.centred(&text.small_icon(glyph, fallback)?, &clear, tone)?;
        }

        paint.field(&layout.folder, radius);
        paint.field(&layout.browse, radius);
        if let Some(tint) = self.tint(Target::Browse) {
            paint.tint(&layout.browse, tint, radius);
        }
        paint.centred(&text.label("Browse")?, &layout.browse, Tone::Text)?;

        let [first, .., last] = layout.themes;
        let choice = D2D_RECT_F {
            right: last.right,
            ..first
        };
        paint.field(&choice, radius);
        for ((theme, name), area) in Theme::ALL.iter().zip(THEME_NAMES).zip(&layout.themes) {
            let label = text.label(name)?;
            let tint = self.tint(Target::Theme(*theme));
            if *theme == settings.theme {
                paint.accent_button(area, &label, tint, radius)?;
            } else {
                if let Some(tint) = tint {
                    paint.tint(area, tint, radius);
                }
                paint.centred(&label, area, Tone::Text)?;
            }
        }

        for (((background, name), tile), under) in Background::ALL
            .iter()
            .zip(BACKGROUND_NAMES)
            .zip(&layout.tiles)
            .zip(&layout.tile_names)
        {
            paint.tile(
                tile,
                *background != Background::TrimShadow,
                *background == Background::Wallpaper,
                *background == settings.window_background,
                self.tint(Target::Background(*background)),
                scale,
            )?;
            paint.centred(&text.label(name)?, under, Tone::Text)?;
        }

        let on = settings.start_with_windows;
        paint.toggle(
            &layout.toggle,
            on,
            self.tint(Target::StartWithWindows),
            scale,
        );
        let state = text.label(if on { "On" } else { "Off" })?;
        self.beside(&state, &layout.toggle_state, Tone::Text)?;

        let focused = match self.focus.filter(|_| self.ring) {
            Some(Stop::Hotkey(mode)) => Some((layout.hotkeys[mode as usize], radius)),
            Some(Stop::Folder) => Some((layout.folder, radius)),
            Some(Stop::Browse) => Some((layout.browse, radius)),
            Some(Stop::Theme) => Some((choice, radius)),
            Some(Stop::Background) => {
                let [first, .., last] = layout.tiles;
                let tiles = D2D_RECT_F {
                    right: last.right,
                    bottom: layout.tile_names[0].bottom,
                    ..first
                };
                Some((tiles, radius))
            }
            Some(Stop::StartWithWindows) => {
                let half = (layout.toggle.bottom - layout.toggle.top) / 2.0;
                Some((layout.toggle, half))
            }
            None => None,
        };
        if let Some((area, radius)) = focused {
            paint.focus(&area, radius, scale);
        }
        Ok(())
    }

    /// `layout` at the left of `area`, centred down it.
    fn beside(&self, layout: &IDWriteTextLayout, area: &D2D_RECT_F, tone: Tone) -> Result<()> {
        let height = bar::size(layout)?.Y;
        let at = Vector2 {
            X: area.left,
            Y: (area.top + (area.bottom - area.top - height) / 2.0).round(),
        };
        self.paint.text(layout, at, tone);
        Ok(())
    }

    /// The tint behind `target`: pressed while the left button holds it and the pointer is on it,
    /// hovered while the pointer is on it and no button is held.
    fn tint(&self, target: Target) -> Option<Tint> {
        match (self.pressed, self.hover) {
            (Some(pressed), Some(hover)) if pressed == target && hover == target => {
                Some(Tint::Pressed)
            }
            (None, Some(hover)) if hover == target => Some(Tint::Hover),
            _ => None,
        }
    }

    /// Gives the keyboard to `to`, or to the window itself when that is no control, and takes
    /// what the folder field holds if it left it. A hotkey field it comes to listens if `listen`;
    /// any other field that was listening stops and shows what it held.
    fn move_focus(&mut self, to: Option<Stop>, listen: bool) {
        let from = self.focus;
        self.focus = to;
        set_listening(match to {
            Some(Stop::Hotkey(mode)) if listen => Some(mode),
            _ => None,
        });
        let field = native(self.hwnd, to);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        unsafe {
            let _ = SetFocus(Some(field));
            // A dialog selects the text of a field the keyboard comes to, so typing replaces it.
            if to == Some(Stop::Folder) && from != to {
                SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            }
        }
        if from != to {
            self.leave(from);
        }
        let _ = self.present();
    }

    /// Follows the keyboard into the folder field when the user clicked it.
    fn focus_moved(&mut self) {
        let folder = Some(Stop::Folder);
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        if native(self.hwnd, folder) == unsafe { GetFocus() } && self.focus != folder {
            let from = self.focus;
            self.focus = folder;
            self.ring = false;
            set_listening(None);
            self.leave(from);
            let _ = self.present();
        }
    }

    /// Gives the keyboard back to the control that had it when the window was last left.
    fn restore_focus(&self) {
        // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
        let _ = unsafe { SetFocus(Some(native(self.hwnd, self.focus))) };
    }

    /// Stops the hotkey field listening, if one is, so it shows what it held again.
    fn stop_listening(&mut self) {
        if LISTENING.get().is_some() {
            set_listening(None);
            let _ = self.present();
        }
    }

    /// Takes what the folder field holds if the keyboard has just left it, unless the window is
    /// closing, which has taken it already.
    fn leave(&mut self, left: Option<Stop>) {
        if left == Some(Stop::Folder) && !CLOSING.get() {
            self.take_folder();
        }
    }

    /// Enter: takes what the folder field holds, or presses Browse.
    fn enter(&mut self) {
        match self.focus {
            Some(Stop::Folder) => self.take_folder(),
            Some(Stop::Browse) => self.browse(),
            _ => {}
        }
    }

    /// Space: presses the control the keyboard is on. False when the folder field takes it.
    fn space(&mut self) -> bool {
        let Some(stop) = self.focus else {
            return false;
        };
        let settings = settings::current();
        match settings_layout::space(
            stop,
            settings.theme,
            settings.window_background,
            settings.start_with_windows,
        ) {
            Space::Typed => return false,
            Space::Listen => self.move_focus(Some(stop), true),
            Space::Browse => self.browse(),
            Space::Theme(theme) => self.choose_theme(theme),
            Space::Background(background) => self.choose_background(background),
            Space::StartWithWindows(on) => self.start_with_windows(on),
        }
        true
    }

    /// Does what clicking `target` does, once the button comes up on it. A hotkey field listens
    /// from the press.
    fn press(&mut self, target: Target) {
        match target {
            Target::Close | Target::Hotkey(_) => {}
            Target::Clear(mode) => self.set_hotkey(mode, None),
            Target::Browse => self.browse(),
            Target::Theme(theme) => self.choose_theme(theme),
            Target::Background(background) => self.choose_background(background),
            Target::StartWithWindows => {
                self.start_with_windows(!settings::current().start_with_windows)
            }
        }
    }

    /// The combination `pressed` was pressed, or `None` for Backspace or Delete: the field that
    /// was listening stops and takes it.
    fn take_pressed(&mut self, pressed: Option<Hotkey>) {
        if let Some(mode) = LISTENING.get() {
            set_listening(None);
            self.set_hotkey(mode, pressed);
        }
    }

    /// Makes `pressed` `mode`'s hotkey at once, the old one stopping, and writes the file. When it
    /// is refused, the hotkey before stays in effect and the field says why under it. When it is
    /// the hotkey already, the field is unchanged, and so is what it says.
    fn set_hotkey(&mut self, mode: Mode, pressed: Option<Hotkey>) {
        let current = settings::current();
        if pressed != current.hotkeys[mode as usize] {
            let refusal = match hotkey::refusal(current.hotkeys, mode, pressed) {
                Some(refusal) => Some(refusal),
                None => claim(mode, pressed),
            };
            let message =
                refusal.map_or_else(String::new, |refusal| hotkey::message(refusal, pressed));
            self.set_message(mode as usize, message);
        }
        let _ = self.present();
    }

    /// Makes the folder field's text the save folder and writes the file, if it can be a folder;
    /// otherwise says so, and the folder before stays in use.
    fn take_folder(&mut self) {
        let current = settings::current();
        match settings::valid_folder(&text(control(self.hwnd, ID_FOLDER))) {
            Some(folder) => {
                if folder != current.save_folder {
                    settings::store(Settings {
                        save_folder: folder,
                        ..current
                    });
                }
                self.set_message(FOLDER_MESSAGE, String::new());
            }
            None => self.set_message(FOLDER_MESSAGE, NOT_A_FOLDER.to_owned()),
        }
    }

    /// Opens the folder picker, starting at the folder in the field if it can be one, or else at
    /// the save folder.
    fn browse(&self) {
        let start = settings::valid_folder(&text(control(self.hwnd, ID_FOLDER)))
            .unwrap_or_else(|| settings::current().save_folder);
        save::choose_folder(start, self.hwnd);
    }

    /// Makes `theme` the theme, in every open Zilum window at once, and writes the file.
    fn choose_theme(&mut self, theme: Theme) {
        let current = settings::current();
        if theme == current.theme {
            return;
        }
        settings::store(Settings { theme, ..current });
        editor::retheme_all();
        let _ = self.retheme();
    }

    /// Makes `window_background` the next window capture's, and writes the file.
    fn choose_background(&mut self, window_background: Background) {
        let current = settings::current();
        if window_background != current.window_background {
            settings::store(Settings {
                window_background,
                ..current
            });
        }
        let _ = self.present();
    }

    /// Turns starting with Windows `on` or off, and writes the file.
    fn start_with_windows(&mut self, on: bool) {
        settings::store(Settings {
            start_with_windows: on,
            ..settings::current()
        });
        startup::set(on);
        let _ = self.present();
    }

    /// Says `message` under the field it is for, the folder's or `Mode::ALL[index]`'s, or nothing
    /// when it is empty.
    fn set_message(&mut self, index: usize, message: String) {
        if self.messages[index] != message {
            self.messages[index] = message;
            let _ = self.relayout();
        }
    }

    /// Follows the pointer, now at `at` in the client area. True if it is on something.
    fn hover_at(&mut self, at: POINT) -> bool {
        let hover = self.layout.hit((at.x as f32, at.y as f32));
        if hover != self.hover {
            self.hover = hover;
            let _ = self.present();
        }
        hover.is_some()
    }
}

/// Moves `mode`'s hotkey to `pressed` for the tray and writes the file, or says why the hotkey
/// before stays.
fn claim(mode: Mode, pressed: Option<Hotkey>) -> Option<Refusal> {
    let current = settings::current();
    let mut wanted = current.hotkeys;
    wanted[mode as usize] = pressed;
    // Claiming may wait half a second to retry, and every key pressed anywhere would wait with
    // the keyboard hook.
    set_listening(None);
    let refused = crate::rebind(TRAY.get(), current.hotkeys, wanted)[mode as usize];
    if refused.is_none() {
        settings::store(Settings {
            hotkeys: wanted,
            ..current
        });
    }
    refused
}

/// Makes the folder field paint in `palette`'s text and field colours.
fn set_field_colours(palette: &Palette) {
    FIELD_COLOURS.set((bgr(palette.text), bgr(palette.field)));
}

/// `rgb` as GDI takes it.
fn bgr(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

fn text(control: HWND) -> String {
    // SAFETY: The buffer has room for the text and its terminator.
    unsafe {
        let mut text = vec![0u16; GetWindowTextLengthW(control) as usize + 1];
        let length = GetWindowTextW(control, &mut text) as usize;
        String::from_utf16_lossy(&text[..length])
    }
}

fn set_text(control: HWND, text: &str) {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    let _ = unsafe { SetWindowTextW(control, &HSTRING::from(text)) };
}

/// The pointer in a mouse message, in physical client pixels.
fn pointer(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let _ = with(State::present);
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
        }
        // The swap chain covers the whole window; erasing would only flicker.
        WM_ERASEBKGND => return LRESULT(1),
        // The folder field paints in the theme, as the field drawn around it does.
        WM_CTLCOLOREDIT => {
            let (ink, fill) = FIELD_COLOURS.get();
            let dc = HDC(wparam.0 as *mut _);
            // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
            unsafe {
                SetTextColor(dc, ink);
                SetBkColor(dc, fill);
                SetDCBrushColor(dc, fill);
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
        }
        // Windows' app mode or accent changed.
        _ if theme::changed(msg, lparam) => {
            let _ = with(State::retheme);
            return win32::default_proc(hwnd, msg, wparam, lparam);
        }
        WM_NCCALCSIZE => frame::calc_size(hwnd, STYLE, wparam, lparam),
        // Windows paints the classic sizing border over the top of the client area, where its
        // own frame would be, in the active or inactive border colour, when the window gains or
        // loses the front: a light band above the title. A -1 asks it to change the state
        // without the painting.
        WM_NCACTIVATE => return win32::default_proc(hwnd, msg, wparam, LPARAM(-1)),
        // The title row moves the window, except over Close; nothing resizes it.
        WM_NCHITTEST => {
            let at = win32::to_client(hwnd, pointer(lparam));
            let part = with(|state| {
                let layout = &state.layout;
                let inside = at.x >= 0 && at.x < layout.width && at.y >= 0 && at.y < layout.height;
                if !inside {
                    HTNOWHERE
                } else if (at.y as f32) < layout.title.bottom
                    && layout.hit((at.x as f32, at.y as f32)).is_none()
                {
                    HTCAPTION
                } else {
                    HTCLIENT
                }
            });
            return LRESULT(part.unwrap_or(HTCLIENT) as isize);
        }
        WM_NCLBUTTONDBLCLK => {}
        // A press on the title row, or a right-click anywhere, is a click elsewhere too: a
        // listening field stops and shows what it held.
        WM_NCLBUTTONDOWN => {
            with(State::stop_listening);
            return win32::default_proc(hwnd, msg, wparam, lparam);
        }
        WM_RBUTTONDOWN => {
            with(State::stop_listening);
        }
        // Fixed in size: snapping or maximising it moves it at most.
        WM_WINDOWPOSCHANGING => {
            // SAFETY: The lparam of WM_WINDOWPOSCHANGING points to the WINDOWPOS to change.
            let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
            let (width, height) = SIZE.get();
            if position.flags.0 & SWP_NOSIZE.0 == 0 && width > 0 {
                (position.cx, position.cy) = (width, height);
            }
            return win32::default_proc(hwnd, msg, wparam, lparam);
        }
        // Alt alone would open the window's menu, which it never shows.
        WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU as usize => {}
        WM_FOLDER_CHOSEN => {
            if lparam.0 != 0 {
                // SAFETY: A nonzero lparam is the Box<PathBuf> post_path leaked for this message.
                let folder = unsafe { Box::from_raw(lparam.0 as *mut PathBuf) };
                set_text(control(hwnd, ID_FOLDER), &folder.to_string_lossy());
                with(State::take_folder);
            }
        }
        WM_FIELD_KEY => {
            let vk = VIRTUAL_KEY(wparam.0 as u16);
            let modifiers = lparam.0 as u32 & 0xFF;
            let typed = if vk == VK_BACK || vk == VK_DELETE {
                Some(None)
            } else {
                Key::from_virtual_key(vk.0 as u32).map(|key| {
                    let held = |modifier: HOT_KEY_MODIFIERS| modifiers & modifier.0 != 0;
                    Some(Hotkey {
                        ctrl: held(MOD_CONTROL),
                        alt: held(MOD_ALT),
                        shift: held(MOD_SHIFT),
                        key,
                    })
                })
            };
            if let Some(typed) = typed {
                with(|state| state.take_pressed(typed));
            }
        }
        WM_FOCUS_MOVED => {
            with(State::focus_moved);
        }
        WM_MOUSEMOVE => {
            if with(|state| state.hover_at(pointer(lparam))) == Some(true) {
                // So the tint goes when the pointer leaves the window.
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
                let _ = unsafe { TrackMouseEvent(&mut track) };
            }
        }
        WM_MOUSELEAVE => {
            with(|state| state.hover_at(POINT { x: -1, y: -1 }));
        }
        // A press takes the keyboard to what it lands on, or off any field onto the window,
        // except on Close, which takes nothing typed. A press on a hotkey field makes it listen;
        // one on its clear control does not.
        WM_LBUTTONDOWN => {
            let at = pointer(lparam);
            with(|state| {
                let target = state.layout.hit((at.x as f32, at.y as f32));
                state.pressed = target;
                let (stop, listen) = match target {
                    Some(Target::Close) => {
                        let _ = state.present();
                        return;
                    }
                    Some(Target::Hotkey(mode)) => (Some(Stop::Hotkey(mode)), true),
                    Some(Target::Clear(mode)) => (Some(Stop::Hotkey(mode)), false),
                    Some(Target::Browse) => (Some(Stop::Browse), false),
                    Some(Target::Theme(_)) => (Some(Stop::Theme), false),
                    Some(Target::Background(_)) => (Some(Stop::Background), false),
                    Some(Target::StartWithWindows) => (Some(Stop::StartWithWindows), false),
                    None => (None, false),
                };
                state.ring = false;
                state.move_focus(stop, listen);
            });
            win32::capture(hwnd);
        }
        WM_LBUTTONUP => {
            let at = pointer(lparam);
            let pressed = with(|state| {
                let pressed = state.pressed.take();
                let _ = state.present();
                pressed.filter(|pressed| {
                    state.layout.hit((at.x as f32, at.y as f32)) == Some(*pressed)
                })
            })
            .flatten();
            win32::release_capture();
            match pressed {
                Some(Target::Close) => close(hwnd),
                Some(target) => {
                    with(|state| state.press(target));
                }
                None => {}
            }
        }
        WM_CAPTURECHANGED => {
            with(|state| {
                if state.pressed.take().is_some() {
                    let _ = state.present();
                }
            });
        }
        // The keyboard leaving the window stops a hotkey field listening and leaves the folder
        // field, which takes what it holds; coming back, it goes to the control that had it.
        WM_ACTIVATE => {
            let active = wparam.0 & 0xFFFF != WA_INACTIVE as usize;
            with(|state| {
                if active {
                    state.restore_focus();
                } else {
                    state.stop_listening();
                    let focus = state.focus;
                    state.leave(focus);
                }
            });
            if !active {
                return win32::default_proc(hwnd, msg, wparam, lparam);
            }
        }
        WM_CLOSE => close(hwnd),
        WM_DPICHANGED => {
            // SAFETY: The lparam of WM_DPICHANGED points to the suggested RECT.
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let at = POINT {
                x: suggested.left,
                y: suggested.top,
            };
            let _ = with(|state| state.rescale(dpi, at));
        }
        WM_DESTROY => {
            set_listening(None);
            WINDOW.set(HWND(std::ptr::null_mut()));
            // The window is only closed from outside `with`, so the state is free here.
            if let Some(mut state) = STATE.with(|slot| slot.try_borrow_mut().ok()?.take()) {
                state.drop_font();
            }
            CLOSING.set(false);
        }
        _ => return win32::default_proc(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}
