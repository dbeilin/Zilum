//! The editor: a window per capture that shows it, zoomed and panned as the user likes, takes
//! arrows, rectangles and blurs drawn on it, lets them be selected, moved, resized and deleted,
//! crops it, undoes each of those, and copies or saves it with the annotations drawn in, which
//! closes it, or closes without either. It opens untouched, with the keyboard, and leaves on its
//! own, handing the keyboard back to the window the user was in; a click, a wheel turn or a key
//! on it enters it, and from then on it only closes when the user closes it.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D_RECT_U, D2D_SIZE_U, D2D1_ALPHA_MODE, D2D1_ALPHA_MODE_IGNORE,
                    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
                },
                D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ,
                D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
                D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_MAP_OPTIONS_READ, D2D1_ROUNDED_RECT,
                ID2D1Bitmap1, ID2D1DeviceContext, ID2D1SolidColorBrush,
            },
            DirectWrite::IDWriteTextLayout,
            Dxgi::{
                Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN},
                DXGI_PRESENT, DXGI_PRESENT_PARAMETERS, DXGI_SWAP_CHAIN_FLAG, IDXGISurface,
                IDXGISwapChain1,
            },
            Gdi::{
                ClientToScreen, GetMonitorInfoW, HDC, InvalidateRect, MONITOR_DEFAULTTONEAREST,
                MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
                ScreenToClient, ValidateRect,
            },
        },
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            LibraryLoader::GetModuleHandleW,
            SystemInformation::GetTickCount,
        },
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                GetCapture, GetFocus, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture,
                SetFocus, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TRACKMOUSEEVENT_FLAGS,
                TrackMouseEvent, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_ESCAPE, VK_F4,
                VK_LEFT, VK_LWIN, VK_MENU, VK_NUMPAD0, VK_NUMPAD1, VK_NUMPAD8, VK_RETURN, VK_RIGHT,
                VK_RWIN, VK_SHIFT, VK_UP,
            },
            Shell::{ITaskbarList, TaskbarList},
            WindowsAndMessaging::{
                CS_DBLCLKS, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
                EN_CHANGE, EVENT_SYSTEM_FOREGROUND, GW_HWNDNEXT, GWL_EXSTYLE, GWL_STYLE,
                GetCaretBlinkTime, GetClientRect, GetCursorPos, GetForegroundWindow, GetTopWindow,
                GetWindow, GetWindowLongPtrW, GetWindowRect, HC_ACTION, HHOOK, HTCAPTION,
                HWND_NOTOPMOST, HWND_TOPMOST, IDC_ARROW, KillTimer, LWA_ALPHA, LoadCursorW,
                MINMAXINFO, MSLLHOOKSTRUCT, PostMessageW, RegisterClassExW, SM_CXICON, SM_CXSMICON,
                SPI_GETCLIENTAREAANIMATION, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                SetForegroundWindow, SetLayeredWindowAttributes, SetTimer,
                SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW,
                ShowWindow, SystemParametersInfoW, UnhookWindowsHookEx, WDA_EXCLUDEFROMCAPTURE,
                WH_MOUSE_LL, WHEEL_DELTA, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT, WM_APP,
                WM_CAPTURECHANGED, WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED,
                WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
                WM_NCACTIVATE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
                WM_NCMBUTTONDOWN, WM_NCMOUSEMOVE, WM_NCRBUTTONDOWN, WM_NCXBUTTONDOWN, WM_PAINT,
                WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN,
                WNDCLASSEXW, WS_CLIPCHILDREN, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
                WS_EX_TOPMOST, WS_POPUP, WS_THICKFRAME, WindowFromPoint,
            },
        },
    },
    core::{BOOL, Error, Result, w},
};
use windows_numerics::{Matrix3x2, Vector2};

use crate::{
    annotation::{self, Annotation, Badge, Bounds, Handle, Kind, Pens, Side, Stroke, Text, color},
    bar::{self, Paint, Tint, Tone},
    caret::{self, Caret},
    clipboard,
    command_bar::{self, Command, Group, Layout, Widths},
    entry::{self, Entry, WM_COMPOSING, WM_ENTRY_CHANGED, WM_ENTRY_KEY, WM_TYPED},
    flyout::{self, Shown, StyleFlyout},
    frame::{self, Frame, Part},
    graphics,
    history::History,
    image::{Image, Region, Resize},
    lifetime::{self, Cause, Lifetime},
    notice, picker,
    save::{self, SavedFile},
    settings::{self, Settings},
    style::{Choice, Colour, Fill, Head, Level, Size, Sizes, Span, Style, TextSize},
    theme::{self, Palette},
    tooltip::Tooltip,
    view::{View, Zoom},
    window,
};

/// Posted to an editor when a copy it asked for has finished. `wparam` is 1 if the capture is
/// on the clipboard.
pub const WM_COPIED: u32 = WM_APP;

/// Posted to an editor when a save it asked for has finished. `wparam` is 1 if the capture is
/// in its file.
pub const WM_SAVED: u32 = WM_APP + 1;

/// Posted to an editor when its Save As dialog has closed. `lparam` is the file chosen, as a
/// `Box<PathBuf>` the editor takes, or null if the user cancelled.
pub const WM_FILE_CHOSEN: u32 = WM_APP + 3;

/// Posted to an untouched editor when a mouse button went down anywhere on the screen. `lparam`
/// is the point, in physical screen pixels.
const WM_PRESSED: u32 = WM_APP + 7;

/// Only the `UI_Controls` feature of the windows crate has it.
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Only the `UI_Controls` feature of the windows crate has it.
const WM_NCMOUSELEAVE: u32 = 0x02A2;

/// No title bar and no system menu, but resizable, which gives it Windows' shadow, corners and
/// invisible resize borders. Presenting leaves the box text is typed into as it is.
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_POPUP.0 | WS_THICKFRAME.0 | WS_CLIPCHILDREN.0);

/// How far along each edge from a corner resizing at the corner reaches, in logical pixels.
const CORNER_REACH: f32 = 16.0;

thread_local! {
    /// The open editors. Only the UI thread has any.
    static EDITORS: RefCell<Vec<Editor>> = const { RefCell::new(Vec::new()) };
    /// Watches for another app coming to the front while an editor is open.
    static FOREGROUND_HOOK: Cell<HWINEVENTHOOK> = const { Cell::new(HWINEVENTHOOK(std::ptr::null_mut())) };
    /// Watches every press on the screen while an untouched editor is open.
    static MOUSE_HOOK: Cell<HHOOK> = const { Cell::new(HHOOK(std::ptr::null_mut())) };
    /// When the newest editor opened, as a tick count.
    static OPENED_AT: Cell<u32> = const { Cell::new(0) };
    /// The untouched editor the mouse hook tells about presses, if there is one. The hook runs on
    /// this thread.
    static PRESSES_TO: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    /// The colour last chosen in any editor, which the next annotation is placed in.
    static CURRENT_COLOUR: Cell<Colour> = const { Cell::new(Colour::FIRST_RUN) };
    /// The size each tool's next annotation is placed at, as it was last set in any editor.
    static CURRENT_SIZES: Cell<Sizes> = const { Cell::new(Sizes::FIRST_RUN) };
    /// The em of the next text, as the last text committed or resized by its handles in any
    /// editor left it.
    static CURRENT_TEXT_SIZE: Cell<TextSize> = const { Cell::new(TextSize::FIRST_RUN) };
    /// Which ends of the next arrow carry a head, as it was last chosen in any editor.
    static CURRENT_HEAD: Cell<Head> = const { Cell::new(Head::FIRST_RUN) };
    /// How the next rectangle is drawn, as it was last chosen in any editor.
    static CURRENT_FILL: Cell<Fill> = const { Cell::new(Fill::FIRST_RUN) };
    /// The level of the next blur, as it was last set in any editor.
    static CURRENT_LEVEL: Cell<Level> = const { Cell::new(Level::FIRST_RUN) };
}

/// What a left drag on the image does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Selects, moves and resizes annotations, and pans where there are none.
    Select,
    Draw(Kind),
    /// Draws a stroke where the pointer is dragged.
    Pen,
    /// A click opens a box to type text into.
    Text,
    /// A click places the next badge.
    Counter,
    /// Shows the colour under the pointer, and a click copies its hex code.
    Picker,
    /// Chooses a rectangle to crop the image to.
    Crop,
}

impl Tool {
    /// Every tool, in the command bar's order.
    const ALL: [Tool; 9] = [
        Tool::Select,
        Tool::Draw(Kind::Arrow),
        Tool::Draw(Kind::Rectangle),
        Tool::Pen,
        Tool::Text,
        Tool::Counter,
        Tool::Draw(Kind::Blur),
        Tool::Picker,
        Tool::Crop,
    ];

    /// The tool the command bar's `command` makes active, if it is a tool's.
    fn of(command: Command) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.command() == command)
    }

    /// The command bar's button that makes it active.
    fn command(self) -> Command {
        match self {
            Tool::Select => Command::Select,
            Tool::Draw(Kind::Arrow) => Command::Arrow,
            Tool::Draw(Kind::Rectangle) => Command::Rect,
            Tool::Pen => Command::Pen,
            Tool::Text => Command::Text,
            Tool::Counter => Command::Counter,
            Tool::Draw(Kind::Blur) => Command::Blur,
            Tool::Picker => Command::Picker,
            Tool::Crop => Command::Crop,
        }
    }
}

/// A drag the left button is making on the image, or on Level or Size.
#[derive(Clone)]
enum Drag {
    /// Panning, with the pointer last at this client point.
    Pan(POINT),
    /// Drawing an annotation of `kind`, from where the drag started to where the pointer is, in
    /// capture pixels. Once the drag has been `marked`, large enough to place it, it shows however
    /// thin it gets, so crossing the start's row or column does not blink it off.
    Draw {
        kind: Kind,
        from: Vector2,
        to: Vector2,
        marked: bool,
    },
    /// Drawing a stroke along the path the pointer has taken, in capture pixels, smoothed over
    /// points `spacing` capture pixels apart. Once the pointer has gone far enough to place it, it
    /// is `marked` and shows.
    Pen {
        path: Vec<Vector2>,
        spacing: f32,
        marked: bool,
    },
    /// Moving the annotation at `index`, or with `handle` resizing it, by as far as the pointer
    /// has come from `from` to `to`, in capture pixels. The annotation is changed only on release.
    Adjust {
        index: usize,
        handle: Option<Handle>,
        from: Vector2,
        to: Vector2,
    },
    /// Choosing a crop, from the pixel corner where the drag started to the one nearest the
    /// pointer, in capture pixels. Once the drag has been `marked`, large enough to choose a crop,
    /// it shows however thin it gets, so crossing the start's row or column does not blink the
    /// dimming off or back to the crop as it was.
    Crop {
        from: Vector2,
        to: Vector2,
        marked: bool,
    },
    /// Moving the edges of the crop that the handle `across` and `down` is on by as far as the
    /// pointer has come from corner `from` to corner `to`. The crop is changed only on release.
    AdjustCrop {
        across: Side,
        down: Side,
        from: Vector2,
        to: Vector2,
    },
    /// Moving Level's knob to `level`, held `grab` pixels left of the pointer, for the blur
    /// selected at `index`, which follows it live and is changed only on release.
    Level {
        index: usize,
        level: Level,
        grab: f32,
    },
    /// Moving the Size slider's knob to `size`, held `grab` pixels left of the pointer, for the
    /// annotation selected at `index`, which follows it live and is changed only on release.
    Size { index: usize, size: Size, grab: f32 },
}

impl Drag {
    /// Whether the image shows it as it goes: anything but a pan.
    fn shows(&self) -> bool {
        !matches!(self, Drag::Pan(_))
    }
}

/// A blur as it is baked into the picture: the pixels it covers, the pixel its blocks start from,
/// which may lie outside the image, and the side of its blocks.
type Blur = (Region, [i64; 2], usize);

/// Text being typed: the box, where the text's top-left corner goes in capture pixels, its colour
/// and its em in capture pixels, and the text annotation it edits again, as its index, if it does.
struct Typing {
    entry: Entry,
    at: Vector2,
    colour: Colour,
    font: f32,
    editing: Option<usize>,
    /// Where the text selected in the box starts and ends, and which end the caret is at.
    selected: (usize, usize),
    caret: usize,
    /// Whether an input method is composing, which only the box can show.
    composing: bool,
    /// Whether the caret shows, between blinks.
    blink: bool,
}

impl Typing {
    /// Reads where the text selected is now, and the caret with it, which is after all of it if
    /// `all` was just selected.
    fn follow(&mut self, all: bool) {
        let now = self.entry.selected();
        self.caret = if all {
            now.1
        } else {
            caret::moved(self.selected, self.caret, now)
        };
        self.selected = now;
    }
}

/// What an edit changes, as it was before it, so undoing restores it exactly, or as it was before
/// an undo, so redoing does.
struct Step {
    annotations: Vec<Annotation>,
    selected: Option<usize>,
    /// The image, which only a crop or a Resize changes, its picture, and the zoom it was shown
    /// at.
    image: Arc<Image>,
    picture: ID2D1Bitmap1,
    zoom: Zoom,
}

/// What a copy does to the editor once it is on the clipboard.
#[derive(Clone, Copy, PartialEq)]
enum AfterCopy {
    /// The capture's own copy and a pick's leave it open.
    Stays,
    /// Copying the image closes it, unless it is pinned.
    Closes,
}

#[derive(Clone, PartialEq)]
enum Status {
    Copied,
    /// The Picker copied the hex code of `colour`, a pixel's blue, green, red and alpha as the
    /// editor shows it. `pointer` is where on the screen it was picked, which the Notice that
    /// reports it shows beside.
    Picked {
        colour: [u8; 4],
        pointer: POINT,
    },
    NotCopied,
    NotSaved,
}

impl Status {
    fn text(&self) -> String {
        match self {
            Status::Copied => "Copied".to_owned(),
            Status::Picked { colour, .. } => format!("Copied {}", picker::hex(*colour)),
            Status::NotCopied => "Not copied".to_owned(),
            Status::NotSaved => "Couldn't save".to_owned(),
        }
    }

    /// Every status the bar makes room for at its narrowest. Picks of #000000, #111111 and so on
    /// to #FFFFFF stand for every hex code: a code is no wider than the one repeating its widest
    /// digit.
    fn to_fit() -> impl Iterator<Item = Status> {
        let picks = (0..16u8).map(|digit| Status::Picked {
            colour: [digit * 0x11, digit * 0x11, digit * 0x11, 0xFF],
            pointer: POINT::default(),
        });
        [Status::Copied, Status::NotCopied, Status::NotSaved]
            .into_iter()
            .chain(picks)
    }
}

struct Editor {
    hwnd: HWND,
    /// The tray window, which shows notifications.
    tray: HWND,
    /// The capture, cropped as the user has cropped it.
    image: Arc<Image>,
    /// The file Save wrote or Save As chose, shared with the save worker.
    file: SavedFile,
    /// Whether the Save As dialog is open. Opening it enters the editor, which then stays.
    choosing_file: bool,
    swap_chain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    picture: ID2D1Bitmap1,
    /// The picture, with any blurs, scaled down once with high quality, with what it was made from
    /// and the scale it was made for, so below 100 percent panning moves the same pixels instead
    /// of filtering them anew, which shimmers.
    scaled: Option<(ID2D1Bitmap1, f32, ID2D1Bitmap1)>,
    /// The picture with the blurs baked in, and the blurs it was made with, until they change.
    blurred: Option<(Vec<Blur>, ID2D1Bitmap1)>,
    /// The blocks of each blur in `blurred`, so a blur is worked out once however often the others
    /// change.
    blocks: HashMap<Blur, Vec<u8>>,
    ink: Ink,
    pens: Pens,
    toolbar: Toolbar,
    tool: Tool,
    /// In the order they were placed.
    annotations: Vec<Annotation>,
    /// The annotation showing its handles, as its index.
    selected: Option<usize>,
    /// The crop being chosen, not yet applied, between pixel corners of the image.
    crop: Option<Bounds>,
    /// Text being typed, which the box has the keyboard for. Any click in the editor outside the
    /// box ends it first, so no other edit is made meanwhile.
    typing: Option<Typing>,
    /// Whether it is a pinned picture: the image alone, above every other window, which only
    /// moves, zooms, copies, saves, closes and comes back as the editor.
    pinned: bool,
    /// Where a pinned picture's top-left is on the screen, exactly; its window is there to the
    /// nearest pixel.
    corner: Vector2,
    /// What each edit changed, as it was before it, and what each undo took away.
    history: History<Step>,
    /// Whether it is untouched and what closes it, or entered and stays until the user closes it.
    lifetime: Lifetime,
    /// The window that had the front before the editor took it as it opened, which gets it back
    /// when the untouched editor leaves on its own, and which the window hotkey captures while the
    /// editor is in front.
    previous: Option<HWND>,
    /// The capture with its annotations drawn in, once copying or saving has made it, until the
    /// next edit.
    output: Option<Arc<Image>>,
    /// How many edits have been made, so a copy or save finishing after a later edit, which is of
    /// the image before it, changes no status and closes nothing.
    edits: u32,
    /// The edit each copy and save still running was asked for at, oldest first, and for each
    /// copy the status it shows once it is on the clipboard and what it then does to the editor.
    /// Each worker reports in the order it was asked. The capture's own copy, made as the editor
    /// opens, is the first.
    copies: VecDeque<(u32, Status, AfterCopy)>,
    saves: VecDeque<u32>,
    /// The client area's size; the image area is below the toolbar.
    width: i32,
    height: i32,
    view: View,
    /// Wheel movement not yet turned into a zoom step, from wheels that move in finer notches.
    unspent_wheel: i32,
    /// Where the pointer last was while the middle button pans.
    panning: Option<POINT>,
    /// Where the pointer is in the client area while it is on it, which the Picker's loupe
    /// follows.
    pointer: Option<POINT>,
    /// Blank until the first copy has finished.
    status: Option<Status>,
    hover: Option<Command>,
    /// The button the left button went down on, while it is held.
    pressed: Option<Command>,
    /// What the left button is doing on the image, while it is held.
    drag: Option<Drag>,
    /// Made the first time a tooltip shows, so opening an editor costs nothing more.
    tooltip: Option<Tooltip>,
    /// The button whose tooltip is showing, if one is.
    tipped: Option<Command>,
    /// Whether a tooltip has shown since the pointer came onto the bar, with no key or press
    /// since, so the next one shows at once instead of waiting.
    warm: bool,
    /// The tool whose flyout is open, if one is. It shows while it is for what is selected.
    flyout: Option<Command>,
    /// The tool whose flyout the window shows as last presented, if any.
    flyout_drawn: Option<Command>,
    /// How many edits had been made once the last press placed a badge, if it did, so a
    /// double-click it began on a text can take the badge back.
    badge_pressed: Option<u32>,
}

/// The timer that shows the first tooltip once the pointer has rested on a button, and how long
/// it waits.
const TIP_TIMER: usize = 1;
const TIP_WAIT_MS: u32 = 500;

/// The untouched editor's one timer: its clock, which runs out after `CLOCK_MS` unless the
/// pointer comes onto it, and then each step of the fade, which lasts `FADE_MS`.
const LIFETIME_TIMER: usize = 2;
const CLOCK_MS: u32 = 8000;
const FADE_MS: u32 = 150;
const FADE_STEP_MS: u32 = 15;

/// The timer that blinks the caret of text being typed, at Windows' rate.
const CARET_TIMER: usize = 3;

/// How opaque the accent is behind text selected while typing.
const HIGHLIGHT_OPACITY: f32 = 1.0 / 3.0;

/// The labels of the buttons that apply or cancel the crop being chosen.
const APPLY: &str = "Apply";
const CANCEL: &str = "Cancel";

/// The command bar at one DPI, in physical pixels, laid out for the client area's width.
struct Toolbar {
    text: bar::Text,
    dpi: u32,
    scale: f32,
    height: i32,
    /// The narrowest client area, which holds every button, the longest status Zilum names
    /// itself and the widest Dimensions readout.
    min_width: i32,
    /// How wide the bar's texts are.
    widths: Widths,
    layout: Layout,
    /// Each button's icon, in the layout's order; the zoom level has none, and Counter's is drawn.
    icons: Vec<Option<IDWriteTextLayout>>,
    /// The digit in Counter's icon.
    counter_digit: IDWriteTextLayout,
    /// The labels of the crop's Apply and Cancel.
    apply: IDWriteTextLayout,
    cancel: IDWriteTextLayout,
    /// The label of each Resize, and how wide the widest is.
    resizes: Vec<(Resize, IDWriteTextLayout)>,
    resize_width: f32,
}

