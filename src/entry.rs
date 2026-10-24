//! The field text is typed into: a native edit control over the editor's image, so the caret's
//! keys, input methods, right-to-left scripts and paste all come from Windows. It has no size and
//! shows nothing, holding the keyboard where the text goes, while the editor draws what it holds;
//! only while an input method composes does it show, as a box around the text in the same font,
//! size and colour at the current zoom, on its halo's colour, since the composition is nowhere
//! else.

use std::cell::Cell;

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DC_BRUSH, DEFAULT_CHARSET,
            DeleteObject, FW_BOLD, GetDC, GetStockObject, GetTextMetricsW, HDC, HFONT,
            OUT_DEFAULT_PRECIS, ReleaseDC, SelectObject, SetBkColor, SetDCBrushColor, SetTextColor,
            TEXTMETRICW,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT,
            },
            WindowsAndMessaging::{
                CallWindowProcW, CreateWindowExW, DestroyWindow, EC_LEFTMARGIN, EC_RIGHTMARGIN,
                ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, GWLP_USERDATA, GetClassInfoExW,
                GetParent, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HideCaret,
                PostMessageW, RegisterClassExW, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW,
                SendMessageW, SetWindowLongPtrW, SetWindowPos, ShowCaret, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_APP, WM_CHAR, WM_CLEAR, WM_CUT, WM_IME_COMPOSITION,
                WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KILLFOCUS, WM_PASTE,
                WM_SETFONT, WM_SYSKEYDOWN, WM_UNDO, WNDCLASSEXW, WNDPROC, WS_CHILD,
            },
        },
    },
    core::{Error, PCWSTR, Result, w},
};

use crate::style::Colour;

/// Posted to the editor when typing ends, which keeps the text: `wparam` is 1 if Esc ended it
/// and 0 otherwise, and `lparam` is the box.
pub const WM_TYPED: u32 = WM_APP + 2;

/// Posted to the editor when a key goes down in the box, which has the keyboard, so the editor
/// still hears of it.
pub const WM_ENTRY_KEY: u32 = WM_APP + 6;

/// Posted to the editor after the text, the caret or the text selected in the box may have changed:
/// `wparam` is 1 when all of it has just been selected, with the caret after it.
pub const WM_ENTRY_CHANGED: u32 = WM_APP + 8;

/// Posted to the editor when an input method starts composing in the box, with `wparam` 1, and
/// when it stops, with 0.
pub const WM_COMPOSING: u32 = WM_APP + 9;

/// Only the `UI_Controls` feature of the windows crate has these.
const EM_GETSEL: u32 = 0x00B0;
const EM_SETSEL: u32 = 0x00B1;
const EM_LINESCROLL: u32 = 0x00B6;
const EM_SETMARGINS: u32 = 0x00D3;

thread_local! {
    /// The edit control's own window procedure, which the box hands everything else to.
    static EDIT: Cell<WNDPROC> = const { Cell::new(None) };
}

/// Makes the box's window class: the edit control's, taking Enter and Esc for itself.
pub fn register(instance: HINSTANCE) -> Result<()> {
    unsafe {
        let mut class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            ..Default::default()
        };
        GetClassInfoExW(None, w!("EDIT"), &mut class)?;
        EDIT.set(class.lpfnWndProc);
        class.lpfnWndProc = Some(entry_proc);
        class.hInstance = instance;
        class.lpszClassName = w!("Zilum.Entry");
        if RegisterClassExW(&class) == 0 {
            return Err(Error::from_thread());
        }
    }
    Ok(())
}

pub struct Entry {
    edit: HWND,
    font: HFONT,
    /// The em the font was made for, and the height of a line in it, in screen pixels.
    em: i32,
    line: i32,
    /// Where the box is in the editor's client area, of no size while it is hidden.
    placed: RECT,
}

