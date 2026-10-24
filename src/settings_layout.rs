//! Where everything in the Settings window goes, and how the keyboard moves through it: four
//! groups in one column under a title row, each row a label and a control, every field with room
//! under it for a message, and the version centred at the bottom.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::{background::Background, hotkey::Mode, theme::Theme};

/// Measurements in logical pixels. The window, its margin either side, and where the controls'
/// column starts, right of the labels.
const WIDTH: f32 = 520.0;
const MARGIN: f32 = 16.0;
const FIELD_LEFT: f32 = 144.0;
/// The title row, as tall as the editor's command bar, with a caption-style Close at its end.
const TITLE_HEIGHT: f32 = 40.0;
const CLOSE_WIDTH: f32 = 46.0;
/// A group's heading and the space above and below it.
const ABOVE_HEADING: f32 = 16.0;
const HEADING_HEIGHT: f32 = 20.0;
const BELOW_HEADING: f32 = 8.0;
/// A row of controls, and the space after a row.
const FIELD_HEIGHT: f32 = 32.0;
const ROW_GAP: f32 = 8.0;
/// Between a field and its message, and one line of a message.
const MESSAGE_GAP: f32 = 4.0;
const LINE_HEIGHT: f32 = 16.0;
/// A hotkey field, and the clear control at its right end, as wide as the field is tall.
const HOTKEY_WIDTH: f32 = 240.0;
const CLEAR_WIDTH: f32 = FIELD_HEIGHT;
/// Browse, and the gap between it and the folder field.
const BROWSE_WIDTH: f32 = 84.0;
const BROWSE_GAP: f32 = 8.0;
/// Each of System, Light and Dark.
const SEGMENT_WIDTH: f32 = 80.0;
/// A window background's tile, its picture and the gap between tiles, and its name under the
/// picture.
const TILE_WIDTH: f32 = 96.0;
const TILE_HEIGHT: f32 = 64.0;
const TILE_GAP: f32 = 12.0;
const NAME_GAP: f32 = 4.0;
const NAME_HEIGHT: f32 = 20.0;
/// The switch, and the gap between it and its state.
const TOGGLE_WIDTH: f32 = 40.0;
const TOGGLE_HEIGHT: f32 = 20.0;
const STATE_GAP: f32 = 12.0;
/// Between the last row's gap and the version's line, and that line.
const ABOVE_VERSION: f32 = 8.0;
const VERSION_HEIGHT: f32 = LINE_HEIGHT;
/// Below the version's line.
const BOTTOM: f32 = 12.0;

/// The rectangle with these edges.
fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

/// A control the keyboard can be on, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Hotkey(Mode),
    Folder,
    Browse,
    Theme,
    /// The window background's tiles.
    Background,
    StartWithWindows,
}

/// Every stop, in the order Tab moves through them.
pub const STOPS: [Stop; 9] = [
    Stop::Hotkey(Mode::Capture),
    Stop::Hotkey(Mode::Screen),
    Stop::Hotkey(Mode::Window),
    Stop::Hotkey(Mode::CopyText),
    Stop::Folder,
    Stop::Browse,
    Stop::Theme,
    Stop::Background,
    Stop::StartWithWindows,
];

/// Where Tab goes from `from`, or Shift+Tab when `back`, round from the last stop to the first.
/// From no stop, the first, or the last going back.
pub fn next(from: Option<Stop>, back: bool) -> Stop {
    let count = STOPS.len();
    let at = from.and_then(|from| STOPS.iter().position(|stop| *stop == from));
    let index = match (at, back) {
        (None, false) => 0,
        (None, true) => count - 1,
        (Some(at), false) => (at + 1) % count,
        (Some(at), true) => (at + count - 1) % count,
    };
    STOPS[index]
}

/// What Space does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// Nothing of the window's: the folder field takes it as it takes any key.
    Typed,
    /// The hotkey field listens for a combination, as clicking it does.
    Listen,
    /// Opens the folder picker, as clicking Browse does.
    Browse,
    /// Chooses this theme.
    Theme(Theme),
    /// Chooses this window background.
    Background(Background),
    /// Turns Start with Windows on or off.
    StartWithWindows(bool),
}

