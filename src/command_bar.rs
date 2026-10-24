//! The editor's command bar laid out at one DPI: where each of its buttons, its dividers, the
//! contextual group, the room for the status, the Dimensions readout and the tools' flyouts go,
//! in physical client pixels, and what the contextual group holds.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::{
    flyout::{self, Flyout, StyleFlyout},
    image::{Image, Resize},
    style::{Colour, Fill, Head},
};

/// A button on the command bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Select,
    Arrow,
    Rect,
    Pen,
    Text,
    Counter,
    Blur,
    Picker,
    Crop,
    /// Toggles between Fit and 100 percent, and reads the zoom level.
    Zoom,
    /// The Dimensions readout, which opens the flyout of Resizes under it.
    Dimensions,
    Undo,
    Copy,
    Save,
    /// Leaves the capture on the screen as a bare picture above every other window.
    Pin,
    Close,
    /// A swatch in a style flyout.
    Colour(Colour),
    /// The slider after the swatches in Arrow's, Rect's, Pen's and Counter's flyouts.
    Size,
    /// One of the three buttons after the Size slider in Arrow's flyout.
    Head(Head),
    /// One of the three buttons after the Size slider in Rect's flyout.
    Fill(Fill),
    /// The slider in the Blur flyout, for a blur.
    Level,
    /// One of the three buttons in the Dimensions flyout.
    Resize(Resize),
    /// Apply and Cancel in the contextual group, for the crop being chosen.
    Apply,
    Cancel,
}

impl Command {
    /// Its Segoe Fluent Icons glyph, with what shows instead on a system without one; the zoom
    /// level, the Dimensions readout, the Resizes, the swatches, Apply and Cancel have none, and
    /// nor do Size and Level. Counter's, each Head's and each Fill's icons are drawn instead.
    pub fn icon(self) -> Option<(char, &'static str)> {
        Some(match self {
            Command::Select => ('\u{E8B0}', "S"),
            Command::Arrow => ('\u{E8AD}', "A"),
            Command::Rect => ('\u{E739}', "R"),
            Command::Pen => ('\u{EE56}', "P"),
            Command::Text => ('\u{E8D2}', "T"),
            Command::Blur => ('\u{E727}', "B"),
            Command::Picker => ('\u{EF3C}', "I"),
            Command::Crop => ('\u{E7A8}', "C"),
            Command::Undo => ('\u{E7A7}', "Undo"),
            Command::Copy => ('\u{E8C8}', "Copy"),
            Command::Save => ('\u{E74E}', "Save"),
            Command::Pin => ('\u{E718}', "Pin"),
            Command::Close => ('\u{E8BB}', "\u{00D7}"),
            Command::Counter
            | Command::Head(_)
            | Command::Fill(_)
            | Command::Zoom
            | Command::Dimensions
            | Command::Resize(_)
            | Command::Colour(_)
            | Command::Size
            | Command::Level
            | Command::Apply
            | Command::Cancel => return None,
        })
    }

    /// What its tooltip reads: its name and its key.
    pub fn tip(self) -> &'static str {
        match self {
            Command::Select => "Select · S",
            Command::Arrow => "Arrow · A",
            Command::Rect => "Rect · R",
            Command::Pen => "Pen · P",
            Command::Text => "Text · T",
            Command::Counter => "Counter · N",
            Command::Blur => "Blur · B",
            Command::Picker => "Picker · I",
            Command::Crop => "Crop · C",
            Command::Zoom => "Fit or 100% · Ctrl+0 · Ctrl+1",
            Command::Dimensions => "Resize",
            Command::Resize(resize) => match resize {
                Resize::Quarter => "Resize to a quarter",
                Resize::Third => "Resize to a third",
                Resize::Half => "Resize to half",
            },
            Command::Undo => "Undo · Ctrl+Z · Ctrl+Y redoes",
            Command::Copy => "Copy · Ctrl+C",
            Command::Save => "Save · Ctrl+S · Right-click for Save As",
            Command::Pin => "Pin · Ctrl+P",
            Command::Close => "Close · Esc",
            Command::Colour(colour) => match colour {
                Colour::Red => "Red · 1",
                Colour::Orange => "Orange · 2",
                Colour::Yellow => "Yellow · 3",
                Colour::Green => "Green · 4",
                Colour::Blue => "Blue · 5",
                Colour::Purple => "Purple · 6",
                Colour::White => "White · 7",
                Colour::Black => "Black · 8",
            },
            Command::Size => "Size",
            Command::Head(head) => match head {
                Head::One => "One head",
                Head::Both => "Both heads",
                Head::None => "No head",
            },
            Command::Fill(fill) => match fill {
                Fill::Outline => "Outline",
                Fill::Tint => "Tint",
                Fill::Solid => "Solid",
            },
            Command::Level => "Level",
            Command::Apply => "Apply · Enter",
            Command::Cancel => "Cancel · Esc",
        }
    }
}

