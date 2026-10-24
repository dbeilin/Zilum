//! The style of an annotation, its colour and its size, an arrow's head, a rectangle's Fill, a
//! blur's level, and what each means when it is drawn.

/// One of the eight colours an annotation can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Colour {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    White,
    Black,
}

impl Colour {
    /// Every colour, in the order the toolbar shows them and keys 1 to 8 pick them.
    pub const ALL: [Colour; 8] = [
        Colour::Red,
        Colour::Orange,
        Colour::Yellow,
        Colour::Green,
        Colour::Blue,
        Colour::Purple,
        Colour::White,
        Colour::Black,
    ];

    /// The colour on first run.
    pub const FIRST_RUN: Colour = Colour::Red;

    /// As sRGB.
    pub fn rgb(self) -> u32 {
        match self {
            Colour::Red => 0xE81123,
            Colour::Orange => 0xF7630C,
            Colour::Yellow => 0xFFCC00,
            Colour::Green => 0x107C10,
            Colour::Blue => 0x1F6FEB,
            Colour::Purple => 0x8B2FC9,
            Colour::White => 0xFFFFFF,
            Colour::Black => 0x000000,
        }
    }

    /// The thin edge drawn around it for contrast: white around colours darker than mid-grey,
    /// and black around the rest.
    pub fn halo(self) -> Colour {
        if luminance(self.rgb()) < luminance(MID_GREY) {
            Colour::White
        } else {
            Colour::Black
        }
    }
}

/// The colour the halo changes at, as sRGB.
const MID_GREY: u32 = 0x808080;

/// The whole values a slider runs over, linearly, from the least at its left end to the most at
/// its right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    least: u8,
    most: u8,
}

impl Span {
    /// The sizes of an arrow, a rectangle and a stroke.
    pub const STROKES: Span = Span { least: 1, most: 12 };
    /// The sizes of a badge.
    pub const BADGES: Span = Span {
        least: 16,
        most: 64,
    };
    /// A blur's levels.
    const LEVELS: Span = Span { least: 4, most: 40 };

    /// `value`, if it is in the span.
    fn holds(self, value: u32) -> Option<u8> {
        (u32::from(self.least)..=u32::from(self.most))
            .contains(&value)
            .then_some(value as u8)
    }

    /// The nearest value to the knob `along` the slider, 0 at its left end and 1 at its right,
    /// and the ends beyond them.
    fn along(self, along: f32) -> u8 {
        let span = f32::from(self.most - self.least);
        self.least + (along.clamp(0.0, 1.0) * span).round() as u8
    }

    /// How far along the slider the knob sits at `value`, 0 at the left end and 1 at the right,
    /// and at an end for a value beyond it.
    fn along_slider(self, value: f32) -> f32 {
        let span = f32::from(self.most - self.least);
        ((value - f32::from(self.least)) / span).clamp(0.0, 1.0)
    }
}

/// How heavy an annotation is drawn: an arrow's, a rectangle's or a stroke's width, from 1 to 12,
/// or a badge's diameter, from 16 to 64, in capture pixels at 100 percent. The slider and the
/// settings file give whole sizes; a Resize leaves any, beyond them too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size(f32);

impl Size {
    /// The size `value`, if `span` holds it.
    pub fn new(value: u32, span: Span) -> Option<Size> {
        span.holds(value).map(|value| Size(f32::from(value)))
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// In capture pixels, for a capture `scale` physical pixels per logical pixel, so it looks
    /// the same on any screen.
    pub fn pixels(self, scale: f32) -> f32 {
        self.0 * scale
    }

    /// As heavy on a capture resized `by`, a fraction of its dimensions.
    pub fn resized(self, by: f32) -> Size {
        Size(self.0 * by)
    }

    /// The nearest size in `span` to the knob `along` its slider, 0 at its left end and 1 at its
    /// right, and the ends beyond them.
    pub fn along(along: f32, span: Span) -> Size {
        Size(f32::from(span.along(along)))
    }

    /// How far along the slider over `span` its knob sits, 0 at the left end and 1 at the right.
    pub fn along_slider(self, span: Span) -> f32 {
        span.along_slider(self.0)
    }
}

/// The size each tool places its next annotation at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    pub arrow: Size,
    pub rect: Size,
    pub pen: Size,
    pub counter: Size,
}

impl Sizes {
    /// The sizes on first run.
    pub const FIRST_RUN: Sizes = Sizes {
        arrow: Size(3.0),
        rect: Size(3.0),
        pen: Size(3.0),
        counter: Size(32.0),
    };
}

/// The em new text is placed at, from 6 to 400, in capture pixels at 100 percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextSize(u16);

impl TextSize {
    /// The size on first run.
    pub const FIRST_RUN: TextSize = TextSize(20);
    const LEAST: u16 = 6;
    const MOST: u16 = 400;

    /// The size `value`, if it is one.
    pub fn new(value: u32) -> Option<TextSize> {
        (u32::from(TextSize::LEAST)..=u32::from(TextSize::MOST))
            .contains(&value)
            .then_some(TextSize(value as u16))
    }