/// What Space does on `stop`, with `theme` and `background` chosen and Start with Windows `on` or
/// not: the theme and the window background each move to the next, round from the last to the
/// first, and the toggle flips.
pub fn space(stop: Stop, theme: Theme, background: Background, on: bool) -> Space {
    match stop {
        Stop::Hotkey(_) => Space::Listen,
        Stop::Folder => Space::Typed,
        Stop::Browse => Space::Browse,
        Stop::Theme => {
            let at = Theme::ALL.iter().position(|t| *t == theme).unwrap_or(0);
            Space::Theme(Theme::ALL[(at + 1) % Theme::ALL.len()])
        }
        Stop::Background => {
            let all = Background::ALL;
            let at = all.iter().position(|b| *b == background).unwrap_or(0);
            Space::Background(all[(at + 1) % all.len()])
        }
        Stop::StartWithWindows => Space::StartWithWindows(!on),
    }
}

/// What a press in the window can land on that the window draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Close,
    /// A mode's hotkey field, which listens for a combination.
    Hotkey(Mode),
    /// The clear control at the right end of a mode's hotkey field, which sets it to None.
    Clear(Mode),
    Browse,
    Theme(Theme),
    /// A window background's tile, its picture or its name.
    Background(Background),
    StartWithWindows,
}

/// The window laid out at one scale, in physical pixels of its client area.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    /// The title row, which moves the window, with Close at its right end.
    pub title: D2D_RECT_F,
    pub close: D2D_RECT_F,
    /// Hotkeys, Saving, Appearance and Startup.
    pub headings: [D2D_RECT_F; 4],
    /// Each row's name: Capture, Screen, Window, Copy text, Save folder, Theme, Window
    /// background and Start with Windows.
    pub labels: [D2D_RECT_F; 8],
    /// One hotkey field per mode in `Mode::ALL`, and the clear control inside its right end.
    pub hotkeys: [D2D_RECT_F; 4],
    pub clears: [D2D_RECT_F; 4],
    pub folder: D2D_RECT_F,
    pub browse: D2D_RECT_F,
    /// Under each hotkey field and the folder field, as many lines tall as its message.
    pub messages: [D2D_RECT_F; 5],
    /// System, Light and Dark, side by side.
    pub themes: [D2D_RECT_F; 3],
    /// One picture per window background in `Background::ALL`, side by side, and its name under
    /// it.
    pub tiles: [D2D_RECT_F; Background::ALL.len()],
    pub tile_names: [D2D_RECT_F; Background::ALL.len()],
    /// The switch, and where "On" or "Off" goes beside it.
    pub toggle: D2D_RECT_F,
    pub toggle_state: D2D_RECT_F,
    /// The line at the bottom that says which version this is, the window's whole width, so
    /// the version centres in the window.
    pub version: D2D_RECT_F,
}