/// Measurements in logical pixels.
const HEIGHT: f32 = 40.0;
const BUTTON_WIDTH: f32 = 36.0;
const BUTTON_HEIGHT: f32 = 32.0;
const CLOSE_WIDTH: f32 = 46.0;
/// Between the window's edges and the buttons, and between groups.
const EDGE: f32 = 4.0;
/// Between the buttons of a group.
const GAP: f32 = 2.0;
/// Either side of a divider, and its height.
const DIVIDER_MARGIN: f32 = 4.0;
const DIVIDER_HEIGHT: f32 = 16.0;
/// Apply and Cancel: their height, either side of their label, and between the crop's readout
/// and Apply.
const TEXT_BUTTON_HEIGHT: f32 = 24.0;
const TEXT_PADDING: f32 = 10.0;
const READOUT_GAP: f32 = 8.0;
/// Between the tools or the contextual group and the room for the status.
const STATUS_GAP: f32 = 8.0;
/// Either side of the status, for its highlight when it can be clicked.
pub const STATUS_PADDING: f32 = 6.0;
/// Either side of the zoom level, and of the Dimensions readout.
const ZOOM_PADDING: f32 = 8.0;
const DIMENSIONS_PADDING: f32 = ZOOM_PADDING;

/// The tools, left to right.
const TOOLS: [Command; 9] = [
    Command::Select,
    Command::Arrow,
    Command::Rect,
    Command::Pen,
    Command::Text,
    Command::Counter,
    Command::Blur,
    Command::Picker,
    Command::Crop,
];

/// What the contextual group after the tools holds. Every tool's style is in its flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Empty,
    /// The width and height of the crop being chosen, in capture pixels, read out with Apply and
    /// Cancel.
    Crop {
        width: u32,
        height: u32,
    },
}

impl Group {
    /// The group with `crop` the width and height of the crop being chosen, if one is.
    pub fn new(crop: Option<(u32, u32)>) -> Group {
        match crop {
            Some((width, height)) => Group::Crop { width, height },
            None => Group::Empty,
        }
    }

    /// The crop's readout, if the group shows one.
    pub fn readout(self) -> Option<String> {
        match self {
            Group::Crop { width, height } => Some(readout(width, height)),
            _ => None,
        }
    }
}

/// A width and height as the crop's readout and the Dimensions readout read them out.
pub fn readout(width: u32, height: u32) -> String {
    format!("{width} \u{00D7} {height}")
}

/// What the Dimensions readout reads for `image`, the capture as it is now: its width and height
/// in capture pixels, after any crop and with any Shadow or Wallpaper margin.
pub fn dimensions(image: &Image) -> String {
    readout(image.width, image.height)
}

/// How wide the bar's texts are at its scale, in physical pixels: the widest zoom level, the
/// widest status Zilum names itself, the widest crop or Dimensions readout, and Apply and Cancel.
pub struct Widths {
    pub zoom: f32,
    pub status: f32,
    pub readout: f32,
    pub apply: f32,
    pub cancel: f32,
}

/// What the contextual group draws for one of the things it holds.
#[derive(Default)]
pub struct Parts {
    pub dividers: Vec<D2D_RECT_F>,
    /// Its buttons, left to right.
    pub buttons: Vec<(Command, D2D_RECT_F)>,
    /// Where the crop's readout goes, left-aligned, as tall as a button.
    pub readout: Option<D2D_RECT_F>,
}