    pub fn get(self) -> u16 {
        self.0
    }

    /// Its em in capture pixels, for a capture `scale` physical pixels per logical pixel, so it
    /// looks the same on any screen.
    pub fn font(self, scale: f32) -> f32 {
        f32::from(self.0) * scale
    }

    /// The nearest size to an em of `font` capture pixels on a capture `scale` physical pixels
    /// per logical pixel, and the least or the most beyond them.
    pub fn of_font(font: f32, scale: f32) -> TextSize {
        let value = (font / scale).round();
        TextSize(value.clamp(f32::from(TextSize::LEAST), f32::from(TextSize::MOST)) as u16)
    }
}

/// How coarse a blur's blocks are: their side in capture pixels at 100 percent, from 4 to 40. The
/// slider and the settings file give whole levels; a Resize leaves any, below them too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level(f32);

impl Level {
    /// The level on first run.
    pub const FIRST_RUN: Level = Level(8.0);

    /// The level `value`, if it is one.
    pub fn new(value: u32) -> Option<Level> {
        Span::LEVELS
            .holds(value)
            .map(|value| Level(f32::from(value)))
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// The side of its blocks, in capture pixels, for a capture `scale` physical pixels per
    /// logical pixel.
    pub fn block(self, scale: f32) -> usize {
        ((self.0 * scale).round() as usize).max(1)
    }

    /// As coarse on a capture resized `by`, a fraction of its dimensions, so its blur keeps as
    /// many blocks.
    pub fn resized(self, by: f32) -> Level {
        Level(self.0 * by)
    }

    /// The nearest level to the knob `along` the slider, 0 at its left end and 1 at its right,
    /// and the ends beyond them.
    pub fn along(along: f32) -> Level {
        Level(f32::from(Span::LEVELS.along(along)))
    }

    /// How far along the slider its knob sits, 0 at the left end and 1 at the right.
    pub fn along_slider(self) -> f32 {
        Span::LEVELS.along_slider(self.0)
    }
}

/// Which ends of an arrow carry a head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Head {
    /// At the tip alone.
    One,
    Both,
    /// Neither, which makes a plain line.
    None,
}

impl Head {
    /// Every head, in the order Arrow's flyout shows them.
    pub const ALL: [Head; 3] = [Head::One, Head::Both, Head::None];

    /// The head on first run.
    pub const FIRST_RUN: Head = Head::One;

    /// Whether the tip carries a head.
    pub fn at_tip(self) -> bool {
        self != Head::None
    }

    /// Whether the tail carries a head.
    pub fn at_tail(self) -> bool {
        self == Head::Both
    }
}

/// How a rectangle is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fill {
    /// The line alone.
    Outline,
    /// The line, with its colour laid translucently over what is inside it.
    Tint,
    /// A block of its colour, with no line and no halo.
    Solid,
}

impl Fill {
    /// Every Fill, in the order Rect's flyout shows them.
    pub const ALL: [Fill; 3] = [Fill::Outline, Fill::Tint, Fill::Solid];

    /// The Fill on first run.
    pub const FIRST_RUN: Fill = Fill::Outline;

    /// How opaque a Tint lays its colour: about a third.
    pub const TINT_OPACITY: f32 = 0.33;
}

/// The colour and size of an annotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub colour: Colour,
    pub size: Size,
}

/// A colour or a size, as a flyout chooses one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Colour(Colour),
    Size(Size),
}

impl Style {
    /// With `choice` in place of its colour or its size.
    pub fn with(self, choice: Choice) -> Style {
        match choice {
            Choice::Colour(colour) => Style { colour, ..self },
            Choice::Size(size) => Style { size, ..self },
        }
    }
}