/// The palette's roles as `context` draws them, made again when the theme or the accent changes.
struct Ink {
    /// What the rest is made from: whether it is dark, and the colours a tooltip draws its own
    /// with.
    palette: Palette,
    backdrop: D2D1_COLOR_F,
    /// The command bar's.
    bar: Paint,
    handle: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    /// Behind text selected while typing.
    highlight: ID2D1SolidColorBrush,
    /// Over the image outside a crop being chosen.
    dim: ID2D1SolidColorBrush,
    /// Behind the Picker's loupe, under its pixels, and the hairline around it, as a tooltip's.
    loupe: ID2D1SolidColorBrush,
    loupe_edge: ID2D1SolidColorBrush,
    /// The loupe's cells beside the image, in the backdrop's colour.
    off_image: ID2D1SolidColorBrush,
}

pub fn register(instance: HINSTANCE) -> Result<()> {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("Zilum.Editor"),
            // A double-click inside a crop applies it.
            style: CS_DBLCLKS,
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

/// Makes `colour` the current colour, which the next annotation is placed in.
pub fn set_colour(colour: Colour) {
    CURRENT_COLOUR.set(colour);
}

/// Makes `sizes` the sizes each tool's next annotation is placed at.
pub fn set_sizes(sizes: Sizes) {
    CURRENT_SIZES.set(sizes);
}

/// The style `tool` places its next annotation in: the current colour at `tool`'s own size. Text
/// and a blur have no size, so they take any.
fn next_style(tool: Command) -> Style {
    let sizes = CURRENT_SIZES.get();
    let size = match tool {
        Command::Rect => sizes.rect,
        Command::Pen => sizes.pen,
        Command::Counter => sizes.counter,
        _ => sizes.arrow,
    };
    Style {
        colour: CURRENT_COLOUR.get(),
        size,
    }
}

/// Makes `size` the size `tool` places its next annotation at. A tool with no size keeps none.
fn remember_size(tool: Command, size: Size) {
    let mut sizes = CURRENT_SIZES.get();
    match tool {
        Command::Arrow => sizes.arrow = size,
        Command::Rect => sizes.rect = size,
        Command::Pen => sizes.pen = size,
        Command::Counter => sizes.counter = size,
        _ => return,
    }
    CURRENT_SIZES.set(sizes);
}

/// Makes `size` the em the next text is placed at.
pub fn set_text_size(size: TextSize) {
    CURRENT_TEXT_SIZE.set(size);
}

/// Makes `font`, the em in capture pixels of a text just placed, typed again or resized by its
/// handles on a capture `scale` physical pixels per logical pixel, the em of the next text, in
/// every editor and in the settings file.
fn remember_text_size(font: f32, scale: f32) {
    let size = TextSize::of_font(font, scale);
    if size != CURRENT_TEXT_SIZE.get() {
        CURRENT_TEXT_SIZE.set(size);
        settings::store(Settings {
            text_size: size,
            ..settings::current()
        });
    }
}

/// Makes `head` the head of the next arrow.
pub fn set_head(head: Head) {
    CURRENT_HEAD.set(head);
}

/// Makes `fill` the Fill of the next rectangle.
pub fn set_fill(fill: Fill) {
    CURRENT_FILL.set(fill);
}

/// Makes `level` the level of the next blur.
pub fn set_level(level: Level) {
    CURRENT_LEVEL.set(level);
}

/// Opens an untouched editor showing `image` centred on the primary screen, and closes the
/// untouched editors before it. `previous`, the window the user was in, gets the front back, or
/// keeps it; without one the editor takes it. Returns the new editor, which `WM_COPIED` tells
/// about the clipboard.
pub fn open(image: Arc<Image>, tray: HWND, previous: Option<HWND>) -> Result<HWND> {
    unsafe {
        // The primary screen, whatever screen the capture came from: its top-left is always 0,0.
        let screen = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(screen, &mut info).ok()?;
        let (mut dpi, mut _dpi_y) = (96, 96);
        GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y)?;
        let toolbar = Toolbar::new(dpi)?;
        let borders = frame::borders(STYLE, dpi)?;
        let client = place(info.rcWork, toolbar.height, toolbar.min_width);
        let window = around(client, borders);

        // Not in the taskbar while untouched. Above every window until it has taken the front,
        // and then no longer.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("Zilum.Editor"),
            w!("Zilum"),
            STYLE,
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
            None,
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?;
        // Before it is ever shown, so no capture can contain it.
        let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        let mut editor = match Editor::new(hwnd, tray, image, toolbar).and_then(|mut editor| {
            frame::round(hwnd);
            frame::border(hwnd, editor.ink.palette.dark);
            editor.present().map(|()| editor)
        }) {
            Ok(editor) => editor,
            Err(error) => {
                let _ = DestroyWindow(hwnd);
                return Err(error);
            }
        };
        editor.previous = past_untouched(previous);
        let pointer_part = editor.part(pointer_now(hwnd));
        EDITORS.with(|editors| editors.borrow_mut().push(editor));
        OPENED_AT.set(GetTickCount());
        watch_foreground();

        below_pinned(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Opened under the pointer, the editor needs to know when the pointer leaves it, even with
        // no move on it first. Off it, it hears at once that the pointer is off it.
        let flags = match pointer_part {
            Part::Client => TRACKMOUSEEVENT_FLAGS(0),
            _ => TME_NONCLIENT,
        };
        track_leave(hwnd, flags);
        SetTimer(Some(hwnd), LIFETIME_TIMER, CLOCK_MS, None);
        // Each capture replaces the untouched editors before it.
        close_untouched_but(Some(hwnd), Cause::NewCapture);
        watch_presses();
        // Straight from the overlay or the tray menu, so the window the user was in sees one
        // focus change. Still untouched: only a click, a wheel turn or a key enters it.
        let _ = SetForegroundWindow(hwnd);
        // Above every other window that does not stay above the rest, and below those that do,
        // pinned pictures among them.
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        Ok(hwnd)
    }
}

/// `previous`, the window the user was in when a capture was asked for, or, when that is an
/// untouched editor, which the capture closes, the window that editor took the front from.
fn past_untouched(previous: Option<HWND>) -> Option<HWND> {
    previous
        .and_then(|previous| took_front_from(previous, |editor| !editor.lifetime.entered()))
        .unwrap_or(previous)
}

/// The window the window hotkey captures when `front` is in front: `front` itself, or, when it is
/// an editor, the window that had the front before that editor took it as it opened. A Zilum
/// editor is never captured, and a pinned picture is not looked past.
pub fn looking_past(front: Option<HWND>) -> Option<HWND> {
    front
        .and_then(|front| took_front_from(window::root(front), |editor| !editor.pinned))
        .unwrap_or(front)
}

/// The window the editor `hwnd` took the front from as it opened, if `hwnd` is an editor that
/// `counts`.
fn took_front_from(hwnd: HWND, counts: impl Fn(&Editor) -> bool) -> Option<Option<HWND>> {
    EDITORS.with(|editors| {
        let editors = editors.try_borrow().ok()?;
        let editor = editors.iter().find(|editor| editor.hwnd == hwnd)?;
        counts(editor).then_some(editor.previous)
    })
}

/// Where the client area goes in the work area `work`, under a bar `bar_height` high, in a client
/// area at least `min_width` wide. The image opens at Fit in it, which never enlarges it.
///
/// The client area is the whole editor that shows, bar included: two thirds of the work area
/// each way, whatever the capture, centred in it, and never below the editor's minimum size.
/// The window's borders are invisible resize borders, which lie outside it.
fn place(work: RECT, bar_height: i32, min_width: i32) -> RECT {
    let (work_width, work_height) = (work.right - work.left, work.bottom - work.top);
    let client_width = (work_width * 2 / 3).max(min_width);
    let client_height = (work_height * 2 / 3).max(2 * bar_height);
    let left = work.left + (work_width - client_width) / 2;
    let top = work.top + (work_height - client_height) / 2;
    RECT {
        left,
        top,
        right: left + client_width,
        bottom: top + client_height,
    }
}

/// The pinned picture's window for the image shown with its top-left at `at` on the screen,
/// `extent` physical pixels in size: exactly the image, with the last pixel of an image zoomed
/// between pixels whole.
fn pinned_window(at: Vector2, extent: Vector2) -> RECT {
    let (left, top) = (at.X.round() as i32, at.Y.round() as i32);
    RECT {
        left,
        top,
        right: left + extent.X.ceil() as i32,
        bottom: top + extent.Y.ceil() as i32,
    }
}

/// The editor's client area back around the pinned picture `picture`: the bar `bar_height` high
/// above it, at least `min_width` wide and with an image area at least as tall as the bar, the
/// picture centred in it as the view centres an image that fits. Where the bar would be above
/// the work area, whose top is `work_top`, it moves down onto it, so it can still be dragged.
fn client_around(picture: RECT, bar_height: i32, min_width: i32, work_top: i32) -> RECT {
    let (width, height) = (picture.right - picture.left, picture.bottom - picture.top);
    let (area_width, area_height) = (width.max(min_width), height.max(bar_height));
    let left = picture.left - (area_width - width) / 2;
    let top = (picture.top - bar_height - (area_height - height) / 2).max(work_top);
    RECT {
        left,
        top,
        right: left + area_width,
        bottom: top + bar_height + area_height,
    }
}

fn around(client: RECT, borders: RECT) -> RECT {
    RECT {
        left: client.left + borders.left,
        top: client.top + borders.top,
        right: client.right + borders.right,
        bottom: client.bottom + borders.bottom,
    }
}

impl Editor {
    fn new(hwnd: HWND, tray: HWND, image: Arc<Image>, mut toolbar: Toolbar) -> Result<Editor> {
        let graphics = graphics::get()?;
        unsafe {
            // Windows may have made the window wider than asked, so the client area is the truth.
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client)?;
            // The box text is typed into shows over it, and the command bar can be drawn again
            // alone.
            let swap_chain = graphics.window_swap_chain(
                hwnd,
                (client.right - client.left) as u32,
                (client.bottom - client.top) as u32,
            )?;
            let context = graphics
                .d2d_device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // One unit is one physical pixel, so the image at 100 percent is pixel for pixel.
            context.SetDpi(96.0, 96.0);
            let picture = picture(&context, &image)?;
            let ink = Ink::new(&context)?;
            let pens = Pens::new(&context)?;
            let (width, height) = (client.right - client.left, client.bottom - client.top);
            toolbar.lay_out(width);
            let view = View::new(
                Vector2 {
                    X: image.width as f32,
                    Y: image.height as f32,
                },
                area(width, height, &toolbar),
            );
            Ok(Editor {
                hwnd,
                tray,
                image,
                file: SavedFile::default(),
                choosing_file: false,
                swap_chain,
                context,
                picture,
                scaled: None,
                blurred: None,
                blocks: HashMap::new(),
                ink,
                pens,
                toolbar,
                tool: Tool::Select,
                annotations: Vec::new(),
                selected: None,
                crop: None,
                typing: None,
                pinned: false,
                corner: Vector2::zero(),
                history: History::new(),
                lifetime: Lifetime::opened(within(pointer_now(hwnd), width, height)),
                previous: None,
                output: None,
                edits: 0,
                copies: VecDeque::from([(0, Status::Copied, AfterCopy::Stays)]),
                saves: VecDeque::new(),
                width,
                height,
                view,
                unspent_wheel: 0,
                panning: None,
                pointer: None,
                status: None,
                hover: None,
                pressed: None,
                drag: None,
                tooltip: None,
                tipped: None,
                warm: false,
                flyout: None,
                flyout_drawn: None,
                badge_pressed: None,
            })
        }
    }

    fn present(&mut self) -> Result<()> {
        // The box follows the image as it zooms and pans.
        self.place_typing();
        // A flyout closes once it shows nothing, such as Blur's once no blur is selected, so it
        // does not come back by itself when an undo selects one again.
        self.flyout = self.flyout_under();
        self.flyout_drawn = self.flyout;
        let scale = self.view.scale();
        let full_size = self.view.extent();
        // The annotations placed, with the one being moved or resized where it is going, and the
        // one being drawn, once it has been large enough to place.
        let unplaced = match &self.drag {
            &Some(Drag::Draw {
                kind,
                from,
                to,
                marked: true,
            }) => Some(Annotation::spanned(
                kind,
                from,
                to,
                next_style(Tool::Draw(kind).command()),
                CURRENT_HEAD.get(),
                CURRENT_FILL.get(),
                CURRENT_LEVEL.get(),
            )),
            Some(Drag::Pen {
                path,
                spacing,
                marked: true,
            }) => Some(Annotation::Stroke(
                Stroke::smoothed(path, *spacing),
                next_style(Command::Pen),
            )),
            _ => None,
        };
        let mut annotations: Vec<Annotation> =
            self.annotations.iter().cloned().chain(unplaced).collect();
        if let Some((index, adjusted)) = self.moving() {
            annotations[index] = adjusted;
        }
        // A text being typed again is drawn as it is typed, where it is among the others, and a
        // new one over them all, as each is placed. Nothing is selected meanwhile.
        let typed_at = match self.typing.as_ref().and_then(|typing| typing.editing) {
            Some(index) => {
                annotations.remove(index);
                index
            }
            None => annotations.len(),
        };
        let typed = self.typed();
        let blurred = self.blurred(&annotations)?;
        // At 100 percent and above each capture pixel is a sharp square. Below, at a step, the
        // scaled copy is drawn pixel for pixel; at Fit, which never pans, the picture is scaled
        // as it is drawn, so resizing makes no copy for every size it passes through.
        let (picture, picture_size, mode) = if scale >= 1.0 {
            (blurred, full_size, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR)
        } else if self.view.at_fit() {
            self.scaled = None;
            (
                blurred,
                full_size,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            )
        } else {
            let scaled = self.scaled(&blurred, scale)?;
            let size = unsafe { scaled.GetSize() };
            let size = Vector2 {
                X: size.width,
                Y: size.height,
            };
            (scaled, size, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR)
        };
        let target = self.target()?;
        unsafe {
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            self.context.Clear(Some(&self.ink.backdrop));

            // The image first, so the toolbar covers whatever of it is panned above the image
            // area. The view keeps the image at whole pixels.
            let at = self.view.to_area(Vector2::zero());
            let left = at.X;
            let top = self.image_top() as f32 + at.Y;
            self.context.DrawBitmap(
                &picture,
                Some(&D2D_RECT_F {
                    left,
                    top,
                    right: left + picture_size.X,
                    bottom: top + picture_size.Y,
                }),
                1.0,
                mode,
                None,
                None,
            );
            // The arrows and rectangles over it at the capture's scale, so they zoom with it.
            self.context.SetTransform(
                &(Matrix3x2::scale(scale, scale) * Matrix3x2::translation(left, top)),
            );
            let (below, above) = annotations.split_at(typed_at);
            let drawn_annotations =
                annotation::draw(&self.context, &self.pens, below, self.image.scale)
                    .and(
                        typed
                            .as_ref()
                            .map_or(Ok(()), |(text, colour, highlight, _)| {
                                annotation::draw_typed(
                                    &self.context,
                                    &self.pens,
                                    text,
                                    *colour,
                                    self.image.scale,
                                    highlight,
                                    &self.ink.highlight,
                                )
                            }),
                    )
                    .and(annotation::draw(
                        &self.context,
                        &self.pens,
                        above,
                        self.image.scale,
                    ));
            // A pinned picture is the image and its annotations alone.
            let crop = self.shown_crop().filter(|_| !self.pinned);
            if let Some(crop) = crop {
                self.dim_outside(crop);
            }
            self.context.SetTransform(&Matrix3x2::identity());
            let on_screen = |point: Vector2| {
                self.view.to_area(point)
                    + Vector2 {
                        X: 0.0,
                        Y: self.image_top() as f32,
                    }
            };
            // The handles of the crop being chosen, or else the handles and outline the selected
            // annotation has, in screen pixels, so they are the same size at every zoom.
            let selected = self
                .selected
                .filter(|_| !self.pinned)
                .and_then(|index| annotations.get(index));
            let handles: Vec<Vector2> = match crop {
                Some(crop) => crop.handles().map(|(_, _, at)| at).collect(),
                None => selected
                    .map(|selected| selected.handles().into_iter().map(|(_, at)| at).collect())
                    .unwrap_or_default(),
            };
            annotation::draw_handles(
                &self.context,
                &self.ink.handle,
                &self.ink.accent,
                handles.into_iter().map(on_screen),
            );
            if crop.is_none()
                && let Some(outlined) =
                    selected.and_then(|selected| selected.outlined(self.image.scale))
            {
                annotation::draw_outline(
                    &self.context,
                    &self.ink.accent,
                    on_screen(outlined.min),
                    on_screen(outlined.max),
                );
            }
            // The caret of text being typed, over everything it is typed among, one logical
            // pixel wide at the capture's scale, on whole screen pixels so it is crisp.
            if let Some((text, colour, _, Some(caret))) = &typed {
                let at = on_screen(text.at + caret.at);
                let width = (self.image.scale * scale).round().max(1.0);
                let left = (at.X - width / 2.0).round();
                self.context.FillRectangle(
                    &D2D_RECT_F {
                        left,
                        top: at.Y.round(),
                        right: left + width,
                        bottom: (at.Y + caret.height * scale).round(),
                    },
                    self.pens.brush(*colour),
                );
            }
            // The box an input method composes in is outlined as a selected text is.
            if let Some(typing) = self.typing.as_ref().filter(|typing| typing.composing) {
                let placed = typing.entry.placed();
                annotation::draw_outline(
                    &self.context,
                    &self.ink.accent,
                    Vector2 {
                        X: placed.left as f32,
                        Y: placed.top as f32,
                    },
                    Vector2 {
                        X: placed.right as f32,
                        Y: placed.bottom as f32,
                    },
                );
            }

            let drawn_loupe = self.draw_loupe();
            let drawn_bar = if self.pinned { Ok(()) } else { self.draw_bar() };
            let drawn_flyout = self.draw_flyout();
            let drawn = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn_annotations?;
            drawn_loupe?;
            drawn_bar?;
            drawn_flyout?;
            drawn?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    /// Draws only the command bar again, and the flyout if one shows, after a change nothing else
    /// shows, such as a button the pointer is on or pressing. The rest of the window keeps what
    /// was presented last; a flyout that has closed since, or given way to another, needs the
    /// image drawn where it was, so the whole window is drawn instead.
    fn present_bar(&mut self) -> Result<()> {
        // A pinned picture has no bar; the editor shows it as it is by then when it comes back.
        if self.pinned {
            return Ok(());
        }
        let flyout = self.flyout_under();
        if self.flyout_drawn.is_some() && self.flyout_drawn != flyout {
            return self.present();
        }
        self.flyout_drawn = flyout;
        let target = self.target()?;
        unsafe {
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            let drawn_bar = self.draw_bar();
            let drawn_flyout = self.draw_flyout();
            let drawn = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn_bar?;
            drawn_flyout?;
            drawn?;
            let bar = RECT {
                left: 0,
                top: 0,
                right: self.width,
                bottom: self.toolbar.height,
            };
            let panel = self.flyout_panel().unwrap_or_default();
            // Its edges are on whole pixels, inside the window however small it is.
            let panel = RECT {
                left: panel.left as i32,
                top: panel.top as i32,
                right: (panel.right as i32).min(self.width),
                bottom: (panel.bottom as i32).min(self.height),
            };
            let mut dirty = [bar, panel];
            self.swap_chain
                .Present1(
                    1,
                    DXGI_PRESENT(0),
                    &DXGI_PRESENT_PARAMETERS {
                        DirtyRectsCount: if flyout.is_some() { 2 } else { 1 },
                        pDirtyRects: dirty.as_mut_ptr(),
                        ..Default::default()
                    },
                )
                .ok()
        }
    }

    /// The swap chain's buffer as a bitmap `context` draws into.
    fn target(&self) -> Result<ID2D1Bitmap1> {
        unsafe {
            let buffer: IDXGISurface = self.swap_chain.GetBuffer(0)?;
            self.context.CreateBitmapFromDxgiSurface(
                &buffer,
                Some(&bitmap_properties(
                    D2D1_ALPHA_MODE_IGNORE,
                    D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                )),
            )
        }
    }

    /// Draws the command bar: its buttons with their tints, the dividers, the contextual group,
    /// the status, the Dimensions readout and the zoom level. Only between `BeginDraw` and
    /// `EndDraw`.
    fn draw_bar(&self) -> Result<()> {
        let toolbar = &self.toolbar;
        let paint = &self.ink.bar;
        let radius = bar::RADIUS * toolbar.scale;
        paint.bar(&D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: self.width as f32,
            bottom: toolbar.height as f32,
        });
        for divider in &toolbar.layout.dividers {
            paint.divider(divider);
        }
        for (&(command, ref rect), icon) in toolbar.layout.buttons.iter().zip(&toolbar.icons) {
            // Close fills the bar's corner, as a caption button does.
            if command == Command::Close
                && let Some(icon) = icon
            {
                paint.close_button(rect, self.tint(command), icon)?;
                continue;
            }
            if let Some(tint) = self.tint(command) {
                paint.tint(rect, tint, radius);
            }
            if self.current(command) {
                paint.mark(rect, toolbar.scale);
            }
            if command == Command::Counter {
                paint.counter(
                    rect,
                    &toolbar.counter_digit,
                    self.tint(command),
                    toolbar.scale,
                )?;
            } else if let Some(icon) = icon {
                paint.centred(icon, rect, Tone::Text)?;
            }
        }
        self.draw_group()?;
        let zoom = toolbar.text.label(&self.view.label())?;
        let tone = if self.view.can_toggle() {
            Tone::Secondary
        } else {
            Tone::Disabled
        };
        paint.centred(&zoom, &toolbar.layout.rect(Command::Zoom), tone)?;
        let dimensions = toolbar.text.label(&command_bar::dimensions(&self.image))?;
        let area = toolbar.layout.dimensions(bar::size(&dimensions)?.X);
        if let Some(tint) = self.tint(Command::Dimensions) {
            paint.tint(&area, tint, radius);
        }
        paint.centred(&dimensions, &area, Tone::Secondary)?;
        if let (Some(status), Some((label, at))) = (&self.status, self.status_label()?) {
            let tone = match status {
                Status::Copied | Status::Picked { .. } => Tone::Secondary,
                Status::NotCopied | Status::NotSaved => Tone::Failed,
            };
            paint.text(&label, at, tone);
        }
        Ok(())
    }

    /// Draws the contextual group as it is now: the crop's readout with Apply and Cancel, if a
    /// crop is being chosen. Only between `BeginDraw` and `EndDraw`.
    fn draw_group(&self) -> Result<()> {
        let toolbar = &self.toolbar;
        let paint = &self.ink.bar;
        let radius = bar::RADIUS * toolbar.scale;
        let group = self.group();
        let parts = toolbar.layout.parts(group);
        for divider in &parts.dividers {
            paint.divider(divider);
        }
        for (command, rect) in &parts.buttons {
            let tint = self.tint(*command);
            if *command == Command::Apply {
                paint.accent_button(rect, &toolbar.apply, tint, radius)?;
                continue;
            }
            if let Some(tint) = tint {
                paint.tint(rect, tint, radius);
            }
            if *command == Command::Cancel {
                paint.centred(&toolbar.cancel, rect, Tone::Text)?;
            }
        }
        if let (Some(area), Some(readout)) = (parts.readout, group.readout()) {
            let label = toolbar.text.label(&readout)?;
            let height = bar::size(&label)?.Y;
            let at = Vector2 {
                X: area.left,
                Y: ((toolbar.height as f32 - height) / 2.0).round(),
            };
            paint.text(&label, at, Tone::Text);
        }
        Ok(())
    }

    /// Draws the flyout that shows, if one does: Blur's panel and Level, the knob where a drag of
    /// it has taken it, a style flyout's panel and row, the colour it shows ringed, the Size
    /// slider's knob where a drag of it has taken it and the Head or Fill it shows pressed, or the
    /// Dimensions flyout's panel and Resizes. Only between `BeginDraw` and `EndDraw`.
    fn draw_flyout(&self) -> Result<()> {
        let Some((under, shown)) = self.flyout_shown() else {
            return Ok(());
        };
        let toolbar = &self.toolbar;
        let paint = &self.ink.bar;
        let radius = bar::RADIUS * toolbar.scale;
        let Some(panel) = self.flyout_panel() else {
            return Ok(());
        };
        paint.panel(&panel, radius);
        if let Some((slider, area, along)) = self.shown_slider() {
            let at = flyout::knob(area, along, toolbar.scale);
            let hover = self.hover == Some(slider) || self.sliding().is_some();
            paint.slider(&area, flyout::KNOB, at, hover, toolbar.scale);
        }
        let colour = match shown {
            Shown::Level(_) => return Ok(()),
            Shown::Style { colour, .. } => Some(colour),
            Shown::Resizes => None,
        };
        let Some(row) = self.row(under) else {
            return Ok(());
        };
        for divider in &row.dividers {
            paint.divider(divider);
        }
        for (command, rect) in &row.buttons {
            if let Some(tint) = self.tint(*command) {
                paint.tint(rect, tint, radius);
            }
            match command {
                Command::Colour(swatch) => paint.swatch(
                    rect,
                    self.pens.brush(*swatch),
                    Some(*swatch) == colour,
                    toolbar.scale,
                ),
                Command::Resize(resize) => {
                    if let Some((_, label)) = toolbar.resizes.iter().find(|(of, _)| of == resize) {
                        paint.centred(label, rect, Tone::Text)?;
                    }
                }
                Command::Head(head) => {
                    paint.arrow(rect, head.at_tip(), head.at_tail(), toolbar.scale)?
                }
                Command::Fill(fill) => paint.rectangle(rect, *fill, toolbar.scale),
                _ => {}
            }
        }
        Ok(())
    }

    /// The flyout that shows, if one does, as the tool it hangs under and what it shows: the one
    /// open, while it is for the annotation selected. A pinned picture shows none.
    fn flyout_shown(&self) -> Option<(Command, Shown)> {
        let under = self.flyout.filter(|_| !self.pinned)?;
        let shown = self.with_selected(|selected| flyout::shown(under, selected))?;
        Some((under, shown))
    }

    /// The tool whose flyout shows, if one does.
    fn flyout_under(&self) -> Option<Command> {
        self.flyout_shown().map(|(under, _)| under)
    }

    /// Whether a flyout shows.
    fn flyout_shows(&self) -> bool {
        self.flyout_shown().is_some()
    }

    /// Where the panel of the flyout that shows is, if one does.
    fn flyout_panel(&self) -> Option<D2D_RECT_F> {
        let layout = &self.toolbar.layout;
        match self.flyout_shown()? {
            (_, Shown::Level(_)) => Some(layout.flyout.panel),
            (under, _) => self.row(under).map(|row| row.panel),
        }
    }

    /// The row of the flyout under `under`, if it has one: a style flyout's, or the Dimensions
    /// flyout's under the readout as it reads now.
    fn row(&self, under: Command) -> Option<StyleFlyout> {
        if under != Command::Dimensions {
            return self.toolbar.layout.style_flyout(under).cloned();
        }
        let toolbar = &self.toolbar;
        Some(StyleFlyout::resizes(
            self.readout_area()?,
            toolbar.scale,
            toolbar.resize_width,
        ))
    }

    /// Where the Dimensions readout is, as wide as it reads now.
    fn readout_area(&self) -> Option<D2D_RECT_F> {
        let toolbar = &self.toolbar;
        let width = toolbar
            .text
            .width(&command_bar::dimensions(&self.image))
            .ok()?;
        Some(toolbar.layout.dimensions(width))
    }

    /// After a press that cannot be on a flyout, such as a right or middle press or one that moves
    /// the window: the Dimensions flyout closes, and any other is left to the selection.
    fn press_outside(&mut self) {
        self.flyout_after(flyout::Event::Press { open: self.flyout });
        self.refresh_flyout();
    }

    /// Opens the Dimensions flyout, or closes it if it is open, after a click on the readout.
    fn toggle_dimensions(&mut self) {
        self.flyout_after(flyout::Event::Readout { open: self.flyout });
        let _ = self.present_bar();
    }

    /// The tool that places the annotation selected, if one is.
    fn selected_by(&self) -> Option<Command> {
        self.selected
            .map(|index| flyout::placed_by(&self.annotations[index]))
    }

    /// Opens or closes the flyouts after `event`. Closing the flyout whose slider is being
    /// dragged abandons the drag. The window shows the change once `refresh_flyout` or a present
    /// draws it.
    fn flyout_after(&mut self, event: flyout::Event) {
        self.flyout = flyout::open_after(event);
        if self
            .sliding()
            .is_some_and(|under| self.flyout != Some(under))
        {
            self.drag = None;
        }
    }

    /// The tool whose flyout's slider, Level or Size, is being dragged, if one is.
    fn sliding(&self) -> Option<Command> {
        match self.drag {
            Some(Drag::Level { .. }) => Some(Command::Blur),
            Some(Drag::Size { index, .. }) => Some(flyout::placed_by(&self.annotations[index])),
            _ => None,
        }
    }

    /// Opens the flyout under the tool that places the annotation just selected, by drawing it or
    /// a press under Select, or closes it for nothing selected.
    fn selection_changed(&mut self) {
        self.flyout_after(flyout::Event::Selection {
            selected: self.selected_by(),
        });
    }

    /// Closes the flyout that shows and deselects what it was for, as Esc does. True if one
    /// showed.
    fn escape_flyout(&mut self) -> bool {
        if !self.flyout_shows() {
            return false;
        }
        self.flyout_after(flyout::Event::Escape);
        self.selected = None;
        let _ = self.present();
        true
    }

    /// Whether `at` is on the flyout that shows.
    fn on_flyout(&self, at: POINT) -> bool {
        self.flyout_panel()
            .is_some_and(|panel| contains(&panel, at))
    }

    /// Draws the flyout that shows, or the image where the one before was, if that changed since
    /// the window last showed it.
    fn refresh_flyout(&mut self) {
        if self.flyout_under() != self.flyout_drawn {
            let _ = self.present_bar();
        }
    }

    /// What the contextual group holds: the readout of the crop being chosen, if one is.
    fn group(&self) -> Group {
        // The crop lies on pixel corners.
        Group::new(self.shown_crop().map(|crop| {
            let size = crop.max - crop.min;
            (size.X as u32, size.Y as u32)
        }))
    }

    /// What `f` makes of the annotation selected or being typed again, if any, where a drag
    /// moving, resizing or giving it a level has taken it.
    fn with_selected<T>(&self, f: impl FnOnce(Option<&Annotation>) -> T) -> T {
        let typed_again = self.typing.as_ref().and_then(|typing| typing.editing);
        let moving = self.moving();
        let selected = self
            .selected
            .or(typed_again)
            .and_then(|index| match &moving {
                Some((moved, annotation)) if *moved == index => Some(annotation),
                _ => self.annotations.get(index),
            });
        f(selected)
    }

    /// Darkens the image outside `crop`, through `context`'s transform from capture pixels. Only
    /// between `BeginDraw` and `EndDraw`.
    fn dim_outside(&self, crop: Bounds) {
        let Vector2 {
            X: width,
            Y: height,
        } = self.image_size();
        let (min, max) = (crop.min, crop.max);
        // Aliased, so where the rectangles meet between pixels, below 100 percent, no seam
        // shows.
        unsafe { self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED) };
        for (left, top, right, bottom) in [
            (0.0, 0.0, width, min.Y),
            (0.0, max.Y, width, height),
            (0.0, min.Y, min.X, max.Y),
            (max.X, min.Y, width, max.Y),
        ] {
            unsafe {
                self.context.FillRectangle(
                    &D2D_RECT_F {
                        left,
                        top,
                        right,
                        bottom,
                    },
                    &self.ink.dim,
                )
            };
        }
        unsafe {
            self.context
                .SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE)
        };
    }

    /// Whether `button` reads chosen: it is the active tool, the Head the arrow selected has or
    /// the Fill the rectangle selected has, or the readout while its flyout shows.
    fn current(&self, button: Command) -> bool {
        match button {
            Command::Dimensions => self.flyout_under() == Some(Command::Dimensions),
            Command::Head(head) => matches!(
                self.flyout_shown(),
                Some((_, Shown::Style { head: Some(shown), .. })) if shown == head
            ),
            Command::Fill(fill) => matches!(
                self.flyout_shown(),
                Some((_, Shown::Style { fill: Some(shown), .. })) if shown == fill
            ),
            command => Tool::of(command) == Some(self.tool),
        }
    }

    /// What fills behind `button`: the pressed tint while the left button is down on it and the
    /// pointer still is, the chosen tint while it is the active tool, and the hover tint while
    /// the pointer is on it. Nothing while it is the zoom button, dimmed.
    fn tint(&self, button: Command) -> Option<Tint> {
        if button == Command::Zoom && !self.view.can_toggle() {
            return None;
        }
        match (self.pressed, self.hover) {
            (Some(pressed), Some(hover)) if pressed == button && hover == button => {
                Some(Tint::Pressed)
            }
            _ if self.current(button) => Some(Tint::Chosen),
            (None, Some(hover)) if hover == button => Some(Tint::Hover),
            _ => None,
        }
    }

    /// The status laid out against the Dimensions readout, trimmed to the room the bar has for
    /// it: the text and where it goes.
    fn status_label(&self) -> Result<Option<(IDWriteTextLayout, Vector2)>> {
        let Some(status) = &self.status else {
            return Ok(None);
        };
        let toolbar = &self.toolbar;
        let readout_width = toolbar.text.width(&command_bar::dimensions(&self.image))?;
        let room = toolbar.layout.status(self.group(), readout_width);
        let padding = (command_bar::STATUS_PADDING * toolbar.scale).round();
        let text = bar::trim(
            &status.text(),
            room.right - room.left - 2.0 * padding,
            |text| toolbar.text.width(text).unwrap_or(f32::MAX),
        );
        let label = toolbar.text.label(&text)?;
        let size = bar::size(&label)?;
        let at = Vector2 {
            X: (room.right - padding - size.X).round(),
            Y: ((toolbar.height as f32 - size.Y) / 2.0).round(),
        };
        Ok(Some((label, at)))
    }

    /// What the pointer at `at` is on: a button, the Dimensions readout or a flyout's part.
    fn hit(&self, at: POINT) -> Option<Command> {
        if self.pinned {
            return None;
        }
        // Level, or a swatch, the Size slider, a Head, a Fill or a Resize, in the flyout over
        // the image.
        match self.flyout_shown() {
            Some((_, Shown::Level(_))) if contains(&self.toolbar.layout.flyout.level, at) => {
                return Some(Command::Level);
            }
            Some((under, Shown::Style { .. } | Shown::Resizes)) => {
                let row = self.row(under);
                if let Some(command) = row.and_then(|row| row.hit(at.x as f32, at.y as f32)) {
                    return Some(command);
                }
            }
            _ => {}
        }
        let group = self.group();
        if let Some(command) = self.toolbar.layout.hit(at.x as f32, at.y as f32, group) {
            return Some(command);
        }
        self.readout_area()
            .is_some_and(|area| contains(&area, at))
            .then_some(Command::Dimensions)
    }

    /// What the pointer at `at` is on as Windows sees it: the bar moves the window except where
    /// it can be clicked, and the edges and the space above the buttons resize it. A pinned
    /// picture moves wherever it is dragged.
    fn part(&self, at: POINT) -> Part {
        if self.pinned {
            return Part::Caption;
        }
        self.frame().part(at, self.hit(at).is_some())
    }

    fn frame(&self) -> Frame {
        Frame {
            width: self.width,
            height: self.height,
            bar: self.toolbar.height,
            // The space above the buttons.
            grip: self.toolbar.layout.rect(Command::Select).top as i32,
            corner: (CORNER_REACH * self.toolbar.scale).round() as i32,
        }
    }

    /// Updates the highlighted button and the tooltip for the pointer at `at`. True if the
    /// pointer is on a button.
    fn hover_at(&mut self, at: POINT) -> bool {
        // Close's top edge resizes the window, so it is not on Close there.
        let hover = self
            .hit(at)
            .filter(|_| self.frame().part(at, true) == Part::Client);
        // A flyout's buttons have their tooltips as the bar's do.
        let on_bar = (at.x >= 0 && at.x < self.width && at.y >= 0 && at.y < self.toolbar.height)
            || self.on_flyout(at);
        if !on_bar {
            self.end_tips();
        }
        if hover != self.hover {
            self.hover = hover;
            let _ = self.present_bar();
            if on_bar && !self.holding() {
                self.hover_tip(hover);
            }
        }
        hover.is_some()
    }

    /// Shows the tooltip of `button`, the one the pointer has moved onto, at once while one has
    /// shown since the pointer came onto the bar, and otherwise once it has rested there. Between
    /// buttons none shows.
    fn hover_tip(&mut self, button: Option<Command>) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIP_TIMER);
        }
        match button {
            Some(button) if self.warm => self.show_tip(button),
            Some(_) => {
                self.hide_tip();
                unsafe { SetTimer(Some(self.hwnd), TIP_TIMER, TIP_WAIT_MS, None) };
            }
            None => self.hide_tip(),
        }
    }

    /// The pointer has rested on a button: shows its tooltip, and the next ones at once.
    fn rested(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIP_TIMER);
        }
        if let Some(button) = self.hover
            && !self.holding()
        {
            self.warm = true;
            self.show_tip(button);
        }
    }

    /// After the pointer leaves the bar, a key or a press: hides any tooltip, and the next one
    /// waits again.
    fn end_tips(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIP_TIMER);
        }
        self.warm = false;
        self.hide_tip();
    }

    fn hide_tip(&mut self) {
        if self.tipped.take().is_some()
            && let Some(tooltip) = &self.tooltip
        {
            tooltip.hide();
        }
    }

    /// Shows the tooltip of `button` below it.
    fn show_tip(&mut self, button: Command) {
        if self.tipped == Some(button) {
            return;
        }
        let Some(area) = self.button_area(button) else {
            return self.hide_tip();
        };
        let text = button.tip();
        let mut corners = [
            POINT {
                x: area.left as i32,
                y: area.top as i32,
            },
            POINT {
                x: area.right as i32,
                y: area.bottom as i32,
            },
        ];
        for corner in &mut corners {
            let _ = unsafe { ClientToScreen(self.hwnd, corner) };
        }
        let on_screen = RECT {
            left: corners[0].x,
            top: corners[0].y,
            right: corners[1].x,
            bottom: corners[1].y,
        };
        let shown = (|| {
            if self.tooltip.is_none() {
                self.tooltip = Some(Tooltip::new(self.hwnd)?);
            }
            let layout = self.toolbar.text.tip(text)?;
            if let Some(tooltip) = &self.tooltip {
                tooltip.show(&layout, on_screen, self.toolbar.scale, &self.ink.palette)?;
            }
            Ok::<_, Error>(())
        })();
        match shown {
            Ok(()) => self.tipped = Some(button),
            Err(_) => self.hide_tip(),
        }
    }

    /// Where `button` is in the client area, if it is on the bar as it is now.
    fn button_area(&self, button: Command) -> Option<D2D_RECT_F> {
        if button == Command::Dimensions {
            return self.readout_area();
        }
        let in_row = match self.flyout_shown() {
            Some((under, Shown::Style { .. } | Shown::Resizes)) => {
                self.row(under).and_then(|row| {
                    row.buttons
                        .iter()
                        .find(|(command, _)| *command == button)
                        .map(|(_, rect)| *rect)
                })
            }
            _ => None,
        };
        in_row.or_else(|| self.toolbar.layout.shown_rect(button, self.group()))
    }

    fn set_status(&mut self, status: Status) {
        if self.status.as_ref() != Some(&status) {
            self.status = Some(status);
            let _ = self.present_bar();
        }
    }

    /// The capture with every blur among `annotations` baked in, made again only when the blurs
    /// change. Each blur's blocks come from the capture itself, so a later blur over an earlier one
    /// hides what the capture has there.
    fn blurred(&mut self, annotations: &[Annotation]) -> Result<ID2D1Bitmap1> {
        let (width, height, scale) = (self.image.width, self.image.height, self.image.scale);
        let blurs: Vec<Blur> = annotations
            .iter()
            .filter_map(|annotation| match annotation {
                Annotation::Blur(bounds, level) => Some((
                    bounds.pixels(width, height)?,
                    bounds.first_pixel(),
                    level.block(scale),
                )),
                _ => None,
            })
            .collect();
        if blurs.is_empty() {
            self.blurred = None;
            self.blocks.clear();
            return Ok(self.picture.clone());
        }
        if let Some((made_with, blurred)) = &self.blurred
            && *made_with == blurs
        {
            return Ok(blurred.clone());
        }
        self.blocks.retain(|blur, _| blurs.contains(blur));
        unsafe {
            let blurred = self.context.CreateBitmap(
                D2D_SIZE_U { width, height },
                None,
                0,
                &bitmap_properties(D2D1_ALPHA_MODE_PREMULTIPLIED, Default::default()),
            )?;
            // Copied rather than drawn, so every pixel outside the blurs is exactly the capture's,
            // and the blocks replace what is beneath them even where it is translucent.
            blurred.CopyFromBitmap(None, &self.picture, None)?;
            for &(region, origin, block) in &blurs {
                let blocks = self
                    .blocks
                    .entry((region, origin, block))
                    .or_insert_with(|| {
                        annotation::blur_blocks(&self.image.pixels, width, region, origin, block)
                    });
                blurred.CopyFromMemory(
                    Some(&D2D_RECT_U {
                        left: region.left,
                        top: region.top,
                        right: region.right,
                        bottom: region.bottom,
                    }),
                    blocks.as_ptr() as *const _,
                    region.width() * 4,
                )?;
            }
            self.blurred = Some((blurs, blurred.clone()));
            Ok(blurred)
        }
    }

    /// `picture` scaled to `scale` with high quality, made once for each picture and scale.
    fn scaled(&mut self, picture: &ID2D1Bitmap1, scale: f32) -> Result<ID2D1Bitmap1> {
        if let Some((made_from, made_for, scaled)) = &self.scaled
            && made_from == picture
            && *made_for == scale
        {
            return Ok(scaled.clone());
        }
        let width = self.image.width as f32 * scale;
        let height = self.image.height as f32 * scale;
        unsafe {
            let scaled = self.context.CreateBitmap(
                D2D_SIZE_U {
                    width: (width.ceil() as u32).max(1),
                    height: (height.ceil() as u32).max(1),
                },
                None,
                0,
                &bitmap_properties(D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BITMAP_OPTIONS_TARGET),
            )?;
            self.context.SetTarget(&scaled);
            self.context.BeginDraw();
            self.context.Clear(Some(&D2D1_COLOR_F::default()));
            // The high quality mode filters the image first as it scales down, so it does not
            // alias.
            self.context.DrawBitmap(
                picture,
                Some(&D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: width,
                    bottom: height,
                }),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
            let drawn = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn?;
            self.scaled = Some((picture.clone(), scale, scaled.clone()));
            Ok(scaled)
        }
    }

    /// Follows the client area to its new size: the image is fitted again at Fit, and otherwise
    /// keeps its scale.
    fn resized(&mut self) -> Result<()> {
        let mut client = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut client)? };
        let (width, height) = (client.right - client.left, client.bottom - client.top);
        // Minimised, or not yet laid out.
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        if (width, height) != (self.width, self.height) {
            unsafe {
                self.swap_chain.ResizeBuffers(
                    0,
                    width as u32,
                    height as u32,
                    DXGI_FORMAT_UNKNOWN,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )?;
            }
            (self.width, self.height) = (width, height);
        }
        self.toolbar.lay_out(width);
        let area = if self.pinned {
            Vector2 {
                X: width as f32,
                Y: height as f32,
            }
        } else {
            area(width, height, &self.toolbar)
        };
        self.view.resize(area);
        self.present()
    }

    /// The smallest window, which keeps the whole toolbar visible over an image area as tall
    /// as the toolbar.
    fn min_size(&self) -> Result<POINT> {
        let borders = frame::borders(STYLE, self.toolbar.dpi)?;
        Ok(POINT {
            x: self.toolbar.min_width + borders.right - borders.left,
            y: 2 * self.toolbar.height + borders.bottom - borders.top,
        })
    }

    /// Zooms one step for each notch the wheel turned, about the pointer at `at` in the client
    /// area. A pinned picture returns its window, moved and resized to the image as it now
    /// shows, if it zoomed.
    fn wheel(&mut self, delta: i32, at: POINT) -> Option<RECT> {
        // Turning back starts afresh.
        if self.unspent_wheel.signum() == -delta.signum() {
            self.unspent_wheel = 0;
        }
        self.unspent_wheel += delta;
        let notch = WHEEL_DELTA as i32;
        let notches = self.unspent_wheel / notch;
        self.unspent_wheel %= notch;
        if self.pinned {
            return self.zoom_pinned(notches, at);
        }
        let at = self.area_point(at);
        let mut changed = false;
        for _ in 0..notches.abs() {
            changed |= if notches > 0 {
                self.view.zoom_in(at)
            } else {
                self.view.zoom_out(at)
            };
        }
        if changed {
            let _ = self.present();
        }
        None
    }

    /// Zooms the pinned picture `notches` steps in, or out if negative, about the pointer at `at`
    /// on it, never larger than its screen. Returns its window moved and resized to the image as
    /// it now shows, if it zoomed.
    fn zoom_pinned(&mut self, notches: i32, at: POINT) -> Option<RECT> {
        let mut window = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut window).ok()? };
        let shown = Vector2 {
            X: window.left as f32,
            Y: window.top as f32,
        };
        // From the exact corner, so zooming in and out again never drifts, unless the picture
        // has been dragged since.
        let corner = if self.corner.X.round() == shown.X && self.corner.Y.round() == shown.Y {
            self.corner
        } else {
            shown
        };
        let room = screen_size(&screen_of(self.hwnd));
        let mut at = Vector2 {
            X: at.x as f32,
            Y: at.y as f32,
        } + (shown - corner);
        let mut moved: Option<Vector2> = None;
        for _ in 0..notches.abs() {
            let Some(by) = self.view.zoom_pinned(notches > 0, at, room) else {
                break;
            };
            moved = Some(moved.unwrap_or_default() + by);
            at = at - by;
        }
        self.corner = corner + moved?;
        Some(pinned_window(self.corner, self.view.extent()))
    }

    /// Makes the editor a pinned picture: the image as it shows, where it shows, with its
    /// annotations and nothing else, never larger than its screen. Text being typed is kept, and
    /// what is selected or being cropped waits for the editor to come back. Returns the picture's
    /// window, or None if it already is one.
    fn pin(&mut self) -> Option<RECT> {
        if self.pinned {
            return None;
        }
        self.end_tips();
        self.finish_typing(None);
        self.pointer = None;
        let mut corner = POINT {
            x: 0,
            y: self.toolbar.height,
        };
        let _ = unsafe { ClientToScreen(self.hwnd, &mut corner) };
        let mut at = Vector2 {
            X: corner.x as f32,
            Y: corner.y as f32,
        } + self.view.to_area(Vector2::zero());
        self.view.pin();
        // An image zoomed larger than its screen shrinks about its centre until it fits.
        let room = screen_size(&screen_of(self.hwnd));
        while self.view.extent().X > room.X || self.view.extent().Y > room.Y {
            let centre = self.view.extent() / 2.0;
            match self.view.zoom_pinned(false, centre, room) {
                Some(by) => at = at + by,
                None => break,
            }
        }
        self.pinned = true;
        self.corner = at;
        Some(pinned_window(at, self.view.extent()))
    }

    /// Makes the pinned picture the editor again, around it. Returns the editor's window, or None
    /// if it is not pinned.
    fn unpin(&mut self) -> Option<RECT> {
        if !self.pinned {
            return None;
        }
        // A pinned picture has no borders: its window is the image.
        let mut picture = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut picture).ok()? };
        let work = screen_of(self.hwnd).rcWork;
        let toolbar = &self.toolbar;
        let client = client_around(picture, toolbar.height, toolbar.min_width, work.top);
        let borders = frame::borders(STYLE, toolbar.dpi).ok()?;
        self.pinned = false;
        Some(around(client, borders))
    }

    /// Where the image area starts down the client area: below the bar, or at the top of a pinned
    /// picture.
    fn image_top(&self) -> i32 {
        if self.pinned { 0 } else { self.toolbar.height }
    }

    /// Follows the pointer, now at `at`, with a pan, the annotation being drawn, or the one being
    /// moved or resized.
    fn pointer_moved(&mut self, at: POINT) {
        // The loupe follows the pointer, and goes as it leaves the image.
        let loupe_before = self.loupe_pixel();
        self.pointer = Some(at);
        let mut changed = loupe_before.is_some() || self.loupe_pixel().is_some();
        // Both buttons panning at once move the image once.
        let mut pan_from = None;
        if let Some(last) = self.panning {
            self.panning = Some(at);
            pan_from = Some(last);
        }
        match self.drag {
            Some(Drag::Pan(last)) => {
                self.drag = Some(Drag::Pan(at));
                pan_from.get_or_insert(last);
            }
            Some(Drag::Draw {
                kind,
                from,
                to,
                marked,
            }) => {
                let moved = self.capture_point(at);
                if moved != to {
                    self.drag = Some(Drag::Draw {
                        kind,
                        from,
                        to: moved,
                        marked: marked
                            || Annotation::from_drag(
                                kind,
                                from,
                                moved,
                                next_style(Tool::Draw(kind).command()),
                                CURRENT_HEAD.get(),
                                CURRENT_FILL.get(),
                                CURRENT_LEVEL.get(),
                            )
                            .is_some(),
                    });
                    changed = true;
                }
            }
            Some(Drag::Pen { .. }) => changed = self.extend_stroke(at),
            Some(Drag::Adjust {
                index,
                handle,
                from,
                to,
            }) => {
                let moved = self.capture_point(at);
                if moved != to {
                    self.drag = Some(Drag::Adjust {
                        index,
                        handle,
                        from,
                        to: moved,
                    });
                    changed = true;
                }
            }
            Some(Drag::Crop { from, to, marked }) => {
                let moved = self.capture_corner(at);
                if moved != to {
                    self.drag = Some(Drag::Crop {
                        from,
                        to: moved,
                        marked: marked || Bounds::from_drag(from, moved).is_some(),
                    });
                    changed = true;
                }
            }
            Some(Drag::AdjustCrop {
                across,
                down,
                from,
                to,
            }) => {
                let moved = self.capture_corner(at);
                if moved != to {
                    self.drag = Some(Drag::AdjustCrop {
                        across,
                        down,
                        from,
                        to: moved,
                    });
                    changed = true;
                }
            }
            Some(Drag::Level { index, level, grab }) => {
                if let Some((_, area, _)) = self.shown_slider() {
                    let moved = Level::along(self.along_at(at, grab, area));
                    if moved != level {
                        self.drag = Some(Drag::Level {
                            index,
                            level: moved,
                            grab,
                        });
                        changed = true;
                    }
                }
            }
            Some(Drag::Size { index, size, grab }) => {
                if let Some((_, area, _)) = self.shown_slider() {
                    let moved = Size::along(self.along_at(at, grab, area), self.span_of(index));
                    if moved != size {
                        self.drag = Some(Drag::Size {
                            index,
                            size: moved,
                            grab,
                        });
                        changed = true;
                    }
                }
            }
            None => {}
        }
        if let Some(from) = pan_from {
            changed |= self.pan(from, at);
        }
        if changed {
            let _ = self.present();
        }
    }

    /// Takes the stroke being drawn on to `at`, if the pointer has moved. True if it shows the
    /// stroke anew.
    fn extend_stroke(&mut self, at: POINT) -> bool {
        let point = self.fine_point(at);
        let Some(Drag::Pen {
            path,
            spacing,
            marked,
        }) = &mut self.drag
        else {
            return false;
        };
        if path.last() == Some(&point) {
            return false;
        }
        path.push(point);
        *marked = *marked || Stroke::from_drag(path, *spacing).is_some();
        *marked
    }

    /// Moves the image as far as the pointer moved from `from` to `to`. True if it moved.
    fn pan(&mut self, from: POINT, to: POINT) -> bool {
        self.view.pan_by(Vector2 {
            X: (to.x - from.x) as f32,
            Y: (to.y - from.y) as f32,
        })
    }

    /// Starts what the left button going down at `at` does: pressing a button, or a drag on the
    /// image area. A handle of the crop being chosen adjusts it, and a handle of the selected
    /// annotation resizes it with any tool; otherwise Select picks up the annotation under the
    /// pointer, or pans where there is none, a drawing tool draws, Text opens a box to type into
    /// with its top-left corner on the pixel under the pointer, Counter places the next badge
    /// centred on that pixel, and Crop chooses a new crop. True if the release needs to arrive
    /// here.
    fn left_down(&mut self, at: POINT) -> bool {
        self.history.interrupt();
        self.badge_pressed = None;
        let on_flyout = self.on_flyout(at);
        self.pressed = self.hit(at);
        // The readout's own click opens or closes its flyout on release.
        if !on_flyout && self.pressed != Some(Command::Dimensions) {
            self.flyout_after(flyout::Event::Press { open: self.flyout });
        }
        // Level and Size are no buttons: a knob follows the drag from where it was taken hold
        // of, or jumps to a press on the track first.
        if let Some(Command::Level | Command::Size) = self.pressed
            && let Some((slider, area, along)) = self.shown_slider()
            && let Some(index) = self.selected
        {
            self.pressed = None;
            let grab = flyout::grab(at.x as f32, area, along, self.toolbar.scale);
            let along = self.along_at(at, grab, area);
            self.drag = Some(match slider {
                Command::Level => Drag::Level {
                    index,
                    level: Level::along(along),
                    grab,
                },
                _ => Drag::Size {
                    index,
                    size: Size::along(along, self.span_of(index)),
                    grab,
                },
            });
            let _ = self.present();
            return true;
        }
        if self.pressed.is_some() {
            // Pressed from this moment, even with no move to say the pointer is on it.
            self.hover = self.pressed;
            let _ = self.present_bar();
            return true;
        }
        // A flyout covers the image there, and does nothing between its buttons.
        if on_flyout {
            return false;
        }
        if at.y < self.toolbar.height {
            return false;
        }
        let point = self.area_point(at);
        let from = self.capture_point(at);
        let selected = self.selected;
        let handle = selected.and_then(|index| {
            let handle = self.annotations[index].handle_at(point, &self.view)?;
            Some((index, handle))
        });
        let corner = self.capture_corner(at);
        let crop_handle = self.crop.and_then(|crop| crop.handle_at(point, &self.view));
        let drag = if let Some((across, down)) = crop_handle {
            Drag::AdjustCrop {
                across,
                down,
                from: corner,
                to: corner,
            }
        } else if let Some((index, handle)) = handle {
            Drag::Adjust {
                index,
                handle: Some(handle),
                from,
                to: from,
            }
        } else {
            match self.tool {
                Tool::Select => {
                    self.selected =
                        annotation::hit(&self.annotations, point, &self.view, self.image.scale);
                    // Even the annotation already selected opens the flyout under its tool.
                    self.selection_changed();
                    match self.selected {
                        Some(index) => Drag::Adjust {
                            index,
                            handle: None,
                            from,
                            to: from,
                        },
                        None => Drag::Pan(at),
                    }
                }
                // The press that draws deselects, which closes the flyout.
                Tool::Draw(kind) => {
                    self.selected = None;
                    self.selection_changed();
                    Drag::Draw {
                        kind,
                        from,
                        to: from,
                        marked: false,
                    }
                }
                Tool::Pen => {
                    self.selected = None;
                    self.selection_changed();
                    Drag::Pen {
                        path: vec![self.fine_point(at)],
                        // The same distance on screen at any zoom, as the hand shakes.
                        spacing: annotation::STROKE_SPACING / self.view.scale(),
                        marked: false,
                    }
                }
                Tool::Text => {
                    let corner = Vector2 {
                        X: from.X - 0.5,
                        Y: from.Y - 0.5,
                    };
                    // Only on the image, where the text can be seen in the output.
                    if self.on_image(corner) {
                        let font = CURRENT_TEXT_SIZE.get().font(self.image.scale);
                        self.start_typing(corner, "", CURRENT_COLOUR.get(), font, None);
                    } else if self.selected.take().is_some() {
                        self.selection_changed();
                        let _ = self.present();
                    }
                    return false;
                }
                Tool::Counter => {
                    // Only on the image, where the badge can be seen in the output.
                    if self.on_image(from) {
                        let badge = Badge {
                            centre: from,
                            count: annotation::next_count(&self.annotations),
                        };
                        // Selected, so its flyout opens for it.
                        self.place(Annotation::Badge(badge, next_style(Command::Counter)));
                        self.badge_pressed = Some(self.edits);
                        let _ = self.present();
                    } else if self.selected.take().is_some() {
                        self.selection_changed();
                        let _ = self.present();
                    }
                    return false;
                }
                Tool::Picker => {
                    self.pick(at);
                    return false;
                }
                Tool::Crop => {
                    self.selected = None;
                    Drag::Crop {
                        from: corner,
                        to: corner,
                        marked: false,
                    }
                }
            }
        };
        self.drag = Some(drag);
        if self.selected != selected {
            // A press that deselects closes the flyout.
            self.selection_changed();
            let _ = self.present();
        }
        true
    }

    /// Ends what the left button was doing, released at `at`: places the annotation being drawn
    /// and selects it, moves or resizes the one being adjusted, or keeps the crop being chosen,
    /// and returns the button released on the button it went down on, if any.
    fn left_up(&mut self, at: POINT) -> Option<Command> {
        let to = self.capture_point(at);
        let shown = self.shown_crop();
        match self.drag.take() {
            Some(Drag::Draw { kind, from, .. }) => {
                if let Some(annotation) = Annotation::from_drag(
                    kind,
                    from,
                    to,
                    next_style(Tool::Draw(kind).command()),
                    CURRENT_HEAD.get(),
                    CURRENT_FILL.get(),
                    CURRENT_LEVEL.get(),
                ) {
                    self.place(annotation);
                }
                let _ = self.present();
            }
            Some(Drag::Pen {
                mut path, spacing, ..
            }) => {
                let to = self.fine_point(at);
                if path.last() != Some(&to) {
                    path.push(to);
                }
                if let Some(stroke) = Stroke::from_drag(&path, spacing) {
                    self.place(Annotation::Stroke(stroke, next_style(Command::Pen)));
                }
                let _ = self.present();
            }
            Some(Drag::Adjust {
                index,
                handle,
                from,
                ..
            }) => {
                // As the drag showed it, except that an arrow's middle dropped near the line
                // through its ends straightens it.
                let annotation = &self.annotations[index];
                let adjusted = match handle {
                    Some(handle) => annotation.dropped(handle, to - from, self.view.scale()),
                    None => annotation.moved(to - from),
                };
                // A drag that ends where it started changes nothing.
                if adjusted != self.annotations[index] {
                    // A text resized by its handles sets the em of the next.
                    if let (Some(_), Annotation::Text(text, _)) = (handle, &adjusted) {
                        remember_text_size(text.font, self.image.scale);
                    }
                    self.remember();
                    self.annotations[index] = adjusted;
                    self.edited();
                    let _ = self.present();
                }
            }
            // Choosing a crop is not an edit; applying it is.
            Some(Drag::Crop { from, marked, .. }) => self.chose_crop(
                Drag::Crop {
                    from,
                    to: self.capture_corner(at),
                    marked,
                },
                shown,
            ),
            Some(Drag::AdjustCrop {
                across, down, from, ..
            }) => self.chose_crop(
                Drag::AdjustCrop {
                    across,
                    down,
                    from,
                    to: self.capture_corner(at),
                },
                shown,
            ),
            // As a colour or size chosen with something selected, the level is also the next
            // blur's.
            Some(Drag::Level { index, level, .. }) => {
                CURRENT_LEVEL.set(level);
                let changed = self.annotations[index].with_level(level);
                self.finish_slide(index, changed);
            }
            // And the size the next annotation of its tool is placed at, leaving every other
            // tool's.
            Some(Drag::Size { index, size, .. }) => {
                remember_size(flyout::placed_by(&self.annotations[index]), size);
                let changed = self.annotations[index].restyled(Choice::Size(size));
                self.finish_slide(index, changed);
            }
            Some(Drag::Pan(_)) | None => {}
        }
        let pressed = self.pressed.take()?;
        let _ = self.present_bar();
        (self.hit(at) == Some(pressed)).then_some(pressed)
    }

    /// Puts `changed` in place of the annotation at `index` as one edit, if a drag of a slider
    /// changed it, and shows the knob where the drag left it.
    fn finish_slide(&mut self, index: usize, changed: Annotation) {
        if changed != self.annotations[index] {
            self.remember();
            self.annotations[index] = changed;
            self.edited();
        }
        let _ = self.present();
    }

    /// Places `annotation`, just drawn, as an edit, and selects it, which opens the flyout under
    /// its tool.
    fn place(&mut self, annotation: Annotation) {
        self.remember();
        self.annotations.push(annotation);
        self.selected = Some(self.annotations.len() - 1);
        self.selection_changed();
        self.edited();
    }

    /// Keeps the crop where `drag`, just ended, leaves it, redrawing if that is not the crop
    /// `shown` during the drag: a drag that thinned past its start shows a crop it does not
    /// choose.
    fn chose_crop(&mut self, drag: Drag, shown: Option<Bounds>) {
        self.crop = self.crop_after(Some(&drag));
        if self.crop != shown {
            let _ = self.present();
        }
    }

    /// The crop as it is being chosen: where a drag has taken it, however thin once it has been
    /// large enough to choose, or as it was left.
    fn shown_crop(&self) -> Option<Bounds> {
        match &self.drag {
            &Some(Drag::Crop {
                from,
                to,
                marked: true,
            }) => Some(Bounds::spanned(from, to)),
            drag => self.crop_after(drag.as_ref()),
        }
    }

    /// The crop as `drag` would leave it. A new crop needs a drag of 4 by 4 capture pixels; a
    /// smaller one, such as a click, leaves the crop as it was.
    fn crop_after(&self, drag: Option<&Drag>) -> Option<Bounds> {
        match drag {
            Some(&Drag::Crop { from, to, .. }) => Bounds::from_drag(from, to).or(self.crop),
            Some(&Drag::AdjustCrop {
                across,
                down,
                from,
                to,
            }) => self.crop.map(|crop| {
                crop.adjusted(across, down, to - from)
                    .within(self.image_size())
            }),
            _ => self.crop,
        }
    }

    /// The annotation being moved, resized or given a level or a size, as its index and where
    /// the drag has taken it.
    fn moving(&self) -> Option<(usize, Annotation)> {
        match self.drag {
            Some(Drag::Adjust {
                index,
                handle,
                from,
                to,
            }) => Some((index, self.adjust(index, handle, to - from))),
            Some(Drag::Level { index, level, .. }) => {
                Some((index, self.annotations[index].with_level(level)))
            }
            Some(Drag::Size { index, size, .. }) => {
                Some((index, self.annotations[index].restyled(Choice::Size(size))))
            }
            _ => None,
        }
    }

    /// The slider the flyout that shows holds, if it shows one: Level or Size, where it is and
    /// how far along it its knob sits.
    fn shown_slider(&self) -> Option<(Command, D2D_RECT_F, f32)> {
        let layout = &self.toolbar.layout;
        match self.flyout_shown()? {
            (_, Shown::Level(level)) => {
                Some((Command::Level, layout.flyout.level, level.along_slider()))
            }
            (under, Shown::Style { size, .. }) => {
                let along = size?.along_slider(flyout::span(under));
                Some((Command::Size, layout.style_flyout(under)?.slider?, along))
            }
            (_, Shown::Resizes) => None,
        }
    }

    /// How far along the slider covering `area` the pointer at `at` puts its knob, wherever it
    /// is across, with the knob held `grab` pixels left of it.
    fn along_at(&self, at: POINT, grab: f32, area: D2D_RECT_F) -> f32 {
        flyout::along(at.x as f32 - grab, area, self.toolbar.scale)
    }

    /// The sizes the Size slider runs over for the annotation at `index`.
    fn span_of(&self, index: usize) -> Span {
        flyout::span(flyout::placed_by(&self.annotations[index]))
    }

    /// The annotation at `index` moved `delta` capture pixels, or with `handle` moved that far.
    fn adjust(&self, index: usize, handle: Option<Handle>, delta: Vector2) -> Annotation {
        let annotation = &self.annotations[index];
        match handle {
            Some(handle) => annotation.adjusted(handle, delta),
            None => annotation.moved(delta),
        }
    }

    /// Abandons a drag the left button is making on the image and the crop being chosen, leaving
    /// everything as it was before them. True if there was either.
    fn cancel(&mut self) -> bool {
        let (crop, drag) = (self.crop.take(), self.drag.take());
        if crop.is_some() || drag.as_ref().is_some_and(Drag::shows) {
            let _ = self.present();
        }
        crop.is_some() || drag.is_some()
    }

    /// Whether a mouse button is still held for this editor, so it keeps the mouse captured.
    fn holding(&self) -> bool {
        self.panning.is_some() || self.pressed.is_some() || self.drag.is_some()
    }

    /// Makes `tool` the active one, which a left drag on the image uses, as its button's click or
    /// its key does. Leaving Crop abandons the crop being chosen, even mid-drag. With one of the
    /// tool's own annotations selected its flyout stays open; anything else selected is
    /// deselected, which closes the flyout and opens none. Select's click or key while it is
    /// active changes nothing.
    fn choose(&mut self, tool: Tool) {
        let changed = self.tool != tool;
        if changed {
            self.history.interrupt();
            self.tool = tool;
            // The Picker acts on nothing selected, and shows no style for it. The pointer may have
            // left the window while no tool needed to hear it go, so the loupe starts from where
            // it is now, and hears when it leaves even with no move on the editor first.
            if tool == Tool::Picker {
                self.selected = None;
                self.pointer =
                    Some(pointer_now(self.hwnd)).filter(|&at| within(at, self.width, self.height));
                if self.pointer.is_some() {
                    track_leave(self.hwnd, TRACKMOUSEEVENT_FLAGS(0));
                }
            }
            if tool != Tool::Crop {
                self.crop = None;
                if self.choosing_crop() {
                    self.drag = None;
                }
            }
            // The group the knob was in may go, so a drag of it is abandoned.
            if self.sliding().is_some() {
                self.drag = None;
            }
        }
        // A click on Select while it is active changes nothing, so what it selected keeps its
        // flyout.
        let mut deselected = false;
        if changed || flyout::has_flyout(tool.command()) {
            self.flyout_after(flyout::Event::Tool {
                tool: tool.command(),
                selected: self.selected_by(),
            });
            // What the flyout closed for is another tool's.
            deselected = self.selected.is_some() && self.flyout.is_none();
            if deselected {
                self.selected = None;
            }
        }
        if changed || deselected {
            let _ = self.present();
        } else {
            self.refresh_flyout();
        }
    }

    /// Whether the left button is choosing a crop or adjusting it.
    fn choosing_crop(&self) -> bool {
        matches!(self.drag, Some(Drag::Crop { .. } | Drag::AdjustCrop { .. }))
    }

    /// Undoes the last edit, if any, selecting again what was selected before it. With a crop
    /// being chosen it abandons that instead, as Esc does. Not during a drag that shows on the
    /// image, which the undo would leave behind.
    fn undo(&mut self) {
        if self.drag.as_ref().is_some_and(Drag::shows) {
            return;
        }
        if self.crop.take().is_some() {
            let _ = self.present();
            return;
        }
        let now = self.step();
        let step = self.history.undo(now);
        if let Some(step) = step {
            self.restore(step);
        }
    }

    /// Redoes the last edit undone, if any, selecting again what was selected before the undo.
    /// Not during a drag that shows on the image or with a crop being chosen.
    fn redo(&mut self) {
        if self.drag.as_ref().is_some_and(Drag::shows) || self.crop.is_some() {
            return;
        }
        let now = self.step();
        let step = self.history.redo(now);
        if let Some(step) = step {
            self.restore(step);
        }
    }

    /// Makes the annotations, which is selected and the image what `step` keeps.
    fn restore(&mut self, step: Step) {
        self.annotations = step.annotations;
        // With the Picker nothing is selected, so no handle can be dragged.
        self.selected = step.selected.filter(|_| self.tool != Tool::Picker);
        // What it selects opens its flyout, as a press selecting it would, so Esc then closes
        // that before the editor.
        self.selection_changed();
        if !Arc::ptr_eq(&step.image, &self.image) {
            self.show(step.image, step.picture, step.zoom);
        }
        self.edited();
        let _ = self.present();
    }

    /// Moves the selected annotation `by` capture pixels, as one edit with the nudges just before
    /// it. False, so the key does what it would otherwise, if nothing is selected, during a drag,
    /// while typing or with Crop active.
    fn nudge(&mut self, by: Vector2) -> bool {
        let Some(index) = self.selected else {
            return false;
        };
        if self.drag.is_some() || self.tool == Tool::Crop || self.typing.is_some() {
            return false;
        }
        let now = self.step();
        self.history.nudge(now);
        self.annotations[index] = self.annotations[index].moved(by);
        self.edited();
        let _ = self.present();
        true
    }

    /// Applies the crop being chosen, if any: the image becomes what is inside it, with the
    /// annotations that draw anything there, shown centred at the same zoom. False if there is
    /// none. While a crop is still being dragged, nothing happens until it is released.
    fn apply_crop(&mut self) -> bool {
        if self.choosing_crop() {
            return true;
        }
        let Some(crop) = self.crop else {
            return false;
        };
        // The crop lies on pixel corners inside the image.
        let region = Region {
            left: crop.min.X as u32,
            top: crop.min.Y as u32,
            right: crop.max.X as u32,
            bottom: crop.max.Y as u32,
        };
        if region.width() == 0 || region.height() == 0 {
            return true;
        }
        let image = Arc::new(self.image.cropped(region));
        let Ok(picture) = picture(&self.context, &image) else {
            return true;
        };
        self.remember();
        self.annotations = annotation::crop(&self.annotations, region, self.image.scale);
        self.selected = None;
        self.crop = None;
        self.show(image, picture, self.view.zoom());
        self.edited();
        let _ = self.present();
        true
    }

    /// Resizes the capture by `resize`, as one edit: the image becomes its pixels at the new
    /// dimensions, scaled with the high quality a fitted image is drawn with, and every annotation
    /// shrinks with it, shown centred at the same zoom. A crop being chosen is abandoned, and
    /// nothing is selected, which closes the flyout.
    fn resize(&mut self, resize: Resize) {
        let (width, height) = resize.dimensions(self.image.width, self.image.height);
        // A capture already a pixel each way stays as it is, and so does everything on it.
        if (width, height) == (self.image.width, self.image.height) {
            self.flyout_after(flyout::Event::Escape);
            let _ = self.present_bar();
            return;
        }
        let Ok(image) = self.resized_image(width, height) else {
            return;
        };
        let image = Arc::new(image);
        let Ok(picture) = picture(&self.context, &image) else {
            return;
        };
        self.remember();
        let by = resize.fraction();
        self.annotations = self
            .annotations
            .iter()
            .map(|annotation| annotation.resized(by))
            .collect();
        self.selected = None;
        self.crop = None;
        self.selection_changed();
        // A fitted capture is shown smaller, not fitted again.
        self.view.keep_scale();
        self.show(image, picture, self.view.zoom());
        self.edited();
        let _ = self.present();
    }

    /// The capture's pixels at `width` by `height`, scaled from the picture with high quality, as
    /// a fitted image is drawn.
    fn resized_image(&self, width: u32, height: u32) -> Result<Image> {
        unsafe {
            let target = self.context.CreateBitmap(
                D2D_SIZE_U { width, height },
                None,
                0,
                &bitmap_properties(D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BITMAP_OPTIONS_TARGET),
            )?;
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            self.context.SetTransform(&Matrix3x2::identity());
            self.context.Clear(Some(&D2D1_COLOR_F::default()));
            self.context.DrawBitmap(
                &self.picture,
                Some(&D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: width as f32,
                    bottom: height as f32,
                }),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
            let drawn = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn?;
            Ok(Image::new(
                width,
                height,
                self.read(&target, width, height)?,
                self.image.taken,
                self.image.scale,
            ))
        }
    }

    /// The pixels of `bitmap`, `width` by `height`, in top-down rows.
    fn read(&self, bitmap: &ID2D1Bitmap1, width: u32, height: u32) -> Result<Vec<u8>> {
        unsafe {
            let readable = self.context.CreateBitmap(
                D2D_SIZE_U { width, height },
                None,
                0,
                &bitmap_properties(
                    D2D1_ALPHA_MODE_PREMULTIPLIED,
                    D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ),
            )?;
            readable.CopyFromBitmap(None, bitmap, None)?;
            let mapped = readable.Map(D2D1_MAP_OPTIONS_READ)?;
            let row = width as usize * 4;
            let mut pixels = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                let start = mapped.bits.add(y * mapped.pitch as usize);
                pixels.extend_from_slice(std::slice::from_raw_parts(start, row));
            }
            readable.Unmap()?;
            Ok(pixels)
        }
    }

    /// Applies the crop being chosen if the pointer double-clicked at `at` inside it, or with any
    /// tool opens the text annotation there to type into again. True if it did either. Not on a
    /// flyout, which covers the image there.
    fn double_click(&mut self, at: POINT) -> bool {
        if at.y < self.toolbar.height || self.drag.is_some() || self.on_flyout(at) {
            return false;
        }
        let point = self.area_point(at);
        let Some(crop) = self.crop else {
            // Under Counter the first press placed a badge, on top, which was not what was
            // double-clicked; it is taken back if a text was.
            let badge = self
                .badge_pressed
                .take()
                .is_some_and(|edits| edits == self.edits);
            let under = &self.annotations[..self.annotations.len() - usize::from(badge)];
            let hit = annotation::hit(under, point, &self.view, self.image.scale);
            let Some(index) = hit else {
                return false;
            };
            let Annotation::Text(text, colour) = &self.annotations[index] else {
                return false;
            };
            let (at, font, text, colour) = (text.at, text.font, text.text.clone(), *colour);
            if badge && let Some(step) = self.history.take_back() {
                self.restore(step);
            }
            self.start_typing(at, &text, colour, font, Some(index));
            return true;
        };
        let (min, max) = (self.view.to_area(crop.min), self.view.to_area(crop.max));
        let inside = point.X >= min.X && point.X < max.X && point.Y >= min.Y && point.Y < max.Y;
        inside && self.apply_crop()
    }

    /// Shows `image`, drawn as `picture`, in place of the image shown, centred at `zoom`.
    fn show(&mut self, image: Arc<Image>, picture: ID2D1Bitmap1, zoom: Zoom) {
        self.image = image;
        self.picture = picture;
        self.scaled = None;
        self.blurred = None;
        self.blocks.clear();
        self.view.set_image_at(self.image_size(), zoom);
    }

    /// Whether the capture point `at` is on the image, not beside it in the image area.
    fn on_image(&self, at: Vector2) -> bool {
        let size = self.image_size();
        at.X >= 0.0 && at.Y >= 0.0 && at.X < size.X && at.Y < size.Y
    }

    /// The capture pixel under the pointer at `at` in the client area, as its column and row, if
    /// the pointer is on the image where the bar does not cover it.
    fn pixel_under(&self, at: POINT) -> Option<[u32; 2]> {
        if !within(at, self.width, self.height) || at.y < self.toolbar.height {
            return None;
        }
        let point = self
            .view
            .to_capture(self.area_point(at) + Vector2 { X: 0.5, Y: 0.5 });
        let (x, y) = (point.X.floor(), point.Y.floor());
        self.on_image(Vector2 { X: x, Y: y })
            .then_some([x as u32, y as u32])
    }

    /// The capture pixel the Picker's loupe shows in its middle, if it shows: with the Picker
    /// active and the pointer on the image.
    fn loupe_pixel(&self) -> Option<[u32; 2]> {
        if self.tool != Tool::Picker {
            return None;
        }
        self.pixel_under(self.pointer?)
    }

    /// The pointer has gone off the client area, or onto the bar between its buttons, which is
    /// not the client area: the loupe goes.
    fn pointer_gone(&mut self) {
        let loupe_before = self.loupe_pixel();
        self.pointer = None;
        if loupe_before.is_some() {
            let _ = self.present();
        }
    }

    /// The hex code of the capture pixel at `at`, its column and row, as `shown_at` gives it.
    fn hex_at(&self, at: [u32; 2]) -> String {
        picker::hex(self.shown_at(at))
    }

    /// The capture pixel in column `x` and row `y` as the editor shows it over its backdrop: the
    /// capture's, not the annotations', so over a blur it is what the blur hides.
    fn shown_at(&self, [x, y]: [u32; 2]) -> [u8; 4] {
        picker::shown(self.image.pixel(x, y), self.ink.palette.backdrop)
    }

    /// With the Picker, a click at `at` on the image puts the hex code of the pixel under it on
    /// the clipboard in place of the image, and once it is there a Notice beside the pointer says
    /// so. It changes nothing in the editor.
    fn pick(&mut self, at: POINT) {
        let Some(under) = self.pixel_under(at) else {
            return;
        };
        let colour = self.shown_at(under);
        let mut pointer = at;
        let _ = unsafe { ClientToScreen(self.hwnd, &mut pointer) };
        self.copies.push_back((
            self.edits,
            Status::Picked { colour, pointer },
            AfterCopy::Stays,
        ));
        clipboard::copy_text(picker::hex(colour), self.tray, self.hwnd, WM_COPIED);
    }

    /// Draws the Picker's loupe beside the pointer, if it shows: the capture pixels around the one
    /// under the pointer, each a square, that one outlined, and its hex code under them, in the
    /// theme and in screen pixels. Only between `BeginDraw` and `EndDraw`.
    fn draw_loupe(&self) -> Result<()> {
        let (Some(pixel), Some(pointer)) = (self.loupe_pixel(), self.pointer) else {
            return Ok(());
        };
        let toolbar = &self.toolbar;
        let scale = toolbar.scale;
        let label = toolbar.text.label(&self.hex_at(pixel))?;
        let label_size = bar::size(&label)?;
        let cell = (picker::CELL * scale).round();
        let padding = (picker::PADDING * scale).round();
        let grid = cell * picker::CELLS as f32;
        let size = Vector2 {
            X: grid + 2.0 * padding,
            Y: grid + 3.0 * padding + label_size.Y.ceil(),
        };
        let corner = picker::place(
            Vector2 {
                X: pointer.x as f32,
                Y: (pointer.y - toolbar.height) as f32,
            },
            size,
            (picker::GAP * scale).round(),
            area(self.width, self.height, toolbar),
        );
        let (left, top) = (corner.X.round(), corner.Y.round() + toolbar.height as f32);
        let panel = D2D_RECT_F {
            left,
            top,
            right: left + size.X,
            bottom: top + size.Y,
        };
        let grid_area = D2D_RECT_F {
            left: left + padding,
            top: top + padding,
            right: left + padding + grid,
            bottom: top + padding + grid,
        };
        // The cells from `from` up to `to`, counted across and down from the grid's first.
        let cells = |from: [i64; 2], to: [i64; 2]| D2D_RECT_F {
            left: grid_area.left + from[0] as f32 * cell,
            top: grid_area.top + from[1] as f32 * cell,
            right: grid_area.left + to[0] as f32 * cell,
            bottom: grid_area.top + to[1] as f32 * cell,
        };
        // The capture pixels around the middle one that are on the image; the rest show the
        // backdrop, as beside the image.
        let reach = (picker::CELLS / 2) as i64;
        let [x, y] = pixel.map(i64::from);
        let first = [(x - reach).max(0), (y - reach).max(0)];
        let last = [
            (x + reach + 1).min(self.image.width as i64),
            (y + reach + 1).min(self.image.height as i64),
        ];
        let shown = cells(
            [first[0] - x + reach, first[1] - y + reach],
            [last[0] - x + reach, last[1] - y + reach],
        );
        let middle = cells([reach, reach], [reach + 1, reach + 1]);
        let rounded = |rect: D2D_RECT_F| D2D1_ROUNDED_RECT {
            rect,
            radiusX: bar::RADIUS * scale,
            radiusY: bar::RADIUS * scale,
        };
        unsafe {
            self.context
                .FillRoundedRectangle(&rounded(panel), &self.ink.loupe);
            self.context.DrawRoundedRectangle(
                &rounded(bar::inset(&panel, 0.5)),
                &self.ink.loupe_edge,
                1.0,
                None,
            );
            self.context.FillRectangle(&grid_area, &self.ink.off_image);
            self.context.DrawBitmap(
                &self.picture,
                Some(&shown),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                Some(&D2D_RECT_F {
                    left: first[0] as f32,
                    top: first[1] as f32,
                    right: last[0] as f32,
                    bottom: last[1] as f32,
                }),
                None,
            );
            // Black around white, so the outline shows on any colour.
            self.context.DrawRectangle(
                &bar::inset(&middle, -0.5),
                self.pens.brush(Colour::Black),
                1.0,
                None,
            );
            self.context.DrawRectangle(
                &bar::inset(&middle, 0.5),
                self.pens.brush(Colour::White),
                1.0,
                None,
            );
        }
        let at = Vector2 {
            X: (grid_area.left + (grid - label_size.X) / 2.0).round(),
            Y: grid_area.bottom + padding,
        };
        self.ink.bar.text(&label, at, Tone::Text);
        Ok(())
    }

    /// The image's size in capture pixels.
    fn image_size(&self) -> Vector2 {
        Vector2 {
            X: self.image.width as f32,
            Y: self.image.height as f32,
        }
    }

    /// Removes the selected annotation, if any, unless it is being moved or resized. Removing a
    /// badge renumbers those after it, in the same edit.
    fn delete(&mut self) {
        if self.moving().is_some() {
            return;
        }
        if let Some(index) = self.selected {
            self.remember();
            annotation::remove(&mut self.annotations, index);
            self.selected = None;
            self.edited();
            let _ = self.present();
        }
    }

    /// Opens a box holding `text` with its top-left corner where the text goes, `at` in capture
    /// pixels, written in `colour` with an em `font` capture pixels high, and gives it the
    /// keyboard. `editing` is the text annotation it types again, if any, which is drawn as it is
    /// typed meanwhile. The flyout closes for the typing.
    fn start_typing(
        &mut self,
        at: Vector2,
        text: &str,
        colour: Colour,
        font: f32,
        editing: Option<usize>,
    ) {
        let Ok(entry) = Entry::open(self.hwnd, text, colour) else {
            return;
        };
        self.selected = None;
        self.selection_changed();
        self.typing = Some(Typing {
            entry,
            at,
            colour,
            font,
            editing,
            selected: (0, 0),
            caret: 0,
            composing: false,
            blink: true,
        });
        // Placed, it can take the keyboard.
        self.place_typing();
        if let Some(typing) = &mut self.typing {
            typing.entry.focus();
            typing.follow(false);
        }
        self.blink_caret();
        let _ = self.present();
    }

    /// Reads what the box holds now, which may have changed, and draws it with the caret showing,
    /// which is after all of it if `all` was just selected.
    fn follow_typing(&mut self, all: bool) {
        let Some(typing) = &mut self.typing else {
            return;
        };
        typing.follow(all);
        self.blink_caret();
        let _ = self.present();
    }

    /// Shows the caret of text being typed, and blinks it from now on at Windows' rate, which
    /// may be never.
    fn blink_caret(&mut self) {
        let Some(typing) = &mut self.typing else {
            return;
        };
        typing.blink = true;
        let rate = unsafe { GetCaretBlinkTime() };
        if rate != 0 && rate != u32::MAX {
            unsafe { SetTimer(Some(self.hwnd), CARET_TIMER, rate, None) };
        } else {
            let _ = unsafe { KillTimer(Some(self.hwnd), CARET_TIMER) };
        }
    }

    /// The text being typed as it is placed, unless an input method is composing it, in its
    /// colour, with the bands behind its letters selected and its caret, if it shows, from its
    /// top-left corner.
    fn typed(&self) -> Option<(Text, Colour, Vec<Bounds>, Option<Caret>)> {
        let typing = self.typing.as_ref().filter(|typing| !typing.composing)?;
        let text = self
            .pens
            .text(typing.at, typing.entry.text(), typing.font)
            .ok()?;
        let laid = self.pens.laid(&text.text, typing.font).ok()?;
        let (start, end) = typing.selected;
        let caret = typing.blink.then(|| laid.caret(typing.caret));
        Some((text, typing.colour, laid.highlight(start, end), caret))
    }

    /// Ends typing, if any, keeping the text, and selects it, which opens Text's flyout for it. The
    /// text is placed, or replaces the text it types again, as one edit. Empty text places
    /// nothing, and emptying a text removes it. `press` is the press that ends it, if one does,
    /// which acts afterwards: nothing showed where the flyout hangs when it went down, so a press
    /// there leaves the flyout closed and acts on the image under it.
    fn finish_typing(&mut self, press: Option<POINT>) {
        let Some(Typing {
            entry,
            at,
            colour,
            font,
            editing,
            ..
        }) = self.typing.take()
        else {
            return;
        };
        let _ = unsafe { KillTimer(Some(self.hwnd), CARET_TIMER) };
        // Line breaks and spaces after the last character would only make its box larger.
        let text = entry.text().trim_end().to_owned();
        // The keyboard comes back to the editor, unless another window has taken it.
        if unsafe { GetFocus() } == entry.hwnd() {
            let _ = unsafe { SetFocus(Some(self.hwnd)) };
        }
        drop(entry);
        // A text typed again was selected when it opened, and is again, so undoing selects it.
        self.selected = editing;
        let typed = (!text.is_empty()).then(|| self.pens.text(at, text, font));
        match (editing, typed) {
            // Nothing typed, or a text that could not be laid out, changes nothing.
            (_, Some(Err(_))) | (None, None) => {}
            // Text placed or typed again sets the em of the next.
            (None, Some(Ok(text))) => {
                remember_text_size(text.font, self.image.scale);
                self.remember();
                self.annotations.push(Annotation::Text(text, colour));
                self.selected = Some(self.annotations.len() - 1);
                self.edited();
            }
            (Some(index), None) => {
                self.remember();
                self.annotations.remove(index);
                self.selected = None;
                self.edited();
            }
            (Some(index), Some(Ok(text))) => {
                let typed = Annotation::Text(text, colour);
                // Typing the same text again is no edit.
                if typed != self.annotations[index] {
                    remember_text_size(font, self.image.scale);
                    self.remember();
                    self.annotations[index] = typed;
                    self.edited();
                }
            }
        }
        self.selection_changed();
        if press.is_some_and(|at| self.on_flyout(at)) {
            self.flyout = None;
        }
        let _ = self.present();
    }

    /// Ends typing by Esc: keeps the text as Enter does, and makes Select the active tool, with
    /// the text selected and its flyout open.
    fn escape_typing(&mut self) {
        // Select first, since choosing it closes any flyout, and the text's opens after.
        self.choose(Tool::Select);
        self.finish_typing(None);
    }

    /// Puts the box where its text goes, in the font its text is drawn in at the current zoom: as
    /// large as the text while an input method composes in it, and of no size otherwise.
    fn place_typing(&mut self) {
        let Some(typing) = &mut self.typing else {
            return;
        };
        let em = typing.font * self.view.scale();
        let Ok(metrics) = self.pens.metrics(&typing.entry.text(), em) else {
            return;
        };
        let at = self.view.to_area(typing.at);
        typing.entry.place(
            at.X.round() as i32,
            at.Y.round() as i32 + self.toolbar.height,
            em,
            metrics.widthIncludingTrailingWhitespace,
            metrics.lineCount,
            typing.composing,
        );
    }

    /// Makes `colour` the current colour, and gives it to the annotation selected, if any, as one
    /// edit. A blur has no colour, so choosing one with it selected is no edit.
    fn choose_colour(&mut self, colour: Colour) {
        let mut changed = colour != CURRENT_COLOUR.get();
        CURRENT_COLOUR.set(colour);
        changed |= self.change_selected(|annotation| annotation.restyled(Choice::Colour(colour)));
        if changed {
            let _ = self.present();
        }
    }

    /// Makes `head` the head of the next arrow, and gives it to the arrow selected, if one is, as
    /// one edit.
    fn choose_head(&mut self, head: Head) {
        let mut changed = head != CURRENT_HEAD.get();
        CURRENT_HEAD.set(head);
        changed |= self.change_selected(|annotation| annotation.with_head(head));
        if changed {
            let _ = self.present();
        }
    }

    /// Makes `fill` the Fill of the next rectangle, and gives it to the rectangle selected, if one
    /// is, as one edit.
    fn choose_fill(&mut self, fill: Fill) {
        let mut changed = fill != CURRENT_FILL.get();
        CURRENT_FILL.set(fill);
        changed |= self.change_selected(|annotation| annotation.with_fill(fill));
        if changed {
            let _ = self.present();
        }
    }

    /// Puts what `change` makes of the annotation selected, if any, in its place as one edit, and
    /// says whether that changed it.
    fn change_selected(&mut self, change: impl FnOnce(&Annotation) -> Annotation) -> bool {
        let Some(index) = self.selected else {
            return false;
        };
        let changed = change(&self.annotations[index]);
        if changed == self.annotations[index] {
            return false;
        }
        self.remember();
        self.annotations[index] = changed;
        self.edited();
        true
    }

    /// Keeps the annotations, which is selected, and the image, for undoing the edit about to be
    /// made.
    fn remember(&mut self) {
        let step = self.step();
        self.history.record(step);
    }

    /// The annotations, which is selected, and the image, as they are now.
    fn step(&self) -> Step {
        Step {
            annotations: self.annotations.clone(),
            selected: self.selected,
            image: self.image.clone(),
            picture: self.picture.clone(),
            zoom: self.view.zoom(),
        }
    }

    /// After any change to the annotations or the image: the output is made again, and the
    /// status, which was about the image before, goes.
    fn edited(&mut self) {
        self.edits += 1;
        self.output = None;
        self.status = None;
    }

    /// The capture as Copy and Save produce it: at full resolution, with the blurs baked in and
    /// the arrows and rectangles drawn over them at the capture's scale.
    fn output(&mut self) -> Result<Arc<Image>> {
        if self.annotations.is_empty() {
            return Ok(self.image.clone());
        }
        if let Some(output) = &self.output {
            return Ok(output.clone());
        }
        let annotations = self.annotations.clone();
        let blurred = self.blurred(&annotations)?;
        let (width, height) = (self.image.width, self.image.height);
        let size = D2D_SIZE_U { width, height };
        unsafe {
            let target = self.context.CreateBitmap(
                size,
                None,
                0,
                &bitmap_properties(D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BITMAP_OPTIONS_TARGET),
            )?;
            // Copied rather than drawn, so every pixel the shapes miss is exactly as shown.
            target.CopyFromBitmap(None, &blurred, None)?;
            self.context.SetTarget(&target);
            self.context.BeginDraw();
            self.context.SetTransform(&Matrix3x2::identity());
            let drawn_annotations =
                annotation::draw(&self.context, &self.pens, &annotations, self.image.scale);
            let drawn = self.context.EndDraw(None, None);
            self.context.SetTarget(None);
            drawn_annotations?;
            drawn?;

            let output = Arc::new(Image::new(
                width,
                height,
                self.read(&target, width, height)?,
                self.image.taken,
                self.image.scale,
            ));
            self.output = Some(output.clone());
            Ok(output)
        }
    }

    /// The capture point the pointer at `at` in the client area is over, with the pointer kept
    /// inside the image area. The centre of the capture pixel it is on, so a stroke of odd width
    /// covers whole capture pixels and is crisp in the output.
    fn capture_point(&self, at: POINT) -> Vector2 {
        let point = self
            .view
            .to_capture(self.area_point(at) + Vector2 { X: 0.5, Y: 0.5 });
        Vector2 {
            X: point.X.floor() + 0.5,
            Y: point.Y.floor() + 0.5,
        }
    }

    /// The point under the pointer at `at` in the client area, in capture pixels, with the pointer
    /// kept inside the image area: the centre of the screen pixel it is on, so a stroke drawn
    /// zoomed in follows the hand more finely than a capture pixel. At 100 percent it is the
    /// centre of the capture pixel under the pointer.
    fn fine_point(&self, at: POINT) -> Vector2 {
        self.view
            .to_capture(self.area_point(at) + Vector2 { X: 0.5, Y: 0.5 })
    }

    /// The pixel corner of the image nearest the pointer at `at` in the client area, with the
    /// pointer kept inside the image area and the corner inside the image. At 100 percent it is
    /// the top-left corner of the pixel under the pointer.
    fn capture_corner(&self, at: POINT) -> Vector2 {
        let point = self.view.to_capture(self.area_point(at));
        let size = self.image_size();
        Vector2 {
            X: point.X.round().clamp(0.0, size.X),
            Y: point.Y.round().clamp(0.0, size.Y),
        }
    }

    /// `at` in the client area as a point in the image area, kept inside it.
    fn area_point(&self, at: POINT) -> Vector2 {
        let area = area(self.width, self.height, &self.toolbar);
        Vector2 {
            X: (at.x as f32).clamp(0.0, area.X),
            Y: ((at.y - self.toolbar.height) as f32).clamp(0.0, area.Y),
        }
    }

    /// Draws in the palette as it is now, after Windows' app mode or accent colour changed.
    fn retheme(&mut self) -> Result<()> {
        self.ink = Ink::new(&self.context)?;
        frame::border(self.hwnd, self.ink.palette.dark);
        // A tooltip showing changes colour with the rest.
        if let Some(button) = self.tipped.take() {
            self.show_tip(button);
        }
        self.present()
    }

    /// Lays the toolbar out again at `dpi`, keeps the image area's size, and returns the window's
    /// new size. A pinned picture keeps its size, the image's on the screen, and its bar waits
    /// for the editor to come back.
    fn rescale(&mut self, dpi: u32) -> Result<Option<(i32, i32)>> {
        self.end_tips();
        let toolbar = Toolbar::new(dpi)?;
        if self.pinned {
            self.toolbar = toolbar;
            self.toolbar.lay_out(self.width);
            return Ok(None);
        }
        // A window only as wide as the old toolbar becomes as wide as the new one.
        let width = if self.width <= self.toolbar.min_width {
            toolbar.min_width
        } else {
            self.width.max(toolbar.min_width)
        };
        let height = self.height - self.toolbar.height + toolbar.height;
        self.toolbar = toolbar;
        self.toolbar.lay_out(width);
        let borders = frame::borders(STYLE, dpi)?;
        Ok(Some((
            width + borders.right - borders.left,
            height + borders.bottom - borders.top,
        )))
    }
}