/// The command bar at one scale and width.
pub struct Layout {
    pub height: i32,
    /// Every button but the contextual group's, left to right.
    pub buttons: Vec<(Command, D2D_RECT_F)>,
    /// The hairlines between groups.
    pub dividers: Vec<D2D_RECT_F>,
    /// The contextual group empty and with a crop's readout.
    empty: Parts,
    crop: Parts,
    /// The Blur flyout, under Blur.
    pub flyout: Flyout,
    /// The style flyouts, each under its tool.
    style_flyouts: Vec<(Command, StyleFlyout)>,
    /// The room for the status and the Dimensions readout together with the group empty, as tall
    /// as a button: from just after the tools to the zoom level.
    status: D2D_RECT_F,
    /// Between the contextual group and the status, in physical pixels.
    status_gap: f32,
    /// Either side of the Dimensions readout, in physical pixels.
    dimensions_padding: f32,
    /// The narrowest bar that holds everything, with the widest status Zilum names itself, the
    /// widest Dimensions and the group empty.
    pub min_width: i32,
}

impl Layout {
    /// The bar at `scale`, physical pixels per logical pixel, in a window `width` physical pixels
    /// wide, or as wide as the bar needs if that is wider, with its texts as wide as `widths`.
    pub fn new(scale: f32, width: i32, widths: &Widths) -> Layout {
        // Each size and gap is rounded on its own, so every button is as wide as every other.
        let px = |logical: f32| (logical * scale).round();
        let height = px(HEIGHT);
        let top = px((HEIGHT - BUTTON_HEIGHT) / 2.0);
        let bottom = top + px(BUTTON_HEIGHT);
        let row = |left: f32, width: f32| D2D_RECT_F {
            left,
            top,
            right: left + width,
            bottom,
        };
        let divider_space = 2.0 * px(DIVIDER_MARGIN) + 1.0;
        // The divider in the space starting at `left`.
        let divider = |left: f32| D2D_RECT_F {
            left: left + px(DIVIDER_MARGIN),
            top: px((HEIGHT - DIVIDER_HEIGHT) / 2.0),
            right: left + px(DIVIDER_MARGIN) + 1.0,
            bottom: px((HEIGHT - DIVIDER_HEIGHT) / 2.0) + px(DIVIDER_HEIGHT),
        };

        // Copy and Save lead, then a divider and the tools.
        let copy = row(px(EDGE), px(BUTTON_WIDTH));
        let save = row(copy.right + px(GAP), px(BUTTON_WIDTH));
        let mut buttons = vec![(Command::Copy, copy), (Command::Save, save)];
        // The right edge of what is before the next tool, less the gap the loop puts before it.
        let mut right = save.right + divider_space - px(GAP);
        let mut flyout = Flyout::under(D2D_RECT_F::default(), scale);
        let mut style_flyouts = Vec::new();
        for command in TOOLS {
            let button = row(right + px(GAP), px(BUTTON_WIDTH));
            right = button.right;
            buttons.push((command, button));
            // The tools are left of the status, so they never move, and nor do their flyouts.
            if command == Command::Blur {
                flyout = Flyout::under(button, scale);
            }
            if flyout::STYLED.contains(&command) {
                style_flyouts.push((command, StyleFlyout::under(button, scale, command)));
            }
        }
        let status_left = right + px(STATUS_GAP);

        // The contextual group: a divider, the crop's readout, Apply and Cancel.
        let group_left = right + px(GAP);
        let readout = row(group_left + divider_space, widths.readout.ceil());
        let text_top = px((HEIGHT - TEXT_BUTTON_HEIGHT) / 2.0);
        let text_button = |left: f32, label: f32| D2D_RECT_F {
            left,
            top: text_top,
            right: left + label.ceil() + 2.0 * px(TEXT_PADDING),
            bottom: text_top + px(TEXT_BUTTON_HEIGHT),
        };
        let apply = text_button(readout.right + px(READOUT_GAP), widths.apply);
        let cancel = text_button(apply.right + px(EDGE), widths.cancel);
        let crop = Parts {
            dividers: vec![divider(group_left)],
            buttons: vec![(Command::Apply, apply), (Command::Cancel, cancel)],
            readout: Some(readout),
        };

        // Right to left from the right edge, as if it were at 0, then moved to the window's.
        let close = D2D_RECT_F {
            left: -px(CLOSE_WIDTH),
            top: 0.0,
            right: 0.0,
            bottom: height,
        };
        let pin = row(close.left - px(EDGE) - px(BUTTON_WIDTH), px(BUTTON_WIDTH));
        let undo = row(
            pin.left - divider_space - px(BUTTON_WIDTH),
            px(BUTTON_WIDTH),
        );
        let zoom_button = widths.zoom.ceil() + 2.0 * px(ZOOM_PADDING);
        let zoom = row(undo.left - px(EDGE) - zoom_button, zoom_button);
        let status_room = widths.status.ceil() + 2.0 * px(STATUS_PADDING);
        let dimensions_room = widths.readout.ceil() + 2.0 * px(DIMENSIONS_PADDING);
        let min_width = (status_left + status_room + dimensions_room - zoom.left) as i32;

        let at = width.max(min_width) as f32;
        let moved = |rect: D2D_RECT_F| D2D_RECT_F {
            left: rect.left + at,
            right: rect.right + at,
            ..rect
        };
        buttons.extend(
            [
                (Command::Zoom, zoom),
                (Command::Undo, undo),
                (Command::Pin, pin),
                (Command::Close, close),
            ]
            .map(|(command, rect)| (command, moved(rect))),
        );
        Layout {
            height: height as i32,
            buttons,
            dividers: vec![divider(save.right), moved(divider(undo.right))],
            empty: Parts::default(),
            crop,
            flyout,
            style_flyouts,
            status: D2D_RECT_F {
                left: status_left,
                right: zoom.left + at,
                top,
                bottom,
            },
            status_gap: px(STATUS_GAP),
            dimensions_padding: px(DIMENSIONS_PADDING),
            min_width,
        }
    }