impl Layout {
    /// The window at `scale`, physical pixels per logical pixel, with each message `lines` lines
    /// long, in the order of `messages`. An empty message keeps its one line, so a message
    /// appearing moves nothing; a longer one moves everything below it down.
    pub fn new(scale: f32, lines: [u32; 5]) -> Layout {
        // Each length is rounded on its own, so like parts are the same size wherever they are.
        let px = |logical: f32| (logical * scale).round();
        let width = px(WIDTH);
        let (left, right, field) = (px(MARGIN), width - px(MARGIN), px(FIELD_LEFT));
        let heading = |y: &mut f32| {
            let top = *y + px(ABOVE_HEADING);
            *y = top + px(HEADING_HEIGHT) + px(BELOW_HEADING);
            rect(left, top, right, top + px(HEADING_HEIGHT))
        };
        // A row of controls from `y` down, as wide as the column the controls are in.
        let row = |y: &mut f32| {
            let row = rect(field, *y, right, *y + px(FIELD_HEIGHT));
            *y = row.bottom + px(ROW_GAP);
            row
        };
        // A row with room under it for a message `lines` lines long.
        let field_row = |y: &mut f32, lines: u32| {
            let row = rect(field, *y, right, *y + px(FIELD_HEIGHT));
            let top = row.bottom + px(MESSAGE_GAP);
            let message = rect(
                field,
                top,
                right,
                top + lines.max(1) as f32 * px(LINE_HEIGHT),
            );
            *y = message.bottom + px(ROW_GAP);
            (row, message)
        };

        // The top of what comes next, moved down past each part as it is placed.
        let mut y = px(TITLE_HEIGHT);
        let hotkeys_heading = heading(&mut y);
        let hotkey_rows =
            [lines[0], lines[1], lines[2], lines[3]].map(|lines| field_row(&mut y, lines));
        let saving = heading(&mut y);
        let (folder_row, folder_message) = field_row(&mut y, lines[4]);
        let appearance = heading(&mut y);
        let theme_row = row(&mut y);
        let tile_row = rect(field, y, right, y + px(TILE_HEIGHT));
        let names_top = tile_row.bottom + px(NAME_GAP);
        y = names_top + px(NAME_HEIGHT) + px(ROW_GAP);
        // Each tile's left edge, side by side from the controls' column.
        let tiles: [f32; Background::ALL.len()] =
            std::array::from_fn(|at| field + at as f32 * (px(TILE_WIDTH) + px(TILE_GAP)));
        let startup = heading(&mut y);
        let toggle_row = row(&mut y);
        let version_top = y + px(ABOVE_VERSION);
        let version = rect(0.0, version_top, width, version_top + px(VERSION_HEIGHT));
        y = version.bottom;

        let browse = D2D_RECT_F {
            left: right - px(BROWSE_WIDTH),
            ..folder_row
        };
        let segment = px(SEGMENT_WIDTH);
        let toggle_top = toggle_row.top + px((FIELD_HEIGHT - TOGGLE_HEIGHT) / 2.0);
        let toggle = rect(
            field,
            toggle_top,
            field + px(TOGGLE_WIDTH),
            toggle_top + px(TOGGLE_HEIGHT),
        );
        let rows = [
            hotkey_rows[0].0,
            hotkey_rows[1].0,
            hotkey_rows[2].0,
            hotkey_rows[3].0,
            folder_row,
            theme_row,
            tile_row,
            toggle_row,
        ];
        let hotkeys = hotkey_rows.map(|(row, _)| D2D_RECT_F {
            right: field + px(HOTKEY_WIDTH),
            ..row
        });
        Layout {
            width: width as i32,
            height: (y + px(BOTTOM)) as i32,
            title: rect(left, 0.0, width - px(CLOSE_WIDTH), px(TITLE_HEIGHT)),
            close: rect(width - px(CLOSE_WIDTH), 0.0, width, px(TITLE_HEIGHT)),
            headings: [hotkeys_heading, saving, appearance, startup],
            labels: rows.map(|row| rect(left, row.top, field, row.bottom)),
            hotkeys,
            clears: hotkeys.map(|hotkey| D2D_RECT_F {
                left: hotkey.right - px(CLEAR_WIDTH),
                ..hotkey
            }),
            folder: D2D_RECT_F {
                right: browse.left - px(BROWSE_GAP),
                ..folder_row
            },
            browse,
            messages: [
                hotkey_rows[0].1,
                hotkey_rows[1].1,
                hotkey_rows[2].1,
                hotkey_rows[3].1,
                folder_message,
            ],
            themes: [0.0, 1.0, 2.0].map(|at| D2D_RECT_F {
                left: field + at * segment,
                right: field + (at + 1.0) * segment,
                ..theme_row
            }),
            tiles: tiles.map(|left| D2D_RECT_F {
                left,
                right: left + px(TILE_WIDTH),
                ..tile_row
            }),
            tile_names: tiles.map(|left| {
                rect(
                    left,
                    names_top,
                    left + px(TILE_WIDTH),
                    names_top + px(NAME_HEIGHT),
                )
            }),
            toggle,
            toggle_state: D2D_RECT_F {
                left: toggle.right + px(STATE_GAP),
                ..toggle_row
            },
            version,
        }
    }