/// The size of the image area below the toolbar in a client area `width` by `height`.
fn area(width: i32, height: i32, toolbar: &Toolbar) -> Vector2 {
    Vector2 {
        X: width.max(1) as f32,
        Y: (height - toolbar.height).max(1) as f32,
    }
}

impl Toolbar {
    /// The command bar at `dpi`, laid out as narrow as it gets until `lay_out` says how wide the
    /// client area is.
    fn new(dpi: u32) -> Result<Toolbar> {
        let scale = dpi as f32 / 96.0;
        let text = bar::Text::new(scale)?;
        let mut status_width = 0.0f32;
        for status in Status::to_fit() {
            status_width = status_width.max(text.width(&status.text())?);
        }
        let mut zoom_width = 0.0f32;
        for level in View::labels() {
            zoom_width = zoom_width.max(text.width(&level)?);
        }
        let widths = Widths {
            zoom: zoom_width,
            status: status_width,
            // Every digit is as wide as any other in the bar's font, and no capture has a side
            // of more than five digits: Direct2D's largest bitmap is 16384 pixels across. The
            // crop's readout and the Dimensions readout are both this wide at their widest.
            readout: text.width(&command_bar::readout(99_999, 99_999))?,
            apply: text.width(APPLY)?,
            cancel: text.width(CANCEL)?,
        };
        let layout = Layout::new(scale, 0, &widths);
        let icons = layout
            .buttons
            .iter()
            .map(|(command, _)| {
                command
                    .icon()
                    .map(|(glyph, fallback)| text.icon(glyph, fallback))
                    .transpose()
            })
            .collect::<Result<_>>()?;
        let resizes = Resize::ALL
            .into_iter()
            .map(|resize| Ok((resize, text.label(resize.label())?)))
            .collect::<Result<Vec<_>>>()?;
        let mut resize_width = 0.0f32;
        for (_, label) in &resizes {
            resize_width = resize_width.max(bar::size(label)?.X);
        }
        Ok(Toolbar {
            apply: text.label(APPLY)?,
            cancel: text.label(CANCEL)?,
            resizes,
            resize_width,
            counter_digit: text.counter_digit()?,
            text,
            dpi,
            scale,
            height: layout.height,
            min_width: layout.min_width,
            widths,
            layout,
            icons,
        })
    }