/// The relative luminance of an sRGB colour, from 0 for black to 1 for white.
fn luminance(rgb: u32) -> f32 {
    let linear = |shift: u32| {
        let value = ((rgb >> shift) & 0xFF) as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_colours_have_a_white_halo_and_light_ones_a_black_halo() {
        for (colour, halo) in [
            (Colour::Red, Colour::White),
            // Lighter than mid-grey, at about 0.29 of white's luminance against its 0.22.
            (Colour::Orange, Colour::Black),
            (Colour::Yellow, Colour::Black),
            (Colour::Green, Colour::White),
            (Colour::Blue, Colour::White),
            (Colour::Purple, Colour::White),
            (Colour::White, Colour::Black),
            (Colour::Black, Colour::White),
        ] {
            assert_eq!(colour.halo(), halo, "{colour:?}");
        }
    }

    #[test]
    fn a_stroke_is_1_to_12_and_a_badge_16_to_64() {
        for (span, below, least, most, beyond) in [
            (Span::STROKES, 0, 1, 12, 13),
            (Span::BADGES, 15, 16, 64, 65),
        ] {
            assert_eq!(Size::new(below, span), None, "{span:?}");
            assert_eq!(Size::new(least, span).map(Size::get), Some(least as f32));
            assert_eq!(Size::new(most, span).map(Size::get), Some(most as f32));
            assert_eq!(Size::new(beyond, span), None, "{span:?}");
        }
    }

    #[test]
    fn first_run_gives_arrows_rectangles_and_strokes_3_and_badges_32() {
        let Sizes {
            arrow,
            rect,
            pen,
            counter,
        } = Sizes::FIRST_RUN;
        assert_eq!(
            [arrow, rect, pen, counter].map(Size::get),
            [3.0, 3.0, 3.0, 32.0]
        );
    }

    #[test]
    fn a_size_is_as_many_capture_pixels_at_100_percent_and_grows_with_the_capture_scale() {
        let size = |value| Size::new(value, Span::BADGES).unwrap();
        for (scale, pixels) in [
            (1.0, [16.0, 32.0, 64.0]),
            (1.5, [24.0, 48.0, 96.0]),
            (2.0, [32.0, 64.0, 128.0]),
        ] {
            for (value, pixels) in [16, 32, 64].into_iter().zip(pixels) {
                assert_eq!(size(value).pixels(scale), pixels, "{value} at {scale}");
            }
        }
        assert_eq!(Size::new(3, Span::STROKES).unwrap().pixels(1.25), 3.75);
    }

    #[test]
    fn the_size_slider_runs_linearly_over_each_span() {
        for (along, stroke, badge) in [
            (0.0, 1, 16),
            (1.0, 12, 64),
            (0.5, 7, 40),
            // Nearest whole size: a fifth of 11 past 1 is 3.2, and a fifth of 48 past 16 is 25.6.
            (0.2, 3, 26),
            // Beyond the ends, the ends.
            (-0.5, 1, 16),
            (1.5, 12, 64),
        ] {
            assert_eq!(
                Size::along(along, Span::STROKES).get(),
                stroke as f32,
                "{along}"
            );
            assert_eq!(
                Size::along(along, Span::BADGES).get(),
                badge as f32,
                "{along}"
            );
        }
        let stroke = |value| Size::new(value, Span::STROKES).unwrap();
        let badge = |value| Size::new(value, Span::BADGES).unwrap();
        assert_eq!(stroke(1).along_slider(Span::STROKES), 0.0);
        assert_eq!(stroke(12).along_slider(Span::STROKES), 1.0);
        assert_eq!(badge(16).along_slider(Span::BADGES), 0.0);
        assert_eq!(badge(40).along_slider(Span::BADGES), 0.5);
        assert_eq!(badge(64).along_slider(Span::BADGES), 1.0);
        // A value beyond the span, such as a badge shrunk below 16, puts the knob at the end.
        assert_eq!(Span::BADGES.along_slider(8.0), 0.0);
        assert_eq!(Span::STROKES.along_slider(30.0), 1.0);
        // Every size reads back from where its knob sits.
        for (span, values) in [(Span::STROKES, 1..=12), (Span::BADGES, 16..=64)] {
            for value in values {
                let size = Size::new(value, span).unwrap();
                assert_eq!(Size::along(size.along_slider(span), span), size, "{value}");
            }
        }
    }

    #[test]
    fn a_level_is_4_to_40_and_8_on_first_run() {
        assert_eq!(Level::new(3), None);
        assert_eq!(Level::new(4).map(Level::get), Some(4.0));
        assert_eq!(Level::new(40).map(Level::get), Some(40.0));
        assert_eq!(Level::new(41), None);
        assert_eq!(Level::FIRST_RUN.get(), 8.0);
    }

    #[test]
    fn a_levels_blocks_are_as_many_pixels_at_100_percent_and_grow_with_the_capture_scale() {
        let level = |value| Level::new(value).unwrap();
        for (scale, blocks) in [
            (1.0, [4, 8, 23, 40]),
            (1.25, [5, 10, 29, 50]),
            (1.5, [6, 12, 35, 60]),
            (2.0, [8, 16, 46, 80]),
        ] {
            for (value, block) in [4, 8, 23, 40].into_iter().zip(blocks) {
                assert_eq!(level(value).block(scale), block, "{value} at {scale}");
            }
        }
    }

    #[test]
    fn the_slider_runs_linearly_from_4_at_its_left_end_to_40_at_its_right() {
        for (along, value) in [
            (0.0, 4),
            (0.5, 22),
            (1.0, 40),
            // A ninth of the way is 8: 4 and a ninth of 36.
            (1.0 / 9.0, 8),
            // Nearest whole level.
            (0.26, 13),
            (0.27, 14),
            // Beyond the ends, the ends.
            (-0.5, 4),
            (1.5, 40),
        ] {
            assert_eq!(Level::along(along).get(), value as f32, "{along}");
        }
        for (value, along) in [(4, 0.0), (22, 0.5), (40, 1.0), (13, 0.25)] {
            assert_eq!(Level::new(value).unwrap().along_slider(), along, "{value}");
        }
        // Every level reads back from where its knob sits.
        for value in 4..=40 {
            let level = Level::new(value).unwrap();
            assert_eq!(Level::along(level.along_slider()), level);
        }
    }
}