impl Entry {
    /// Opens a box in the editor `parent` holding `text`, in `colour` on its halo's colour. It
    /// is there once placed.
    pub fn open(parent: HWND, text: &str, colour: Colour) -> Result<Entry> {
        let text: Vec<u16> = text
            .replace('\n', "\r\n")
            .encode_utf16()
            .chain([0])
            .collect();
        unsafe {
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("Zilum.Entry"),
                PCWSTR(text.as_ptr()),
                // Kept to one line per line break, however long; the editor widens the box.
                WS_CHILD | WINDOW_STYLE((ES_MULTILINE | ES_AUTOHSCROLL | ES_AUTOVSCROLL) as u32),
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            // Where painting the box finds its colours, with no editor to ask.
            let colours = ((bgr(colour) as u64) << 32) | bgr(colour.halo()) as u64;
            SetWindowLongPtrW(edit, GWLP_USERDATA, colours as isize);
            Ok(Entry {
                edit,
                font: HFONT::default(),
                em: 0,
                line: 0,
                placed: RECT::default(),
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.edit
    }

    /// Where the box is in the editor's client area.
    pub fn placed(&self) -> RECT {
        self.placed
    }

    /// The text typed, with each line break as `\n`.
    pub fn text(&self) -> String {
        String::from_utf16_lossy(&self.held()).replace("\r\n", "\n")
    }

    /// Where the text selected starts and ends in the text typed, in UTF-16 units of it with each
    /// line break as `\n`, the same if nothing is selected.
    pub fn selected(&self) -> (usize, usize) {
        let (mut start, mut end) = (0u32, 0u32);
        unsafe {
            SendMessageW(
                self.edit,
                EM_GETSEL,
                Some(WPARAM(&mut start as *mut u32 as usize)),
                Some(LPARAM(&mut end as *mut u32 as isize)),
            );
        }
        // The box holds each line break as `\r\n`, where the text typed has only its `\n`.
        let held = self.held();
        let typed = |index: u32| {
            let index = (index as usize).min(held.len());
            index
                - held[..index]
                    .windows(2)
                    .filter(|pair| pair == &['\r' as u16, '\n' as u16])
                    .count()
        };
        (typed(start), typed(end))
    }

    /// The text in the box as it holds it, with each line break as `\r\n`.
    fn held(&self) -> Vec<u16> {
        unsafe {
            let mut text = vec![0u16; GetWindowTextLengthW(self.edit) as usize + 1];
            let length = GetWindowTextW(self.edit, &mut text) as usize;
            text.truncate(length);
            text
        }
    }

    /// Gives the box the keyboard, with the caret after the text.
    pub fn focus(&self) {
        unsafe {
            let _ = SetFocus(Some(self.edit));
            let end = GetWindowTextLengthW(self.edit) as usize;
            SendMessageW(
                self.edit,
                EM_SETSEL,
                Some(WPARAM(end)),
                Some(LPARAM(end as isize)),
            );
        }
    }

    /// Puts the box with its top-left corner at `left`, `top` in the editor's client area, in a
    /// font `em` screen pixels high. If `shown`, it is around text laid out `width` screen pixels
    /// wide in `lines` lines, with room for the next character; otherwise it has no size.
    pub fn place(&mut self, left: i32, top: i32, em: f32, width: f32, lines: u32, shown: bool) {
        let em = (em.round() as i32).max(1);
        if em != self.em {
            self.set_font(em);
        }
        let placed = if shown {
            RECT {
                left,
                top,
                right: left + width.ceil() as i32 + self.em,
                bottom: top + lines.max(1) as i32 * self.line,
            }
        } else {
            RECT {
                left,
                top,
                right: left,
                bottom: top,
            }
        };
        if placed == self.placed {
            return;
        }
        self.placed = placed;
        unsafe {
            // Windows draws the caret by inverting the screen, and a box that loses its size with
            // the caret on can no longer invert it back, so it would stay on the image.
            let hidden = HideCaret(Some(self.edit)).is_ok();
            let _ = SetWindowPos(
                self.edit,
                None,
                placed.left,
                placed.top,
                placed.right - placed.left,
                placed.bottom - placed.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            if hidden {
                let _ = ShowCaret(Some(self.edit));
            }
            // The box scrolled its text along if it ran out of room before it grew.
            SendMessageW(
                self.edit,
                EM_LINESCROLL,
                Some(WPARAM(-10_000isize as usize)),
                Some(LPARAM(-10_000)),
            );
        }
    }

    /// Writes the box in bold Segoe UI `em` screen pixels high, from its very edge.
    fn set_font(&mut self, em: i32) {
        unsafe {
            let font = CreateFontW(
                -em,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                w!("Segoe UI"),
            );
            SendMessageW(
                self.edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            SendMessageW(
                self.edit,
                EM_SETMARGINS,
                Some(WPARAM((EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize)),
                Some(LPARAM(0)),
            );
            let dc = GetDC(Some(self.edit));
            let before = SelectObject(dc, font.into());
            let mut metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, before);
            ReleaseDC(Some(self.edit), dc);
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
            self.font = font;
            self.em = em;
            self.line = metrics.tmHeight.max(1);
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.edit);
            if !self.font.is_invalid() {
                let _ = DeleteObject(self.font.into());
            }
        }
    }
}

/// What the editor answers when the box `edit` asks how to paint itself on `dc`: its colour on
/// its halo's.
pub fn colours(dc: HDC, edit: HWND) -> LRESULT {
    let colours = unsafe { GetWindowLongPtrW(edit, GWLP_USERDATA) } as u64;
    let (text, back) = (COLORREF((colours >> 32) as u32), COLORREF(colours as u32));
    unsafe {
        SetTextColor(dc, text);
        SetBkColor(dc, back);
        SetDCBrushColor(dc, back);
        LRESULT(GetStockObject(DC_BRUSH).0 as isize)
    }
}

/// `colour` as GDI takes it.
fn bgr(colour: Colour) -> u32 {
    let rgb = colour.rgb();
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// Enter, Esc and the box losing the keyboard each end typing, keeping the text; Shift+Enter
/// breaks the line, and Ctrl+A selects all of it. Each tells the editor
/// afterwards, by a posted message, so the box is never closed while it is still handling a
/// message, as does every change to what it holds and every start and end of a composition.
unsafe extern "system" fn entry_proc(
    edit: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let down = |key: u16| unsafe { GetKeyState(key as i32) } < 0;
    let shift = down(VK_SHIFT.0);
    if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN) {
        tell(edit, WM_ENTRY_KEY, 0);
    }
    let ends = match msg {
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize && !shift => Some(false),
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => Some(true),
        // A multiline edit control selects nothing on Ctrl+A itself.
        WM_KEYDOWN if wparam.0 == 'A' as usize && down(VK_CONTROL.0) && !down(VK_MENU.0) => {
            unsafe { SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1))) };
            tell(edit, WM_ENTRY_CHANGED, 1);
            return LRESULT(0);
        }
        // The characters of those keys, which the edit control would type, and Ctrl+A's.
        WM_CHAR
            if (wparam.0 == '\r' as usize && !shift) || wparam.0 == 0x1B || wparam.0 == 0x01 =>
        {
            return LRESULT(0);
        }
        WM_IME_STARTCOMPOSITION => {
            tell(edit, WM_COMPOSING, 1);
            None
        }
        WM_IME_ENDCOMPOSITION => {
            tell(edit, WM_COMPOSING, 0);
            None
        }
        WM_KILLFOCUS => {
            tell_editor(edit, false);
            None
        }
        _ => None,
    };
    if let Some(escaped) = ends {
        tell_editor(edit, escaped);
        return LRESULT(0);
    }
    let result = unsafe { CallWindowProcW(EDIT.get(), edit, msg, wparam, lparam) };
    // Once the edit control has acted on it, so the editor reads what it holds now.
    if matches!(
        msg,
        WM_KEYDOWN
            | WM_CHAR
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_IME_COMPOSITION
            | WM_IME_ENDCOMPOSITION
    ) {
        tell(edit, WM_ENTRY_CHANGED, 0);
    }
    result
}

/// Posts `msg` with `wparam` to the editor the box `edit` is in.
fn tell(edit: HWND, msg: u32, wparam: usize) {
    unsafe {
        if let Ok(editor) = GetParent(edit) {
            let _ = PostMessageW(Some(editor), msg, WPARAM(wparam), LPARAM(0));
        }
    }
}

/// Tells the editor typing in `edit` has ended, by Esc if `escaped`.
fn tell_editor(edit: HWND, escaped: bool) {
    unsafe {
        if let Ok(editor) = GetParent(edit) {
            let _ = PostMessageW(
                Some(editor),
                WM_TYPED,
                WPARAM(escaped as usize),
                LPARAM(edit.0 as isize),
            );
        }
    }
}