    /// Lays the bar out for a client area `width` wide.
    fn lay_out(&mut self, width: i32) {
        self.layout = Layout::new(self.scale, width, &self.widths);
    }
}

fn contains(rect: &D2D_RECT_F, at: POINT) -> bool {
    let (x, y) = (at.x as f32, at.y as f32);
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

impl Ink {
    /// The palette for the theme the settings file asks for, as Windows is set now.
    fn new(context: &ID2D1DeviceContext) -> Result<Ink> {
        let palette = theme::palette(settings::current().theme);
        let brush = |rgb| unsafe { context.CreateSolidColorBrush(&color(rgb), None) };
        Ok(Ink {
            palette,
            backdrop: color(palette.backdrop),
            bar: Paint::new(context, &palette)?,
            handle: brush(palette.handle)?,
            accent: brush(palette.accent)?,
            highlight: unsafe {
                context.CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        a: HIGHLIGHT_OPACITY,
                        ..color(palette.accent)
                    },
                    None,
                )?
            },
            loupe: brush(palette.tooltip)?,
            loupe_edge: brush(palette.tooltip_border)?,
            off_image: brush(palette.backdrop)?,
            // The editor blends sRGB values, so this is about as dark as the overlay's dimming,
            // which blends in linear light.
            dim: unsafe {
                context.CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        a: palette.dim,
                        ..Default::default()
                    },
                    None,
                )?
            },
        })
    }
}