    /// What the window draws at `at`, a client point, if anything a press acts on.
    pub fn hit(&self, at: (f32, f32)) -> Option<Target> {
        let inside = |rect: &D2D_RECT_F| {
            at.0 >= rect.left && at.0 < rect.right && at.1 >= rect.top && at.1 < rect.bottom
        };
        if inside(&self.close) {
            return Some(Target::Close);
        }
        for mode in Mode::ALL {
            if inside(&self.clears[mode as usize]) {
                return Some(Target::Clear(mode));
            }
            if inside(&self.hotkeys[mode as usize]) {
                return Some(Target::Hotkey(mode));
            }
        }
        if inside(&self.browse) {
            return Some(Target::Browse);
        }
        if let Some(at) = self.themes.iter().position(inside) {
            return Some(Target::Theme(Theme::ALL[at]));
        }
        let tile = |at: usize| D2D_RECT_F {
            bottom: self.tile_names[at].bottom,
            ..self.tiles[at]
        };
        if let Some(at) = (0..self.tiles.len()).position(|at| inside(&tile(at))) {
            return Some(Target::Background(Background::ALL[at]));
        }
        // The whole height of its row, as its state beside it is, so the switch is easy to hit.
        let switch = D2D_RECT_F {
            top: self.toggle_state.top,
            bottom: self.toggle_state.bottom,
            ..self.toggle
        };
        inside(&switch).then_some(Target::StartWithWindows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(rect: D2D_RECT_F) -> [f32; 4] {
        [rect.left, rect.top, rect.right, rect.bottom]
    }

    fn all(rects: &[D2D_RECT_F]) -> Vec<[f32; 4]> {
        rects.iter().copied().map(edges).collect()
    }

    #[test]
    fn at_100_percent_the_title_row_heads_four_groups_in_one_column() {
        let layout = Layout::new(1.0, [1; 5]);
        assert_eq!((layout.width, layout.height), (520, 728));
        assert_eq!(edges(layout.title), [16.0, 0.0, 474.0, 40.0]);
        assert_eq!(edges(layout.close), [474.0, 0.0, 520.0, 40.0]);
        // 16 above each heading and 8 below it.
        assert_eq!(
            all(&layout.headings),
            [
                [16.0, 56.0, 504.0, 76.0],
                [16.0, 340.0, 504.0, 360.0],
                [16.0, 444.0, 504.0, 464.0],
                [16.0, 624.0, 504.0, 644.0],
            ]
        );
    }

    #[test]
    fn the_version_line_is_the_window_s_whole_width_under_the_last_row() {
        let layout = Layout::new(1.0, [1; 5]);
        // 16 below the toggle's row, 12 above the bottom.
        assert_eq!(edges(layout.version), [0.0, 700.0, 520.0, 716.0]);
        let large = Layout::new(1.5, [1; 5]);
        assert_eq!(edges(large.version), [0.0, 1050.0, 780.0, 1074.0]);
        // A longer message moves it down with everything else.
        let moved = Layout::new(1.0, [1, 3, 1, 1, 1]);
        assert_eq!(edges(moved.version), [0.0, 732.0, 520.0, 748.0]);
    }

    #[test]
    fn each_row_is_a_label_beside_its_control_with_its_message_under_the_field() {
        let layout = Layout::new(1.0, [1; 5]);
        assert_eq!(
            all(&layout.labels),
            [
                [16.0, 84.0, 144.0, 116.0],
                [16.0, 144.0, 144.0, 176.0],
                [16.0, 204.0, 144.0, 236.0],
                [16.0, 264.0, 144.0, 296.0],
                [16.0, 368.0, 144.0, 400.0],
                [16.0, 472.0, 144.0, 504.0],
                [16.0, 512.0, 144.0, 576.0],
                [16.0, 652.0, 144.0, 684.0],
            ]
        );
        assert_eq!(
            all(&layout.hotkeys),
            [
                [144.0, 84.0, 384.0, 116.0],
                [144.0, 144.0, 384.0, 176.0],
                [144.0, 204.0, 384.0, 236.0],
                [144.0, 264.0, 384.0, 296.0],
            ]
        );
        // The folder field and Browse share the row, 8 apart.
        assert_eq!(edges(layout.folder), [144.0, 368.0, 412.0, 400.0]);
        assert_eq!(edges(layout.browse), [420.0, 368.0, 504.0, 400.0]);
        // 4 below each field, one 16-pixel line, to the right margin.
        assert_eq!(
            all(&layout.messages),
            [
                [144.0, 120.0, 504.0, 136.0],
                [144.0, 180.0, 504.0, 196.0],
                [144.0, 240.0, 504.0, 256.0],
                [144.0, 300.0, 504.0, 316.0],
                [144.0, 404.0, 504.0, 420.0],
            ]
        );
        assert_eq!(
            all(&layout.themes),
            [
                [144.0, 472.0, 224.0, 504.0],
                [224.0, 472.0, 304.0, 504.0],
                [304.0, 472.0, 384.0, 504.0],
            ]
        );
        // A 40 by 20 switch in the middle of its row, its state 12 after it.
        assert_eq!(edges(layout.toggle), [144.0, 658.0, 184.0, 678.0]);
        assert_eq!(edges(layout.toggle_state), [196.0, 652.0, 504.0, 684.0]);
    }

    #[test]
    fn the_window_background_is_a_row_of_tiles_under_the_theme_each_named_beneath() {
        let layout = Layout::new(1.0, [1; 5]);
        // 96 by 64, 12 apart, 8 below the theme's row; the label beside the pictures.
        assert_eq!(
            all(&layout.tiles),
            [
                [144.0, 512.0, 240.0, 576.0],
                [252.0, 512.0, 348.0, 576.0],
                [360.0, 512.0, 456.0, 576.0]
            ]
        );
        assert_eq!(
            all(&layout.tile_names),
            [
                [144.0, 580.0, 240.0, 600.0],
                [252.0, 580.0, 348.0, 600.0],
                [360.0, 580.0, 456.0, 600.0]
            ]
        );
        // Inside the margin.
        assert!(layout.tiles[2].right <= 504.0);
        let large = Layout::new(1.5, [1; 5]);
        assert_eq!(edges(large.tiles[2]), [540.0, 768.0, 684.0, 864.0]);
    }

    #[test]
    fn at_150_percent_every_length_is_half_as_long_again() {
        let layout = Layout::new(1.5, [1; 5]);
        assert_eq!((layout.width, layout.height), (780, 1092));
        assert_eq!(edges(layout.close), [711.0, 0.0, 780.0, 60.0]);
        assert_eq!(edges(layout.folder), [216.0, 552.0, 618.0, 600.0]);
        assert_eq!(edges(layout.browse), [630.0, 552.0, 756.0, 600.0]);
        assert_eq!(edges(layout.toggle), [216.0, 987.0, 276.0, 1017.0]);
    }

    #[test]
    fn a_longer_message_moves_everything_below_it_down_and_the_window_grows() {
        // The Screen field's message takes three lines, 32 more than one.
        let layout = Layout::new(1.0, [1, 3, 1, 1, 1]);
        assert_eq!(layout.height, 728 + 32);
        assert_eq!(edges(layout.hotkeys[1]), [144.0, 144.0, 384.0, 176.0]);
        assert_eq!(edges(layout.messages[1]), [144.0, 180.0, 504.0, 228.0]);
        assert_eq!(edges(layout.hotkeys[2]), [144.0, 236.0, 384.0, 268.0]);
        assert_eq!(edges(layout.hotkeys[3]), [144.0, 296.0, 384.0, 328.0]);
        assert_eq!(edges(layout.folder), [144.0, 400.0, 412.0, 432.0]);
        // An empty message keeps its line.
        assert_eq!(Layout::new(1.0, [0; 5]), Layout::new(1.0, [1; 5]));
    }

    #[test]
    fn a_press_lands_on_close_browse_a_theme_a_tile_or_the_switch() {
        let layout = Layout::new(1.0, [1; 5]);
        assert_eq!(layout.hit((500.0, 20.0)), Some(Target::Close));
        assert_eq!(layout.hit((420.0, 368.0)), Some(Target::Browse));
        assert_eq!(
            layout.hit((150.0, 480.0)),
            Some(Target::Theme(Theme::System))
        );
        assert_eq!(
            layout.hit((224.0, 480.0)),
            Some(Target::Theme(Theme::Light))
        );
        assert_eq!(layout.hit((383.0, 503.0)), Some(Target::Theme(Theme::Dark)));
        // A tile's picture or its name.
        assert_eq!(
            layout.hit((144.0, 512.0)),
            Some(Target::Background(Background::TrimShadow))
        );
        assert_eq!(
            layout.hit((347.0, 599.0)),
            Some(Target::Background(Background::Shadow))
        );
        assert_eq!(
            layout.hit((400.0, 540.0)),
            Some(Target::Background(Background::Wallpaper))
        );
        assert_eq!(layout.hit((160.0, 668.0)), Some(Target::StartWithWindows));
        // The title, a label, a field and the space between rows.
        assert_eq!(layout.hit((100.0, 20.0)), None);
        assert_eq!(layout.hit((50.0, 100.0)), None);
        assert_eq!(layout.hit((200.0, 380.0)), None);
        assert_eq!(layout.hit((384.0, 480.0)), None);
        assert_eq!(layout.hit((245.0, 540.0)), None);
        assert_eq!(layout.hit((300.0, 604.0)), None);
    }

    #[test]
    fn each_hotkey_field_ends_in_its_clear_control_and_a_press_lands_on_either() {
        let layout = Layout::new(1.0, [1; 5]);
        // A field's height square at its right end.
        assert_eq!(
            all(&layout.clears),
            [
                [352.0, 84.0, 384.0, 116.0],
                [352.0, 144.0, 384.0, 176.0],
                [352.0, 204.0, 384.0, 236.0],
                [352.0, 264.0, 384.0, 296.0],
            ]
        );
        assert_eq!(
            layout.hit((144.0, 84.0)),
            Some(Target::Hotkey(Mode::Capture))
        );
        assert_eq!(
            layout.hit((351.0, 175.0)),
            Some(Target::Hotkey(Mode::Screen))
        );
        assert_eq!(
            layout.hit((352.0, 175.0)),
            Some(Target::Clear(Mode::Screen))
        );
        assert_eq!(
            layout.hit((383.0, 235.0)),
            Some(Target::Clear(Mode::Window))
        );
        assert_eq!(
            layout.hit((144.0, 264.0)),
            Some(Target::Hotkey(Mode::CopyText))
        );
        assert_eq!(
            layout.hit((383.0, 295.0)),
            Some(Target::Clear(Mode::CopyText))
        );
        // Beside a field, under it and between two.
        assert_eq!(layout.hit((384.0, 100.0)), None);
        assert_eq!(layout.hit((200.0, 120.0)), None);
        assert_eq!(layout.hit((200.0, 140.0)), None);
        let large = Layout::new(1.5, [1; 5]);
        assert_eq!(edges(large.clears[0]), [528.0, 126.0, 576.0, 174.0]);
    }

    #[test]
    fn tab_goes_from_each_hotkey_field_to_the_folder_browse_the_theme_the_tiles_and_the_toggle() {
        let mut at = next(None, false);
        let mut visited = vec![at];
        for _ in 0..9 {
            at = next(Some(at), false);
            visited.push(at);
        }
        assert_eq!(
            visited,
            [
                Stop::Hotkey(Mode::Capture),
                Stop::Hotkey(Mode::Screen),
                Stop::Hotkey(Mode::Window),
                Stop::Hotkey(Mode::CopyText),
                Stop::Folder,
                Stop::Browse,
                Stop::Theme,
                Stop::Background,
                Stop::StartWithWindows,
                Stop::Hotkey(Mode::Capture),
            ]
        );
    }

    #[test]
    fn shift_tab_goes_back_the_same_way() {
        assert_eq!(next(None, true), Stop::StartWithWindows);
        assert_eq!(next(Some(Stop::Folder), true), Stop::Hotkey(Mode::CopyText));
        assert_eq!(next(Some(Stop::Theme), true), Stop::Browse);
        assert_eq!(next(Some(Stop::StartWithWindows), true), Stop::Background);
        assert_eq!(
            next(Some(Stop::Hotkey(Mode::Capture)), true),
            Stop::StartWithWindows
        );
    }

    /// What Space does on `stop` with Trim shadow chosen.
    fn space_at(stop: Stop, theme: Theme, on: bool) -> Space {
        space(stop, theme, Background::TrimShadow, on)
    }

    #[test]
    fn space_moves_the_window_background_on_round_to_the_first() {
        assert_eq!(
            space(Stop::Background, Theme::Dark, Background::TrimShadow, false),
            Space::Background(Background::Shadow)
        );
        assert_eq!(
            space(Stop::Background, Theme::Dark, Background::Shadow, false),
            Space::Background(Background::Wallpaper)
        );
        assert_eq!(
            space(Stop::Background, Theme::Dark, Background::Wallpaper, false),
            Space::Background(Background::TrimShadow)
        );
    }

    #[test]
    fn space_presses_browse_moves_the_theme_on_flips_the_toggle_and_listens_in_a_hotkey_field() {
        assert_eq!(space_at(Stop::Browse, Theme::System, false), Space::Browse);
        assert_eq!(
            space_at(Stop::Theme, Theme::System, false),
            Space::Theme(Theme::Light)
        );
        assert_eq!(
            space_at(Stop::Theme, Theme::Light, false),
            Space::Theme(Theme::Dark)
        );
        assert_eq!(
            space_at(Stop::Theme, Theme::Dark, false),
            Space::Theme(Theme::System)
        );
        assert_eq!(
            space_at(Stop::StartWithWindows, Theme::Dark, false),
            Space::StartWithWindows(true)
        );
        assert_eq!(
            space_at(Stop::StartWithWindows, Theme::Dark, true),
            Space::StartWithWindows(false)
        );
        assert_eq!(space_at(Stop::Folder, Theme::Dark, true), Space::Typed);
        for mode in Mode::ALL {
            assert_eq!(
                space_at(Stop::Hotkey(mode), Theme::Light, true),
                Space::Listen
            );
        }
    }
}