    /// What the contextual group draws while it holds `group`.
    pub fn parts(&self, group: Group) -> &Parts {
        match group {
            Group::Empty => &self.empty,
            Group::Crop { .. } => &self.crop,
        }
    }

    /// Where the status goes with the contextual group holding `group` and the Dimensions
    /// readout's text `readout_width` wide: right-aligned against the readout, from 8 after the
    /// group, or after the tools while it is empty. Beside the group in the narrowest bar it has
    /// little room or none.
    pub fn status(&self, group: Group, readout_width: f32) -> D2D_RECT_F {
        let parts = self.parts(group);
        let left = parts
            .buttons
            .last()
            .map_or(self.status.left, |(_, rect)| rect.right + self.status_gap);
        D2D_RECT_F {
            left,
            right: self.dimensions(readout_width).left,
            ..self.status
        }
    }

    /// Where the Dimensions readout goes when its text is `readout_width` wide: right-aligned
    /// against the zoom level, with room either side as the zoom level has, as tall as a button.
    pub fn dimensions(&self, readout_width: f32) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.status.right - readout_width.ceil() - 2.0 * self.dimensions_padding,
            ..self.status
        }
    }

    /// The style flyout under `tool`'s button, if it has one.
    pub fn style_flyout(&self, tool: Command) -> Option<&StyleFlyout> {
        self.style_flyouts
            .iter()
            .find(|(under, _)| *under == tool)
            .map(|(_, row)| row)
    }

    /// The button at `x`, `y` with the contextual group holding `group`, if any.
    pub fn hit(&self, x: f32, y: f32, group: Group) -> Option<Command> {
        self.shown(group)
            .find(|(_, rect)| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
            .map(|(command, _)| *command)
    }

    /// Where `command`'s button is.
    pub fn rect(&self, command: Command) -> D2D_RECT_F {
        self.buttons
            .iter()
            .find(|(button, _)| *button == command)
            .map_or_else(D2D_RECT_F::default, |(_, rect)| *rect)
    }

    /// Where `command`'s button is with the contextual group holding `group`, if it shows one.
    pub fn shown_rect(&self, command: Command, group: Group) -> Option<D2D_RECT_F> {
        self.shown(group)
            .find(|(button, _)| *button == command)
            .map(|(_, rect)| *rect)
    }

    /// Every button shown with the contextual group holding `group`.
    fn shown(&self, group: Group) -> impl Iterator<Item = &(Command, D2D_RECT_F)> {
        self.buttons.iter().chain(&self.parts(group).buttons)
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::SYSTEMTIME;

    use super::*;
    use crate::image::{Image, Region};

    /// A crop being chosen.
    const CROP: Group = Group::Crop {
        width: 640,
        height: 360,
    };

    /// The widest zoom level 30 pixels wide, the widest status 200, the widest crop readout 80,
    /// Apply 32 and Cancel 40.
    const WIDTHS: Widths = Widths {
        zoom: 30.0,
        status: 200.0,
        readout: 80.0,
        apply: 32.0,
        cancel: 40.0,
    };
    /// The same texts at 150 percent.
    const WIDTHS_150: Widths = Widths {
        zoom: 45.0,
        status: 300.0,
        readout: 120.0,
        apply: 48.0,
        cancel: 60.0,
    };

    fn edges(rect: D2D_RECT_F) -> [f32; 4] {
        [rect.left, rect.top, rect.right, rect.bottom]
    }

    fn all_edges(rects: &[D2D_RECT_F]) -> Vec<[f32; 4]> {
        rects.iter().copied().map(edges).collect()
    }

    fn button_edges(buttons: &[(Command, D2D_RECT_F)]) -> Vec<(Command, [f32; 4])> {
        buttons
            .iter()
            .map(|(command, rect)| (*command, edges(*rect)))
            .collect()
    }

    #[test]
    fn with_the_group_empty_the_status_starts_8_after_the_tools() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        let empty = bar.parts(Group::Empty);
        assert!(empty.dividers.is_empty() && empty.buttons.is_empty() && empty.readout.is_none());
        // 8 after Crop, the widest status with 6 either side.
        assert_eq!(
            edges(bar.status(Group::Empty, WIDTHS.readout)),
            [435.0, 4.0, 647.0, 36.0]
        );
        let bar = Layout::new(1.5, 0, &WIDTHS_150);
        assert_eq!(
            edges(bar.status(Group::Empty, WIDTHS_150.readout)),
            [652.0, 6.0, 970.0, 54.0]
        );
    }

    #[test]
    fn a_crop_shows_a_divider_its_readout_and_apply_and_cancel_after_the_tools() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        let crop = bar.parts(CROP);
        assert_eq!(all_edges(&crop.dividers), [[433.0, 12.0, 434.0, 28.0]]);
        // Room for the widest readout, then 24-pixel text buttons with 10 either side of their
        // label, 8 after the readout and 4 apart.
        assert_eq!(crop.readout.map(edges), Some([438.0, 4.0, 518.0, 36.0]));
        assert_eq!(
            button_edges(&crop.buttons),
            [
                (Command::Apply, [526.0, 8.0, 578.0, 32.0]),
                (Command::Cancel, [582.0, 8.0, 642.0, 32.0]),
            ]
        );
        assert_eq!(
            edges(bar.status(CROP, WIDTHS.readout)),
            [650.0, 4.0, 647.0, 36.0]
        );
    }

    #[test]
    fn at_150_percent_the_group_is_scaled_and_rounded() {
        let bar = Layout::new(1.5, 0, &WIDTHS_150);
        let crop = bar.parts(CROP);
        assert_eq!(crop.readout.map(edges), Some([656.0, 6.0, 776.0, 54.0]));
        assert_eq!(
            button_edges(&crop.buttons),
            [
                (Command::Apply, [788.0, 12.0, 866.0, 48.0]),
                (Command::Cancel, [872.0, 12.0, 962.0, 48.0]),
            ]
        );
    }

    #[test]
    fn level_is_in_the_flyout_under_blur_wherever_the_right_edge_is() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        assert_eq!(bar.flyout, Flyout::under(bar.rect(Command::Blur), 1.0));
        assert_eq!(edges(bar.flyout.level), [323.0, 48.0, 443.0, 72.0]);
        assert_eq!(Layout::new(1.0, 1600, &WIDTHS).flyout, bar.flyout);
        let bar = Layout::new(1.5, 0, &WIDTHS_150);
        assert_eq!(edges(bar.flyout.level), [484.0, 72.0, 664.0, 108.0]);
    }

    #[test]
    fn arrow_rect_pen_text_and_counter_each_have_a_style_flyout_under_their_button() {
        for (scale, widths) in [(1.0, &WIDTHS), (1.5, &WIDTHS_150)] {
            let bar = Layout::new(scale, 0, widths);
            for tool in [
                Command::Arrow,
                Command::Rect,
                Command::Pen,
                Command::Text,
                Command::Counter,
            ] {
                assert_eq!(
                    bar.style_flyout(tool),
                    Some(&StyleFlyout::under(bar.rect(tool), scale, tool)),
                    "{tool:?}"
                );
            }
            for tool in [
                Command::Select,
                Command::Blur,
                Command::Picker,
                Command::Crop,
            ] {
                assert_eq!(bar.style_flyout(tool), None, "{tool:?}");
            }
        }
        // They never move with the right edge.
        let bar = Layout::new(1.0, 0, &WIDTHS);
        let wide = Layout::new(1.0, 1600, &WIDTHS);
        assert_eq!(
            bar.style_flyout(Command::Counter),
            wide.style_flyout(Command::Counter)
        );
        assert_eq!(
            edges(bar.style_flyout(Command::Counter).unwrap().panel),
            [277.0, 44.0, 628.0, 76.0]
        );
    }

    #[test]
    fn the_sliders_read_their_names() {
        assert_eq!(Command::Level.tip(), "Level");
        assert_eq!(Command::Level.icon(), None);
        assert_eq!(Command::Size.tip(), "Size");
        assert_eq!(Command::Size.icon(), None);
    }

    #[test]
    fn only_what_the_group_shows_can_be_clicked() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        // Just after the tools, where a crop's readout goes, and Apply at its centre.
        let (after_tools, on_apply) = ((450.0, 20.0), (552.0, 20.0));
        let hit = |group, (x, y)| bar.hit(x, y, group);

        assert_eq!(hit(Group::Empty, after_tools), None);
        assert_eq!(hit(Group::Empty, on_apply), None);
        assert_eq!(hit(CROP, on_apply), Some(Command::Apply));
        assert_eq!(hit(CROP, after_tools), None);
        // The bar's own buttons whatever the group.
        assert_eq!(hit(CROP, (20.0, 20.0)), Some(Command::Copy));
        assert_eq!(hit(CROP, (100.0, 20.0)), Some(Command::Select));
    }

    #[test]
    fn at_100_percent_every_button_has_its_place_left_to_right() {
        // The widest zoom level 30 pixels wide and the widest status 200.
        let bar = Layout::new(1.0, 0, &WIDTHS);
        assert_eq!(bar.height, 40);
        let order: Vec<Command> = bar.buttons.iter().map(|(command, _)| *command).collect();
        assert_eq!(
            order,
            [
                Command::Copy,
                Command::Save,
                Command::Select,
                Command::Arrow,
                Command::Rect,
                Command::Pen,
                Command::Text,
                Command::Counter,
                Command::Blur,
                Command::Picker,
                Command::Crop,
                Command::Zoom,
                Command::Undo,
                Command::Pin,
                Command::Close,
            ]
        );
        // 36 by 32 buttons, 2 apart, from 4 in: Copy and Save, a divider, then the tools.
        assert_eq!(edges(bar.rect(Command::Copy)), [4.0, 4.0, 40.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Save)), [42.0, 4.0, 78.0, 36.0]);
        assert_eq!(edges(bar.dividers[0]), [82.0, 12.0, 83.0, 28.0]);
        assert_eq!(edges(bar.rect(Command::Select)), [87.0, 4.0, 123.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Arrow)), [125.0, 4.0, 161.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Pen)), [201.0, 4.0, 237.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Counter)), [277.0, 4.0, 313.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Picker)), [353.0, 4.0, 389.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Crop)), [391.0, 4.0, 427.0, 36.0]);
        // The widest status with 6 either side, from 8 after Crop.
        assert_eq!(
            edges(bar.status(Group::Empty, WIDTHS.readout)),
            [435.0, 4.0, 647.0, 36.0]
        );
        // The widest Dimensions with 8 either side.
        assert_eq!(
            edges(bar.dimensions(WIDTHS.readout)),
            [647.0, 4.0, 743.0, 36.0]
        );
        // The widest level with 8 either side, then Undo, a divider and Pin.
        assert_eq!(edges(bar.rect(Command::Zoom)), [743.0, 4.0, 789.0, 36.0]);
        assert_eq!(edges(bar.rect(Command::Undo)), [793.0, 4.0, 829.0, 36.0]);
        assert_eq!(edges(bar.dividers[1]), [833.0, 12.0, 834.0, 28.0]);
        assert_eq!(edges(bar.rect(Command::Pin)), [838.0, 4.0, 874.0, 36.0]);
        // Close is 46 wide and the bar's full height, at its right end.
        assert_eq!(edges(bar.rect(Command::Close)), [878.0, 0.0, 924.0, 40.0]);
        // The divider after Save is 9 wide, and Pin is no longer 2 after Save.
        assert_eq!(bar.min_width, 924);
        assert_eq!(bar.dividers.len(), 2);
    }

    #[test]
    fn a_wider_window_moves_only_what_is_right_of_the_status() {
        let narrow = Layout::new(1.0, 924, &WIDTHS);
        let wide = Layout::new(1.0, 1124, &WIDTHS);
        assert_eq!(wide.min_width, 924);
        let status = narrow.status(Group::Empty, WIDTHS.readout);
        for (command, rect) in &narrow.buttons {
            let moved = wide.rect(*command);
            let by = if rect.left < status.left { 0.0 } else { 200.0 };
            assert_eq!(
                edges(moved),
                [rect.left + by, rect.top, rect.right + by, rect.bottom]
            );
        }
        // The divider after Save stays, and the one after Undo moves.
        assert_eq!(edges(wide.dividers[0]), edges(narrow.dividers[0]));
        assert_eq!(edges(wide.dividers[1]), [1033.0, 12.0, 1034.0, 28.0]);
        assert_eq!(
            edges(wide.status(Group::Empty, WIDTHS.readout)),
            [435.0, 4.0, 847.0, 36.0]
        );
        assert_eq!(
            edges(wide.dimensions(WIDTHS.readout)),
            [847.0, 4.0, 943.0, 36.0]
        );
        assert_eq!(
            edges(wide.rect(Command::Close)),
            [1078.0, 0.0, 1124.0, 40.0]
        );
    }

    #[test]
    fn a_window_narrower_than_the_bar_is_laid_out_as_wide_as_the_bar() {
        let bar = Layout::new(1.0, 500, &WIDTHS);
        assert_eq!(edges(bar.rect(Command::Close)), [878.0, 0.0, 924.0, 40.0]);
    }

    #[test]
    fn at_150_percent_each_size_and_gap_is_scaled_and_rounded() {
        let bar = Layout::new(1.5, 0, &WIDTHS_150);
        assert_eq!(bar.height, 60);
        assert_eq!(edges(bar.rect(Command::Copy)), [6.0, 6.0, 60.0, 54.0]);
        assert_eq!(edges(bar.rect(Command::Save)), [63.0, 6.0, 117.0, 54.0]);
        assert_eq!(edges(bar.dividers[0]), [123.0, 18.0, 124.0, 42.0]);
        assert_eq!(edges(bar.rect(Command::Select)), [130.0, 6.0, 184.0, 54.0]);
        assert_eq!(edges(bar.rect(Command::Arrow)), [187.0, 6.0, 241.0, 54.0]);
        assert_eq!(edges(bar.rect(Command::Crop)), [586.0, 6.0, 640.0, 54.0]);
        assert_eq!(
            edges(bar.status(Group::Empty, WIDTHS_150.readout)),
            [652.0, 6.0, 970.0, 54.0]
        );
        assert_eq!(
            edges(bar.dimensions(WIDTHS_150.readout)),
            [970.0, 6.0, 1114.0, 54.0]
        );
        assert_eq!(edges(bar.rect(Command::Zoom)), [1114.0, 6.0, 1183.0, 54.0]);
        assert_eq!(edges(bar.rect(Command::Undo)), [1189.0, 6.0, 1243.0, 54.0]);
        assert_eq!(edges(bar.dividers[1]), [1249.0, 18.0, 1250.0, 42.0]);
        assert_eq!(edges(bar.rect(Command::Pin)), [1256.0, 6.0, 1310.0, 54.0]);
        assert_eq!(edges(bar.rect(Command::Close)), [1316.0, 0.0, 1385.0, 60.0]);
        assert_eq!(bar.min_width, 1385);
    }

    #[test]
    fn each_fill_reads_its_name() {
        assert_eq!(Command::Fill(Fill::Outline).tip(), "Outline");
        assert_eq!(Command::Fill(Fill::Tint).tip(), "Tint");
        assert_eq!(Command::Fill(Fill::Solid).tip(), "Solid");
    }

    #[test]
    fn the_picker_reads_its_name_and_key() {
        assert_eq!(Command::Picker.tip(), "Picker · I");
    }

    #[test]
    fn pin_reads_its_name_and_key() {
        assert_eq!(Command::Pin.tip(), "Pin · Ctrl+P");
    }

    #[test]
    fn dimensions_sits_right_aligned_against_the_zoom_as_wide_as_its_text() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        // "249 × 249" 50 wide, with 8 either side, ending where the zoom starts.
        assert_eq!(edges(bar.dimensions(50.0)), [677.0, 4.0, 743.0, 36.0]);
        // A fraction of a pixel takes a whole one.
        assert_eq!(edges(bar.dimensions(49.2)), [677.0, 4.0, 743.0, 36.0]);
        let bar = Layout::new(1.5, 0, &WIDTHS_150);
        assert_eq!(edges(bar.dimensions(75.0)), [1015.0, 6.0, 1114.0, 54.0]);
    }

    #[test]
    fn the_status_is_trimmed_and_never_dimensions() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        // A readout narrower than the widest leaves the status the difference.
        assert_eq!(
            edges(bar.status(Group::Empty, 50.0)),
            [435.0, 4.0, 677.0, 36.0]
        );
        // In the narrowest bar the widest readout still has its room, and the widest status
        // its own beside it.
        let widest = bar.status(Group::Empty, WIDTHS.readout);
        assert_eq!(widest.right - widest.left, 200.0 + 2.0 * 6.0);
        assert_eq!(widest.right, bar.dimensions(WIDTHS.readout).left);
        // Beside a crop's group the status's room is what the readout leaves.
        assert_eq!(edges(bar.status(CROP, 50.0)), [650.0, 4.0, 677.0, 36.0]);
    }

    #[test]
    fn dimensions_cannot_be_clicked() {
        let bar = Layout::new(1.0, 0, &WIDTHS);
        assert_eq!(bar.hit(700.0, 20.0, Group::Empty), None);
    }

    #[test]
    fn dimensions_reads_the_captures_width_and_height_after_any_crop() {
        let image = Image::new(
            1706,
            928,
            vec![0; 1706 * 928 * 4],
            SYSTEMTIME::default(),
            1.0,
        );
        assert_eq!(dimensions(&image), "1706 × 928");
        let cropped = image.cropped(Region {
            left: 10,
            top: 20,
            right: 110,
            bottom: 70,
        });
        assert_eq!(dimensions(&cropped), "100 × 50");
    }

    #[test]
    fn a_crop_being_chosen_reads_out_its_width_and_height() {
        let group = Group::new(Some((640, 360)));
        assert_eq!(group, CROP);
        assert_eq!(group.readout().as_deref(), Some("640 × 360"));
        assert_eq!(Group::new(None), Group::Empty);
        assert_eq!(Group::Empty.readout(), None);
    }
}