/// `image` as a bitmap `context` draws, premultiplied, as the capture holds it, so the backdrop
/// shows through a window's transparent corners.
fn picture(context: &ID2D1DeviceContext, image: &Image) -> Result<ID2D1Bitmap1> {
    unsafe {
        context.CreateBitmap(
            D2D_SIZE_U {
                width: image.width,
                height: image.height,
            },
            Some(image.pixels.as_ptr() as *const _),
            image.width * 4,
            &bitmap_properties(D2D1_ALPHA_MODE_PREMULTIPLIED, Default::default()),
        )
    }
}

fn bitmap_properties(
    alpha: D2D1_ALPHA_MODE,
    options: D2D1_BITMAP_OPTIONS,
) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: alpha,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        colorContext: std::mem::ManuallyDrop::new(None),
    }
}

/// Runs `f` on the editor `hwnd` is, if it is open and not already in use further up the stack.
fn with<T>(hwnd: HWND, f: impl FnOnce(&mut Editor) -> T) -> Option<T> {
    EDITORS.with(|editors| {
        let mut editors = editors.try_borrow_mut().ok()?;
        editors.iter_mut().find(|editor| editor.hwnd == hwnd).map(f)
    })
}

/// Copies the image. Once it is on the clipboard the editor closes, unless it is pinned or was
/// edited meanwhile.
fn copy(hwnd: HWND) {
    let output = with(hwnd, |editor| match editor.output() {
        Ok(image) => {
            editor
                .copies
                .push_back((editor.edits, Status::Copied, AfterCopy::Closes));
            Some((image, editor.tray))
        }
        Err(_) => {
            editor.set_status(Status::NotCopied);
            None
        }
    });
    if let Some(Some((image, tray))) = output {
        clipboard::copy(image, tray, Some(hwnd));
    }
}

/// Saves the capture to `to`, which then becomes its file, or else as Save does. Once it is saved a
/// Notice names the file and the editor closes, unless it is pinned or was edited meanwhile.
fn save(hwnd: HWND, to: Option<PathBuf>) {
    let output = with(hwnd, |editor| match editor.output() {
        Ok(image) => {
            editor.saves.push_back(editor.edits);
            Some((image, editor.file.clone(), editor.tray))
        }
        Err(_) => {
            editor.set_status(Status::NotSaved);
            None
        }
    });
    if let Some(Some((image, file, tray))) = output {
        save::save(image, file, to, tray, hwnd);
    }
}

/// Opens the Save As dialog, which posts `WM_FILE_CHOSEN` back when it closes. The dialog is the
/// editor's own, so the editor is entered and stays.
fn save_as(hwnd: HWND) {
    enter(hwnd);
    let asked = with(hwnd, |editor| {
        if editor.choosing_file {
            return None;
        }
        editor.choosing_file = true;
        Some((editor.image.taken, editor.file.lock().unwrap().clone()))
    });
    if let Some(Some((taken, file))) = asked {
        save::choose_file(taken, file, hwnd);
    }
}

/// Makes `fill` the Fill of the next rectangle in every editor and in the settings file, and
/// gives it to the rectangle the editor `hwnd` has selected. Written now, as a colour or size is.
fn choose_fill(hwnd: HWND, fill: Fill) {
    let before = CURRENT_FILL.get();
    with(hwnd, |editor| editor.choose_fill(fill));
    if CURRENT_FILL.get() != before {
        settings::store(Settings {
            fill: CURRENT_FILL.get(),
            ..settings::current()
        });
        repaint_others(hwnd);
    }
}

/// Makes `head` the head of the next arrow in every editor and in the settings file, and gives it
/// to the arrow the editor `hwnd` has selected. Written now, as a colour or size is.
fn choose_head(hwnd: HWND, head: Head) {
    let before = CURRENT_HEAD.get();
    with(hwnd, |editor| editor.choose_head(head));
    if CURRENT_HEAD.get() != before {
        settings::store(Settings {
            head: CURRENT_HEAD.get(),
            ..settings::current()
        });
        repaint_others(hwnd);
    }
}

/// Writes the level of the next blur and each tool's size to the settings file, if a drag of
/// Level or Size just changed them from `before`, and shows them in every editor but `hwnd`.
fn store_slider_choices(hwnd: HWND, before: (Level, Sizes)) {
    if (CURRENT_LEVEL.get(), CURRENT_SIZES.get()) != before {
        settings::store(Settings {
            blur: CURRENT_LEVEL.get(),
            sizes: CURRENT_SIZES.get(),
            ..settings::current()
        });
        repaint_others(hwnd);
    }
}

/// Makes `colour` the current colour in every editor and in the settings file, and gives it to
/// what the editor `hwnd` has selected.
fn choose_colour(hwnd: HWND, colour: Colour) {
    let before = CURRENT_COLOUR.get();
    with(hwnd, |editor| editor.choose_colour(colour));
    if CURRENT_COLOUR.get() != before {
        // Written now rather than at exit, so a crash never loses the choice. Only on a change,
        // so an invalid hand edit stays in the file until then.
        settings::store(Settings {
            colour: CURRENT_COLOUR.get(),
            ..settings::current()
        });
        repaint_others(hwnd);
    }
}

/// Repaints every editor but `hwnd` after the current colour, size, Head, Fill or level changed:
/// each toolbar shows them, unless something is selected there.
fn repaint_others(hwnd: HWND) {
    EDITORS.with(|editors| {
        for editor in editors.borrow().iter().filter(|editor| editor.hwnd != hwnd) {
            let _ = unsafe { InvalidateRect(Some(editor.hwnd), None, false) };
        }
    });
}

/// Draws every open editor in the palette as it is now, after the theme setting changed.
pub fn retheme_all() {
    let open: Vec<HWND> =
        EDITORS.with(|editors| editors.borrow().iter().map(|editor| editor.hwnd).collect());
    for hwnd in open {
        let _ = with(hwnd, Editor::retheme);
    }
}

/// Closes every untouched editor except `keep` for `cause`. Entered editors close only when the
/// user closes them. Text being typed is kept first.
fn close_untouched_but(keep: Option<HWND>, cause: Cause) {
    let closing: Vec<(HWND, Option<HWND>)> = EDITORS.with(|editors| {
        let Ok(mut editors) = editors.try_borrow_mut() else {
            return Vec::new();
        };
        for editor in editors.iter_mut() {
            editor.finish_typing(None);
        }
        editors
            .iter()
            .filter(|editor| editor.lifetime.closes_on(cause))
            .map(|editor| (editor.hwnd, editor.previous))
            .filter(|&(hwnd, _)| Some(hwnd) != keep)
            .collect()
    });
    for (hwnd, previous) in closing {
        if cause == Cause::NewCapture {
            let _ = unsafe { DestroyWindow(hwnd) };
        } else {
            leave(hwnd, previous);
        }
    }
}

/// Closes the untouched editor `hwnd`, which is leaving on its own, by its clock or a click
/// outside it. If it still has the keyboard, it goes back to `previous`, the window the user was
/// in, if that is still shown; otherwise Windows picks, as when any window closes.
fn leave(hwnd: HWND, previous: Option<HWND>) {
    unsafe {
        if GetForegroundWindow() == hwnd {
            window::give_front(previous);
        }
        let _ = DestroyWindow(hwnd);
    }
}

/// Enters the editor `hwnd` if it is untouched, fading or not.
fn enter(hwnd: HWND) {
    if with(hwnd, |editor| editor.lifetime.enter()) == Some(true) {
        became_entered(hwnd);
    }
}

/// `key` went down on the editor `hwnd`: it enters it if it is untouched, unless it is a modifier
/// on its own or copies, saves or closes it, which only stops its clock.
fn key_down(hwnd: HWND, key: lifetime::Key) {
    let (entering, stopped) = with(hwnd, |editor| {
        let running = editor.lifetime.clock_running();
        let entering = editor.lifetime.key(key);
        (entering, running && !editor.lifetime.clock_running())
    })
    .unwrap_or_default();
    if entering {
        became_entered(hwnd);
    } else if stopped {
        let _ = unsafe { KillTimer(Some(hwnd), LIFETIME_TIMER) };
    }
}

/// The editor `hwnd` has just been entered: it has the keyboard, is in the taskbar, shows fully
/// again if it was fading, and from now on stays until the user closes it.
fn became_entered(hwnd: HWND) {
    // Before anything that blocks: every mouse event on the system waits for the hook.
    watch_presses();
    unsafe {
        let _ = KillTimer(Some(hwnd), LIFETIME_TIMER);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let untouched = (WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0) as isize;
        if style & untouched != 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !untouched);
        }
        // A wheel turn does not activate the window as a click does.
        let _ = SetForegroundWindow(hwnd);
        // Windows adds no taskbar button for a window that was a tool window when it was shown.
        if let Ok(taskbar) =
            CoCreateInstance::<_, ITaskbarList>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            && taskbar.HrInit().is_ok()
        {
            let _ = taskbar.AddTab(hwnd);
        }
    }
}

/// What a key going down with Ctrl, Shift and Alt as they are is to an untouched editor: Esc,
/// Enter, Ctrl+C, Ctrl+S and Alt+F4 copy, save or close it, and Ctrl+Shift+S, which opens a
/// dialog, does not.
fn key_kind(key: u16, ctrl: bool, shift: bool, alt: bool) -> lifetime::Key {
    let modifiers = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN];
    if modifiers.iter().any(|modifier| modifier.0 == key) {
        lifetime::Key::Modifier
    } else if key == VK_ESCAPE.0
        || key == VK_RETURN.0
        || (ctrl && !alt && key == b'C' as u16)
        || (ctrl && !shift && !alt && key == b'S' as u16)
        || (alt && key == VK_F4.0)
    {
        lifetime::Key::Closes
    } else {
        lifetime::Key::Other
    }
}

/// `key_kind` for the key in a `WM_KEYDOWN` or `WM_SYSKEYDOWN`, with Ctrl, Shift and Alt as they
/// are now.
fn key_kind_now(wparam: WPARAM) -> lifetime::Key {
    let down = |key: u16| unsafe { GetKeyState(key as i32) } < 0;
    key_kind(
        wparam.0 as u16,
        down(VK_CONTROL.0),
        down(VK_SHIFT.0),
        down(VK_MENU.0),
    )
}

/// Pins the editor `hwnd`, entering it first if it is untouched: the bar, border, corners and
/// shadow go, the window becomes the image as it shows, where it shows, and it stays above every
/// other window. A drag it was making is abandoned.
fn pin(hwnd: HWND) {
    enter(hwnd);
    if unsafe { GetCapture() } == hwnd {
        let _ = unsafe { ReleaseCapture() };
    }
    let Some(Some(picture)) = with(hwnd, Editor::pin) else {
        return;
    };
    unsafe {
        // Without its resize frame it has no border, shadow or resize edges, and its client area
        // is the whole window.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_THICKFRAME.0 as isize));
        frame::square(hwnd);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            picture.left,
            picture.top,
            picture.right - picture.left,
            picture.bottom - picture.top,
            SWP_FRAMECHANGED | SWP_NOACTIVATE,
        );
    }
}

/// Brings the editor back around the pinned picture `hwnd`, as it was, with the image where the
/// picture was. It no longer stays above other windows.
fn unpin(hwnd: HWND) {
    let Some(Some(window)) = with(hwnd, Editor::unpin) else {
        return;
    };
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_THICKFRAME.0 as isize);
        frame::round(hwnd);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
            SWP_FRAMECHANGED | SWP_NOACTIVATE,
        );
    }
}

/// Puts the editor `hwnd`, just opened above every other window, below the pinned pictures, which
/// keep their order, until it no longer stays above other windows.
fn below_pinned(hwnd: HWND) {
    let pinned: Vec<HWND> = EDITORS.with(|editors| {
        editors
            .borrow()
            .iter()
            .filter(|editor| editor.pinned)
            .map(|editor| editor.hwnd)
            .collect()
    });
    if pinned.is_empty() {
        return;
    }
    // From the front back, the last pinned picture found is the lowest.
    let mut lowest = None;
    let mut next = unsafe { GetTopWindow(None) };
    while let Ok(window) = next {
        if pinned.contains(&window) {
            lowest = Some(window);
        }
        next = unsafe { GetWindow(window, GW_HWNDNEXT) };
    }
    if let Some(lowest) = lowest {
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                Some(lowest),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }
}

/// The screen the window `hwnd` is on, or the one most of it is on.
fn screen_of(hwnd: HWND) -> MONITORINFO {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let screen = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let _ = GetMonitorInfoW(screen, &mut info);
    }
    info
}

/// How large the screen `info` is, which a pinned picture is never larger than.
fn screen_size(info: &MONITORINFO) -> Vector2 {
    let screen = info.rcMonitor;
    Vector2 {
        X: (screen.right - screen.left) as f32,
        Y: (screen.bottom - screen.top) as f32,
    }
}

/// Whether Windows' "Animation effects" setting is on.
pub fn animates() -> bool {
    let mut on = BOOL(1);
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    on.as_bool()
}

/// The pointer is now at `at` in the client area of the editor `hwnd`, which is on it or off it.
/// Coming onto it stops the clock for good.
fn pointer_at(hwnd: HWND, at: POINT) {
    let stopped = with(hwnd, |editor| {
        let on = within(at, editor.width, editor.height);
        editor.lifetime.pointer(on)
    });
    if stopped == Some(true) {
        let _ = unsafe { KillTimer(Some(hwnd), LIFETIME_TIMER) };
    }
}

/// The editor `hwnd`'s clock has run out, or a step of its fade is due. The fade ends with the
/// editor leaving; with Windows' animation effects off, it leaves at once.
fn lifetime_tick(hwnd: HWND) {
    let now = unsafe { GetTickCount() };
    let Some((started, lifetime, previous)) = with(hwnd, |editor| {
        (
            editor.lifetime.clock_out(now),
            editor.lifetime,
            editor.previous,
        )
    }) else {
        return;
    };
    let Lifetime::Fading { since } = lifetime else {
        let _ = unsafe { KillTimer(Some(hwnd), LIFETIME_TIMER) };
        return;
    };
    let elapsed = now.wrapping_sub(since);
    if (started && !animates()) || elapsed >= FADE_MS {
        leave(hwnd, previous);
        return;
    }
    unsafe {
        if started {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
            SetTimer(Some(hwnd), LIFETIME_TIMER, FADE_STEP_MS, None);
        }
        let alpha = 255 - 255 * elapsed / FADE_MS;
        let _ = SetLayeredWindowAttributes(hwnd, Default::default(), alpha as u8, LWA_ALPHA);
    }
}

/// Watches every press on the screen while an untouched editor is open, for a click outside it,
/// and stops once none is. A click on another app brings it to the front, which closes the editor
/// as it does, but a click on a window that takes no front does not.
fn watch_presses() {
    let untouched = EDITORS.with(|editors| {
        let editors = editors.try_borrow().ok()?;
        editors
            .iter()
            .find(|editor| !editor.lifetime.entered())
            .map(|editor| editor.hwnd)
    });
    PRESSES_TO.set(untouched.unwrap_or_default());
    MOUSE_HOOK.with(|hook| unsafe {
        match untouched {
            Some(_) if hook.get().is_invalid() => {
                let instance = GetModuleHandleW(None).unwrap_or_default();
                hook.set(
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_pressed), Some(instance.into()), 0)
                        .unwrap_or_default(),
                );
            }
            None if !hook.get().is_invalid() => {
                let _ = UnhookWindowsHookEx(hook.replace(HHOOK::default()));
            }
            _ => {}
        }
    });
}

/// Every mouse event on the system waits for this, so it only posts each press to the untouched
/// editor and returns, never swallowing it.
unsafe extern "system" fn mouse_pressed(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let editor = PRESSES_TO.get();
    if code == HC_ACTION as i32
        && !editor.is_invalid()
        && matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    {
        let at = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt };
        let packed = (at.x as u16 as isize) | ((at.y as u16 as isize) << 16);
        let _ = unsafe { PostMessageW(Some(editor), WM_PRESSED, WPARAM(0), LPARAM(packed)) };
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Another app coming to the front, by a click or from the keyboard, closes the untouched editors.
fn watch_foreground() {
    FOREGROUND_HOOK.with(|hook| {
        if hook.get().is_invalid() {
            hook.set(unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_changed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            });
        }
    });
}

/// Stops watching for another app coming to the front, once no editor is open.
fn stop_watching_foreground() {
    FOREGROUND_HOOK.with(|hook| {
        let old = hook.replace(HWINEVENTHOOK(std::ptr::null_mut()));
        if !old.is_invalid() {
            let _ = unsafe { UnhookWinEvent(old) };
        }
    });
}

/// Called on the UI thread, from its message loop, some time after `hwnd` came to the front. A
/// change no newer than the newest editor closes nothing: the window the user was in may come to
/// the front for a moment as the overlay goes, before the editor takes it.
unsafe extern "system" fn foreground_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    time: u32,
) {
    // Tick counts wrap, so compare the difference. They also move in steps of about 16 ms, so a
    // change in the same step as the opening counts as older.
    if (time.wrapping_sub(OPENED_AT.get()) as i32) <= 0 {
        return;
    }
    if !hwnd.is_invalid() && !keeps_editors_open(hwnd) {
        close_untouched_but(None, Cause::ClickOutside);
    }
}

/// A mouse button went down at `at` on the screen. Outside every editor, on a window that takes no
/// front and would not keep them open, it is a click outside the untouched ones, which hand the
/// keyboard back as they close. A window that takes the front closes them as it comes to the
/// front, so the window the user was in never gets the keyboard for a moment on the way.
fn pressed_at(at: POINT) {
    let under = window::root(unsafe { WindowFromPoint(at) });
    if !keeps_editors_open(under) && !takes_front(under) {
        close_untouched_but(None, Cause::ClickOutside);
    }
}

/// Whether a click on the top-level window `hwnd` brings it to the front: it is enabled and not
/// one that never takes the front.
fn takes_front(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    unsafe { IsWindowEnabled(hwnd) }.as_bool() && style & WS_EX_NOACTIVATE.0 as isize == 0
}

/// Zilum's own windows, such as the tray menu and the overlay, and the taskbar, which the tray
/// icon sits on, with the flyout of hidden tray icons. Coming to the front, they do not count as
/// a click outside an editor.
fn keeps_editors_open(hwnd: HWND) -> bool {
    let root = window::root(hwnd);
    window::is_zilum(root) || window::is_taskbar(root)
}

/// Whether `at`, a point in the client area of a window `width` by `height`, is on it. The client
/// area is all of the editor that shows.
fn within(at: POINT, width: i32, height: i32) -> bool {
    (0..width).contains(&at.x) && (0..height).contains(&at.y)
}

/// The pointer in a mouse message, in physical client pixels.
fn pointer(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam.0 & 0xFFFF) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    }
}

/// Where the pointer is in the client area of `hwnd`.
fn pointer_now(hwnd: HWND) -> POINT {
    let mut at = POINT::default();
    let _ = unsafe { GetCursorPos(&mut at) };
    in_client(hwnd, at)
}

/// The point `at` on the screen as a point in the client area of `hwnd`.
fn in_client(hwnd: HWND, mut at: POINT) -> POINT {
    let _ = unsafe { ScreenToClient(hwnd, &mut at) };
    at
}

/// Asks for `WM_MOUSELEAVE`, or `WM_NCMOUSELEAVE` with `TME_NONCLIENT`, when the pointer leaves
/// that part of `hwnd`.
fn track_leave(hwnd: HWND, flags: TRACKMOUSEEVENT_FLAGS) {
    let mut track = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | flags,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = unsafe { TrackMouseEvent(&mut track) };
}

/// Captures the mouse for `hwnd`, unless it already has it: capturing again would tell it that it
/// lost the capture, ending the drag or pan another button is making.
fn capture(hwnd: HWND) {
    if unsafe { GetCapture() } != hwnd {
        unsafe { SetCapture(hwnd) };
    }
}

fn press(hwnd: HWND, command: Command) {
    if let Some(tool) = Tool::of(command) {
        with(hwnd, |editor| editor.choose(tool));
        return;
    }
    match command {
        Command::Zoom => {
            with(hwnd, |editor| {
                if editor.view.toggle() {
                    let _ = editor.present();
                }
            });
        }
        Command::Dimensions => {
            with(hwnd, Editor::toggle_dimensions);
        }
        Command::Resize(resize) => {
            with(hwnd, |editor| editor.resize(resize));
        }
        Command::Undo => {
            with(hwnd, Editor::undo);
        }
        Command::Copy => copy(hwnd),
        Command::Save => save(hwnd, None),
        Command::Pin => pin(hwnd),
        Command::Close => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        Command::Colour(colour) => choose_colour(hwnd, colour),
        Command::Head(head) => choose_head(hwnd, head),
        Command::Fill(fill) => choose_fill(hwnd, fill),
        // As Enter and Esc do with a crop being chosen.
        Command::Apply => {
            with(hwnd, Editor::apply_crop);
        }
        Command::Cancel => {
            with(hwnd, Editor::cancel);
        }
        _ => {}
    }
}

/// How far an arrow key moves a selected annotation in capture pixels: one, or ten with Shift.
/// None for any other key.
fn arrow(key: u16, shift: bool) -> Option<Vector2> {
    let [x, y] = match key {
        key if key == VK_LEFT.0 => [-1.0, 0.0],
        key if key == VK_RIGHT.0 => [1.0, 0.0],
        key if key == VK_UP.0 => [0.0, -1.0],
        key if key == VK_DOWN.0 => [0.0, 1.0],
        _ => return None,
    };
    let distance = if shift { 10.0 } else { 1.0 };
    Some(Vector2 {
        X: x * distance,
        Y: y * distance,
    })
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let _ = with(hwnd, |editor| editor.present());
            let _ = unsafe { ValidateRect(Some(hwnd), None) };
        }
        // The swap chain covers the whole window; erasing would only flicker.
        WM_ERASEBKGND => return LRESULT(1),
        WM_COPIED => {
            let copied = wparam.0 != 0;
            let mut asked = false;
            let close = with(hwnd, |editor| {
                let (at, status, after) = editor.copies.pop_front().unwrap_or((
                    editor.edits,
                    Status::Copied,
                    AfterCopy::Stays,
                ));
                // A pick's Notice shows whatever was edited since: it reports the clipboard.
                if let Status::Picked { colour, pointer } = status
                    && copied
                {
                    let text = picker::notice_text(colour);
                    notice::show_picked(&text, pointer, picker::rgb(colour));
                }
                if at != editor.edits {
                    return false;
                }
                asked = after == AfterCopy::Closes;
                // Only once it is on the clipboard, so a failed copy can be tried again. A pinned
                // picture stays: Esc is how it goes.
                if copied && after == AfterCopy::Closes && !editor.pinned {
                    return true;
                }
                editor.set_status(if copied { status } else { Status::NotCopied });
                false
            });
            if close == Some(true) {
                let _ = unsafe { DestroyWindow(hwnd) };
            } else if asked && !copied {
                // A copy asked for by a key on an untouched editor failed: it stays, to be tried
                // again, rather than close on the next click outside it.
                enter(hwnd);
            }
        }
        WM_SAVED => {
            let close = with(hwnd, |editor| {
                let current = editor.saves.pop_front().is_none_or(|at| at == editor.edits);
                let saved = (wparam.0 != 0)
                    .then(|| editor.file.lock().unwrap().clone())
                    .flatten();
                // The file is saved whatever was edited since.
                // On the current screen, the one under the pointer.
                if let Some(file) = &saved {
                    let mut pointer = POINT::default();
                    let _ = unsafe { GetCursorPos(&mut pointer) };
                    let screen = unsafe { MonitorFromPoint(pointer, MONITOR_DEFAULTTONEAREST) };
                    notice::show(&save::notice_text(file), screen);
                }
                if !current {
                    return false;
                }
                if saved.is_none() {
                    editor.set_status(Status::NotSaved);
                    return false;
                }
                !editor.pinned
            });
            if close == Some(true) {
                let _ = unsafe { DestroyWindow(hwnd) };
            } else if wparam.0 == 0 {
                // As for a failed copy.
                enter(hwnd);
            }
        }
        WM_FILE_CHOSEN => {
            let chosen =
                (lparam.0 != 0).then(|| *unsafe { Box::from_raw(lparam.0 as *mut PathBuf) });
            with(hwnd, |editor| editor.choosing_file = false);
            // Cancelling changes nothing.
            if let Some(chosen) = chosen {
                save(hwnd, Some(chosen));
            }
        }
        WM_TYPED => {
            with(hwnd, |editor| {
                // Not a box that has already gone.
                let open = editor.typing.as_ref().map(|typing| typing.entry.hwnd());
                if open.is_some_and(|edit| edit.0 as isize == lparam.0) {
                    if wparam.0 != 0 {
                        editor.escape_typing();
                    } else {
                        editor.finish_typing(None);
                    }
                }
            });
        }
        // What the box holds may have changed, so the text is drawn as it is now, with the caret
        // showing from the start of a blink, and the box, if it shows, fits it.
        WM_ENTRY_CHANGED => {
            with(hwnd, |editor| editor.follow_typing(wparam.0 != 0));
        }
        WM_COMMAND if (wparam.0 >> 16) as u32 == EN_CHANGE => {
            with(hwnd, |editor| editor.follow_typing(false));
        }
        // The box shows only while an input method composes.
        WM_COMPOSING => {
            with(hwnd, |editor| {
                if let Some(typing) = &mut editor.typing {
                    typing.composing = wparam.0 != 0;
                    let _ = editor.present();
                }
            });
        }
        WM_TIMER if wparam.0 == CARET_TIMER => {
            with(hwnd, |editor| {
                if let Some(typing) = &mut editor.typing {
                    typing.blink = !typing.blink;
                    let _ = editor.present();
                }
            });
        }
        // Windows' app mode or accent changed: the editor stays open, as it was, in the new colours.
        _ if theme::changed(msg, lparam) => {
            let _ = with(hwnd, Editor::retheme);
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_CTLCOLOREDIT => {
            return entry::colours(HDC(wparam.0 as *mut _), HWND(lparam.0 as *mut _));
        }
        // Any key hides the tooltip, and the next one waits again, in the box text is typed into
        // as anywhere.
        WM_ENTRY_KEY => {
            with(hwnd, Editor::end_tips);
        }
        WM_SYSKEYDOWN => {
            key_down(hwnd, key_kind_now(wparam));
            with(hwnd, Editor::end_tips);
            // Alt+F4 closes the editor as Esc does. Bit 30 is set when the key was already down:
            // holding it on closes nothing more.
            if wparam.0 == VK_F4.0 as usize {
                let repeat = lparam.0 & (1 << 30) != 0;
                if !repeat {
                    let _ = unsafe { DestroyWindow(hwnd) };
                }
                return LRESULT(0);
            }
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // A pinned picture closes, copies and saves, and no other key does anything.
        WM_KEYDOWN if with(hwnd, |editor| editor.pinned) == Some(true) => {
            let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let repeat = lparam.0 & (1 << 30) != 0;
            match wparam.0 as u16 {
                key if key == VK_ESCAPE.0 && !repeat => {
                    let _ = unsafe { DestroyWindow(hwnd) };
                }
                key if key == b'C' as u16 && ctrl && !repeat => copy(hwnd),
                key if key == b'S' as u16 && ctrl && !shift && !repeat => save(hwnd, None),
                _ => {}
            }
        }
        // A key enters an untouched editor, and then does what it does.
        WM_KEYDOWN => {
            key_down(hwnd, key_kind_now(wparam));
            with(hwnd, Editor::end_tips);
            let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            // Bit 30 is set when the key was already down: holding Ctrl+C copies once, and
            // holding Ctrl+S saves once.
            let repeat = lparam.0 & (1 << 30) != 0;
            match wparam.0 as u16 {
                key if key == VK_ESCAPE.0 => {
                    // During a drag or with a crop being chosen Esc cancels them. Otherwise with
                    // a flyout showing it closes it and deselects what it was for, and nothing
                    // else. Holding it on closes nothing.
                    if with(hwnd, Editor::cancel) != Some(true)
                        && with(hwnd, Editor::escape_flyout) != Some(true)
                        && !repeat
                    {
                        let _ = unsafe { DestroyWindow(hwnd) };
                    }
                }
                // With a crop being chosen Enter applies it; holding it on copies nothing more.
                key if key == VK_RETURN.0 && !repeat => {
                    if with(hwnd, Editor::apply_crop) != Some(true) {
                        copy(hwnd);
                    }
                }
                key if key == b'C' as u16 && ctrl && !repeat => copy(hwnd),
                key if key == b'S' as u16 && ctrl && shift && !repeat => save_as(hwnd),
                key if key == b'S' as u16 && ctrl && !repeat => save(hwnd, None),
                key if (key == b'Z' as u16 && ctrl && shift) || (key == b'Y' as u16 && ctrl) => {
                    with(hwnd, Editor::redo);
                }
                key if key == b'Z' as u16 && ctrl => {
                    with(hwnd, Editor::undo);
                }
                key if key == VK_DELETE.0 || key == VK_BACK.0 => {
                    with(hwnd, Editor::delete);
                }
                key if key == b'S' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Select));
                }
                key if key == b'A' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Draw(Kind::Arrow)));
                }
                key if key == b'R' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Draw(Kind::Rectangle)));
                }
                key if key == b'P' as u16 && ctrl && !repeat => pin(hwnd),
                key if key == b'P' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Pen));
                }
                key if key == b'T' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Text));
                }
                key if key == b'N' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Counter));
                }
                key if key == b'B' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Draw(Kind::Blur)));
                }
                key if key == b'I' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Picker));
                }
                key if key == b'C' as u16 && !ctrl => {
                    with(hwnd, |editor| editor.choose(Tool::Crop));
                }
                key if (key == b'0' as u16 || key == VK_NUMPAD0.0) && ctrl => {
                    with(hwnd, |editor| {
                        if editor.view.fit() {
                            let _ = editor.present();
                        }
                    });
                }
                key if (key == b'1' as u16 || key == VK_NUMPAD1.0) && ctrl => {
                    with(hwnd, |editor| {
                        if editor.view.actual_size() {
                            let _ = editor.present();
                        }
                    });
                }
                // 1 to 8, on the main keys or the keypad, pick the colours in the toolbar's order.
                key if !ctrl
                    && ((b'1' as u16..=b'8' as u16).contains(&key)
                        || (VK_NUMPAD1.0..=VK_NUMPAD8.0).contains(&key)) =>
                {
                    let digit = if key >= VK_NUMPAD1.0 {
                        key - VK_NUMPAD1.0
                    } else {
                        key - b'1' as u16
                    };
                    let colour = Colour::ALL[digit as usize];
                    choose_colour(hwnd, colour);
                }
                // An arrow key moves the selected annotation one capture pixel, ten with Shift.
                key if !ctrl && arrow(key, shift).is_some() => {
                    let by = arrow(key, shift).unwrap_or_default();
                    if with(hwnd, |editor| editor.nudge(by)) != Some(true) {
                        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                    }
                }
                _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_PRESSED => pressed_at(pointer(lparam)),
        // A wheel turn or a press enters an untouched editor, and then does what it does.
        WM_MOUSEWHEEL => {
            enter(hwnd);
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
            // The wheel's pointer is on the screen.
            let at = in_client(hwnd, pointer(lparam));
            // A pinned picture is resized to the image as it zooms.
            if let Some(Some(picture)) = with(hwnd, |editor| editor.wheel(delta, at)) {
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        picture.left,
                        picture.top,
                        picture.right - picture.left,
                        picture.bottom - picture.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
        }
        WM_MBUTTONDOWN => {
            enter(hwnd);
            with(hwnd, |editor| {
                editor.end_tips();
                editor.press_outside();
                editor.history.interrupt();
                editor.panning = Some(pointer(lparam));
            });
            // So the image follows a drag that leaves the window.
            capture(hwnd);
        }
        WM_MBUTTONUP => {
            let holding = with(hwnd, |editor| {
                editor.panning = None;
                editor.holding()
            });
            if holding == Some(false) {
                let _ = unsafe { ReleaseCapture() };
            }
        }
        WM_MOUSEMOVE => {
            pointer_at(hwnd, pointer(lparam));
            with(hwnd, |editor| editor.pointer_moved(pointer(lparam)));
            let on_button = with(hwnd, |editor| editor.hover_at(pointer(lparam))) == Some(true);
            // An untouched editor's clock needs to know when the pointer leaves it, and so does
            // the loupe.
            let watching = with(hwnd, |editor| {
                editor.lifetime.clock_running() || editor.tool == Tool::Picker
            }) == Some(true);
            if on_button || watching {
                // So the highlight goes when the pointer leaves the button for the rest of the
                // bar, which moves the window and so is not the client area, or the window.
                track_leave(hwnd, TRACKMOUSEEVENT_FLAGS(0));
            }
        }
        // On the bar between buttons, or on an edge, which is off the editor where Windows keeps
        // its invisible resize borders.
        WM_NCMOUSEMOVE => {
            let at = in_client(hwnd, pointer(lparam));
            pointer_at(hwnd, at);
            with(hwnd, |editor| {
                editor.pointer_gone();
                editor.hover_at(at)
            });
            track_leave(hwnd, TME_NONCLIENT);
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // Off a button, or off the bar between buttons, onto wherever the pointer is now: still
        // on the bar, the next tooltip still shows at once.
        WM_MOUSELEAVE | WM_NCMOUSELEAVE => {
            let at = pointer_now(hwnd);
            pointer_at(hwnd, at);
            if msg == WM_MOUSELEAVE {
                with(hwnd, Editor::pointer_gone);
            }
            if with(hwnd, |editor| editor.hover_at(at)) == Some(true) {
                track_leave(hwnd, TRACKMOUSEEVENT_FLAGS(0));
            }
        }
        WM_TIMER if wparam.0 == TIP_TIMER => {
            with(hwnd, Editor::rested);
        }
        WM_TIMER if wparam.0 == LIFETIME_TIMER => lifetime_tick(hwnd),
        // The second press of a double-click, which applies a crop when inside it, opens a text
        // with any tool, and is otherwise a press like the first. Either press outside the box
        // text is typed into keeps the text before it acts.
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            enter(hwnd);
            let at = pointer(lparam);
            with(hwnd, |editor| {
                editor.end_tips();
                editor.finish_typing(Some(at));
            });
            let applied = msg == WM_LBUTTONDBLCLK
                && with(hwnd, |editor| editor.double_click(at)) == Some(true);
            if !applied && with(hwnd, |editor| editor.left_down(at)) == Some(true) {
                // So the release arrives here even off the button, where it presses nothing,
                // and a drag follows the pointer out of the window.
                capture(hwnd);
            }
            // A press on an annotation under Select opens the flyout under its tool, and the press
            // that draws closes it, which shows once the press has done what it does, even on
            // the annotation already selected.
            with(hwnd, Editor::refresh_flyout);
        }
        WM_LBUTTONUP => {
            let before = (CURRENT_LEVEL.get(), CURRENT_SIZES.get());
            let (pressed, holding) = with(hwnd, |editor| {
                let pressed = editor.left_up(pointer(lparam));
                (pressed, editor.holding())
            })
            .unwrap_or((None, false));
            // Written on release, so a drag of Level or Size writes the file once.
            store_slider_choices(hwnd, before);
            // A pan the middle button is still making keeps following the pointer.
            if !holding {
                let _ = unsafe { ReleaseCapture() };
            }
            if let Some(button) = pressed {
                press(hwnd, button);
            }
        }
        WM_CAPTURECHANGED => {
            with(hwnd, |editor| {
                editor.panning = None;
                // What was being drawn, moved, resized or cropped goes back to how it was.
                let on_screen = editor.drag.take().is_some_and(|drag| drag.shows());
                let pressed = editor.pressed.take().is_some();
                if on_screen {
                    let _ = editor.present();
                } else if pressed {
                    let _ = editor.present_bar();
                }
            });
        }
        WM_RBUTTONDOWN => {
            enter(hwnd);
            with(hwnd, |editor| {
                editor.end_tips();
                editor.press_outside();
            });
        }
        // A right-click on Save opens Save As, keeping any text being typed first as a click
        // does.
        WM_RBUTTONUP => {
            let on_save = with(hwnd, |editor| {
                let on_save = editor.hit(pointer(lparam)) == Some(Command::Save);
                if on_save {
                    editor.finish_typing(None);
                }
                on_save
            });
            if on_save == Some(true) {
                save_as(hwnd);
            }
        }
        // Windows keeps the resize borders beside and below the client area, and draws nothing
        // above it: the bar is the window's top. A pinned picture has none.
        WM_NCCALCSIZE => {
            let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
            frame::calc_size(hwnd, WINDOW_STYLE(style as u32), wparam, lparam);
        }
        // Windows paints the classic sizing border over the top of the client area, where its
        // own frame would be, in the active or inactive border colour, when the editor gains or
        // loses the front: a light band above the bar. A -1 asks it to change the state without
        // the painting. The border, corners and shadow are DWM's and are not affected.
        WM_NCACTIVATE => {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, LPARAM(-1)) };
        }
        WM_NCHITTEST => {
            let at = in_client(hwnd, pointer(lparam));
            // Before the editor exists, while the window is created.
            let Some(part) = with(hwnd, |editor| editor.part(at)) else {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            };
            return LRESULT(part.code() as isize);
        }
        // A press that moves or resizes the window enters it, and hides the tooltip. On the bar it
        // keeps any text being typed first, as a click does. It is outside any flyout, so the
        // Dimensions flyout closes.
        WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN => {
            enter(hwnd);
            with(hwnd, |editor| {
                editor.end_tips();
                if msg == WM_NCLBUTTONDOWN && wparam.0 == HTCAPTION as usize {
                    editor.finish_typing(None);
                }
                editor.press_outside();
            });
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // A side button enters an untouched editor as any press does, and does nothing else.
        WM_XBUTTONDOWN | WM_NCXBUTTONDOWN => {
            enter(hwnd);
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // Double-clicking the bar or an edge does nothing, rather than maximise the window or
        // stretch it to the screen's height. Double-clicking a pinned picture brings the editor
        // back.
        WM_NCLBUTTONDBLCLK => unpin(hwnd),
        WM_SIZE => {
            let _ = with(hwnd, |editor| editor.resized());
        }
        WM_GETMINMAXINFO => {
            // Before the editor exists, while the window is created at a size that is larger.
            if let Some(Ok(min)) = with(hwnd, |editor| editor.min_size()) {
                unsafe { (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = min };
            }
        }
        WM_DPICHANGED => {
            // The suggested place, at the size the new toolbar and the same image area need. The
            // toolbar's height differs at every DPI, so the client area changes and WM_SIZE lays
            // the editor out again.
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            if let Some(Ok(Some((width, height)))) = with(hwnd, |editor| editor.rescale(dpi)) {
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
        }
        WM_DESTROY => {
            let empty = EDITORS.with(|editors| {
                let mut editors = editors.borrow_mut();
                editors.retain(|editor| editor.hwnd != hwnd);
                editors.is_empty()
            });
            if empty {
                stop_watching_foreground();
            }
            watch_presses();
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The primary screen's work area on the development machine, above the taskbar.
    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 2560,
        bottom: 1392,
    };
    const BAR_HEIGHT: i32 = 40;
    const MIN_WIDTH: i32 = 560;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    #[test]
    fn every_editor_opens_two_thirds_of_the_work_area_each_way_centred_in_it() {
        // 2560 by 1392: 1706 by 928, 427 and 232 in from the left and top.
        let client = place(WORK, BAR_HEIGHT, MIN_WIDTH);
        assert_eq!(edges(client), (427, 232, 2133, 1160));
    }

    #[test]
    fn with_the_taskbar_on_the_left_the_editor_is_centred_right_of_it() {
        // 2490 by 1440 from 70 in: 1660 by 960, 415 and 240 in from its left and top.
        let client = place(rect(70, 0, 2560, 1440), BAR_HEIGHT, MIN_WIDTH);
        assert_eq!(edges(client), (485, 240, 2145, 1200));
    }

    #[test]
    fn on_a_small_work_area_the_editor_is_never_below_its_minimum_size() {
        // Two thirds of 600 by 90 is 400 by 60: below the bar's 560 and twice the bar's 40.
        let client = place(rect(0, 0, 600, 90), BAR_HEIGHT, MIN_WIDTH);
        assert_eq!(edges(client), (20, 5, 580, 85));
    }

    fn v(x: f32, y: f32) -> Vector2 {
        Vector2 { X: x, Y: y }
    }

    #[test]
    fn a_pinned_picture_is_exactly_the_image_where_it_was_shown() {
        let picture = pinned_window(v(1180.0, 641.0), v(200.0, 150.0));
        assert_eq!(edges(picture), (1180, 641, 1380, 791));
    }

    #[test]
    fn a_pinned_picture_covers_the_last_part_pixel_of_an_image_zoomed_between_pixels() {
        // 201 by 99 capture pixels at 75 percent.
        let picture = pinned_window(v(-300.0, 20.0), v(150.75, 74.25));
        assert_eq!(edges(picture), (-300, 20, -149, 95));
    }

    #[test]
    fn a_small_picture_comes_back_centred_under_a_bar_wider_than_it() {
        // 560 wide, the 200 by 150 picture 180 in from the left and under the bar.
        let picture = pinned_window(v(1180.0, 641.0), v(200.0, 150.0));
        let back = client_around(picture, BAR_HEIGHT, MIN_WIDTH, WORK.top);
        assert_eq!(edges(back), (1000, 601, 1560, 791));
    }

    #[test]
    fn the_editor_comes_back_with_the_bar_above_a_picture_wider_than_it() {
        let back = client_around(rect(100, 300, 900, 700), BAR_HEIGHT, MIN_WIDTH, WORK.top);
        assert_eq!(edges(back), (100, 260, 900, 700));
    }

    #[test]
    fn a_picture_shorter_than_the_bar_comes_back_centred_in_an_image_area_as_tall_as_the_bar() {
        // 20 high in an image area 40 high: 10 above it.
        let back = client_around(rect(1000, 500, 1600, 520), BAR_HEIGHT, MIN_WIDTH, WORK.top);
        assert_eq!(edges(back), (1000, 450, 1600, 530));
    }

    #[test]
    fn a_picture_pinned_at_the_top_of_the_screen_comes_back_with_the_bar_on_it() {
        let back = client_around(rect(1000, 10, 1600, 160), BAR_HEIGHT, MIN_WIDTH, WORK.top);
        assert_eq!(edges(back), (1000, 0, 1600, 190));
    }

    #[test]
    fn copy_save_and_close_keys_close_an_untouched_editor_without_entering_it() {
        use lifetime::Key;
        let (c, s) = (b'C' as u16, b'S' as u16);
        assert_eq!(key_kind(c, true, false, false), Key::Closes);
        assert_eq!(key_kind(s, true, false, false), Key::Closes);
        assert_eq!(key_kind(VK_RETURN.0, false, false, false), Key::Closes);
        assert_eq!(key_kind(VK_ESCAPE.0, false, false, false), Key::Closes);
        assert_eq!(key_kind(VK_F4.0, false, false, true), Key::Closes);
    }

    #[test]
    fn save_as_tool_keys_and_other_keys_enter_an_untouched_editor() {
        use lifetime::Key;
        let (a, c, s, z) = (b'A' as u16, b'C' as u16, b'S' as u16, b'Z' as u16);
        assert_eq!(key_kind(s, true, true, false), Key::Other);
        assert_eq!(key_kind(a, false, false, false), Key::Other);
        assert_eq!(key_kind(c, false, false, false), Key::Other);
        assert_eq!(key_kind(s, false, false, false), Key::Other);
        assert_eq!(key_kind(z, true, false, false), Key::Other);
        assert_eq!(key_kind(VK_F4.0, false, false, false), Key::Other);
    }

    #[test]
    fn a_modifier_on_its_own_is_not_a_key_press_yet() {
        use lifetime::Key;
        for modifier in [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN] {
            assert_eq!(key_kind(modifier.0, true, true, true), Key::Modifier);
        }
    }
}
