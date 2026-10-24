//! The flyouts at one DPI, in physical client pixels: where each hangs under its tool's button,
//! where the swatches, the Size slider, the Heads and the Fills of a style flyout's row and the
//! Blur flyout's Level slider sit, where a slider's knob sits, what each shows, and what opens and
//! closes them.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::{
    annotation::Annotation,
    command_bar::Command,
    image::Resize,
    style::{Colour, Fill, Head, Level, Size, Span, Style},
};

/// Measurements in logical pixels, for every flyout: from its tool's button's bottom to the
/// panel's top, the panel's height, and either side of what it holds and above and below it.
/// Then the width of a slider, Level or Size.
const GAP: f32 = 8.0;
const HEIGHT: f32 = 32.0;
const PADDING_ACROSS: f32 = 8.0;
const PADDING_DOWN: f32 = 4.0;
const SLIDER_WIDTH: f32 = 120.0;
/// The diameter of a slider's knob.
pub const KNOB: f32 = 18.0;

/// The panel and the Level slider in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flyout {
    pub panel: D2D_RECT_F,
    pub level: D2D_RECT_F,
}

impl Flyout {
    /// The flyout under `blur`, the Blur button, at `scale`, physical pixels per logical pixel.
    /// It hangs from the button's left edge, so it is aligned with it.
    pub fn under(blur: D2D_RECT_F, scale: f32) -> Flyout {
        // Each size and gap is rounded on its own, as the bar's are.
        let px = |logical: f32| (logical * scale).round();
        let top = blur.bottom + px(GAP);
        let panel = D2D_RECT_F {
            left: blur.left,
            top,
            right: blur.left + 2.0 * px(PADDING_ACROSS) + px(SLIDER_WIDTH),
            bottom: top + px(HEIGHT),
        };
        Flyout {
            panel,
            level: D2D_RECT_F {
                left: panel.left + px(PADDING_ACROSS),
                top: panel.top + px(PADDING_DOWN),
                right: panel.right - px(PADDING_ACROSS),
                bottom: panel.bottom - px(PADDING_DOWN),
            },
        }
    }
}

/// Measurements of a style flyout's row in logical pixels: a swatch's, a Head's and a Fill's
/// button, square, between two of them, and either side of the divider and its height, as on the
/// bar. Then either side of a Resize's label.
const ROW_BUTTON: f32 = 24.0;
const ROW_GAP: f32 = 2.0;
const DIVIDER_MARGIN: f32 = 4.0;
const DIVIDER_HEIGHT: f32 = 16.0;
const LABEL_PADDING: f32 = 8.0;

/// A style flyout: a panel like Blur's holding one row, the eight swatches, a divider and the
/// Size slider, and for Arrow a divider and the three Heads after them, and for Rect the three
/// Fills. Text's holds the swatches alone, as a text is resized by its handles. The Dimensions
/// flyout is a row too, of the three Resizes.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleFlyout {
    pub panel: D2D_RECT_F,
    pub dividers: Vec<D2D_RECT_F>,
    /// Its buttons, left to right.
    pub buttons: Vec<(Command, D2D_RECT_F)>,
    /// The Size slider, which Text's has not.
    pub slider: Option<D2D_RECT_F>,
}

impl StyleFlyout {
    /// The row under `button`, `tool`'s, at `scale`. It hangs from the button's left edge, as the
    /// Blur flyout does.
    pub fn under(button: D2D_RECT_F, scale: f32, tool: Command) -> StyleFlyout {
        let px = |logical: f32| (logical * scale).round();
        let top = button.bottom + px(GAP);
        let bottom = top + px(HEIGHT);
        // What the row holds from `left`, `width` across.
        let place = |left: f32, width: f32| D2D_RECT_F {
            left,
            top: top + px(PADDING_DOWN),
            right: left + width,
            bottom: bottom - px(PADDING_DOWN),
        };
        let divider_top = top + px((HEIGHT - DIVIDER_HEIGHT) / 2.0);
        let mut dividers = Vec::new();
        // The divider after `right`, and where the next control starts.
        let mut divide = |right: f32| {
            let left = right + px(DIVIDER_MARGIN);
            dividers.push(D2D_RECT_F {
                left,
                top: divider_top,
                right: left + 1.0,
                bottom: divider_top + px(DIVIDER_HEIGHT),
            });
            left + 1.0 + px(DIVIDER_MARGIN)
        };
        let mut buttons = Vec::new();
        let mut left = button.left + px(PADDING_ACROSS);
        for (at, colour) in Colour::ALL.into_iter().enumerate() {
            if at > 0 {
                left += px(ROW_GAP);
            }
            let rect = place(left, px(ROW_BUTTON));
            buttons.push((Command::Colour(colour), rect));
            left = rect.right;
        }
        let mut slider = None;
        if tool != Command::Text {
            let rect = place(divide(left), px(SLIDER_WIDTH));
            slider = Some(rect);
            left = rect.right;
        }
        let choices: Vec<Command> = match tool {
            Command::Arrow => Head::ALL.into_iter().map(Command::Head).collect(),
            Command::Rect => Fill::ALL.into_iter().map(Command::Fill).collect(),
            _ => Vec::new(),
        };
        if !choices.is_empty() {
            left = divide(left);
            for (at, choice) in choices.into_iter().enumerate() {
                if at > 0 {
                    left += px(ROW_GAP);
                }
                let rect = place(left, px(ROW_BUTTON));
                buttons.push((choice, rect));
                left = rect.right;
            }
        }
        StyleFlyout {
            panel: D2D_RECT_F {
                left: button.left,
                top,
                right: left + px(PADDING_ACROSS),
                bottom,
            },
            dividers,
            buttons,
            slider,
        }
    }

    /// The Dimensions flyout under `readout`, the Dimensions readout's area, at `scale`: a row of
    /// the three Resizes, each its label, `label` wide, with room either side, as tall as a
    /// swatch. It hangs from the readout's left edge, as the others hang from their button's.
    pub fn resizes(readout: D2D_RECT_F, scale: f32, label: f32) -> StyleFlyout {
        let px = |logical: f32| (logical * scale).round();
        let top = readout.bottom + px(GAP);
        let bottom = top + px(HEIGHT);
        let width = label.ceil() + 2.0 * px(LABEL_PADDING);
        let mut buttons = Vec::new();
        let mut left = readout.left + px(PADDING_ACROSS);
        for (at, resize) in Resize::ALL.into_iter().enumerate() {
            if at > 0 {
                left += px(ROW_GAP);
            }
            let rect = D2D_RECT_F {
                left,
                top: top + px(PADDING_DOWN),
                right: left + width,
                bottom: bottom - px(PADDING_DOWN),
            };
            buttons.push((Command::Resize(resize), rect));
            left = rect.right;
        }
        StyleFlyout {
            panel: D2D_RECT_F {
                left: readout.left,
                top,
                right: left + px(PADDING_ACROSS),
                bottom,
            },
            dividers: Vec::new(),
            buttons,
            slider: None,
        }
    }

    /// The button or the Size slider at `x`, `y`, if either is there.
    pub fn hit(&self, x: f32, y: f32) -> Option<Command> {
        let inside = |rect: &D2D_RECT_F| {
            x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
        };
        if self.slider.as_ref().is_some_and(inside) {
            return Some(Command::Size);
        }
        self.buttons
            .iter()
            .find(|(_, rect)| inside(rect))
            .map(|(command, _)| *command)
    }
}

/// The tools whose flyout holds a row of the style; Blur's holds Level instead.
pub const STYLED: [Command; 5] = [
    Command::Arrow,
    Command::Rect,
    Command::Pen,
    Command::Text,
    Command::Counter,
];

/// The sizes the Size slider in `tool`'s flyout runs over: a badge's diameters under Counter, and
/// a stroke's widths under the rest.
pub fn span(tool: Command) -> Span {
    if tool == Command::Counter {
        Span::BADGES
    } else {
        Span::STROKES
    }
}

/// Something that may open or close a flyout. `selected` is the tool that places the annotation
/// selected, if one is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The selection changing, by a press or by drawing an annotation, which is selected on
    /// release, or a press under Select even on what is selected. The press that draws selects
    /// nothing until the release.
    Selection {
        selected: Option<Command>,
    },
    /// A tool becoming active by its button's click or its key, or its click or key when it
    /// already is.
    Tool {
        tool: Command,
        selected: Option<Command>,
    },
    Escape,
    /// A click on the Dimensions readout, with the flyout `open` before it, if one was.
    Readout {
        open: Option<Command>,
    },
    /// A press on neither the flyout that shows nor the readout, with the flyout `open` before it,
    /// if one was, which selects nothing and activates no tool by itself.
    Press {
        open: Option<Command>,
    },
}

/// The tool whose flyout is open after `event`, if any, whatever was open before. An annotation
/// becoming selected opens the flyout under the tool that places it. A tool's click or key
/// leaves its own open while one of its annotations is selected, and otherwise opens nothing,
/// so a colour is chosen on the annotation just drawn and remembered for the next. A press does
/// nothing to a tool's flyout but through the selection. A click on the Dimensions readout opens
/// its flyout in place of any other, or closes it, and anything else closes it.
pub fn open_after(event: Event) -> Option<Command> {
    match event {
        Event::Selection { selected } => selected.filter(|&tool| has_flyout(tool)),
        Event::Tool { tool, selected } => selected.filter(|&by| by == tool),
        Event::Escape => None,
        Event::Readout { open } => {
            (open != Some(Command::Dimensions)).then_some(Command::Dimensions)
        }
        Event::Press { open } => open.filter(|&open| open != Command::Dimensions),
    }
}

/// Whether `tool` has a flyout under its button.
pub fn has_flyout(tool: Command) -> bool {
    tool == Command::Blur || STYLED.contains(&tool)
}

/// What a flyout shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    /// The row with `colour` ringed, the Size slider's knob at `size` if the row has the slider,
    /// which Text's has not, `head` pressed for Arrow's, and `fill` pressed for Rect's.
    Style {
        colour: Colour,
        size: Option<Size>,
        head: Option<Head>,
        fill: Option<Fill>,
    },
    /// Level, its knob at the selected blur's level.
    Level(Level),
    /// The Dimensions flyout's three Resizes.
    Resizes,
}

/// What the flyout under `under`'s button shows with `selected` selected, if it shows: the style
/// of what is selected if `under` places it, and nothing otherwise, as every flyout waits for an
/// annotation. Text's shows a colour alone. The Dimensions flyout shows whatever is selected.
pub fn shown(under: Command, selected: Option<&Annotation>) -> Option<Shown> {
    if under == Command::Dimensions {
        return Some(Shown::Resizes);
    }
    let style = |style: Style, head, fill| Shown::Style {
        colour: style.colour,
        size: Some(style.size),
        head,
        fill,
    };
    match selected.filter(|annotation| placed_by(annotation) == under)? {
        Annotation::Blur(_, level) => Some(Shown::Level(*level)),
        Annotation::Rectangle(_, shape, fill) => Some(style(*shape, None, Some(*fill))),
        Annotation::Arrow(arrow, shape) => Some(style(*shape, Some(arrow.heads()), None)),
        Annotation::Stroke(_, shape) | Annotation::Badge(_, shape) => {
            Some(style(*shape, None, None))
        }
        Annotation::Text(_, colour) => Some(Shown::Style {
            colour: *colour,
            size: None,
            head: None,
            fill: None,
        }),
    }
}

/// The tool that places `annotation`.
pub fn placed_by(annotation: &Annotation) -> Command {
    match annotation {
        Annotation::Arrow(..) => Command::Arrow,
        Annotation::Rectangle(..) => Command::Rect,
        Annotation::Text(..) => Command::Text,
        Annotation::Stroke(..) => Command::Pen,
        Annotation::Badge(..) => Command::Counter,
        Annotation::Blur(..) => Command::Blur,
    }
}

/// Where the centre of the knob of a slider covering `area` sits across `along` it, 0 at its
/// left end and 1 at its right, at `scale`: from half a knob in to half a knob in from the right.
pub fn knob(area: D2D_RECT_F, along: f32, scale: f32) -> f32 {
    let (start, travel) = knob_travel(area, scale);
    start + along * travel
}

/// How far along a slider covering `area` a press or drag at `x` puts its knob, at `scale`: 0 at
/// its left end and 1 at its right, and beyond them beyond either end.
pub fn along(x: f32, area: D2D_RECT_F, scale: f32) -> f32 {
    let (start, travel) = knob_travel(area, scale);
    (x - start) / travel
}

/// How far from the knob's centre a press at `x` took hold of it, with a slider covering `area`
/// at `along`, at `scale`, so the knob follows the pointer from where it was; none for a press
/// on the track off the knob, which jumps the knob there.
pub fn grab(x: f32, area: D2D_RECT_F, along: f32, scale: f32) -> f32 {
    let off = x - knob(area, along, scale);
    if off.abs() <= KNOB * scale / 2.0 {
        off
    } else {
        0.0
    }
}

/// Where the knob's centre is at the left end, and how far it goes from there.
fn knob_travel(area: D2D_RECT_F, scale: f32) -> (f32, f32) {
    let half = KNOB * scale / 2.0;
    (area.left + half, area.right - area.left - 2.0 * half)
}

#[cfg(test)]
mod tests {
    use windows_numerics::Vector2;

    use super::*;
    use crate::{
        annotation::{Badge, Kind, Stroke, Text},
        image::Resize,
    };

    /// The flyout under Blur's button as the bar lays it out at 100 and 150 percent.
    fn at_100() -> Flyout {
        Flyout::under(rect([232.0, 4.0, 268.0, 36.0]), 1.0)
    }

    fn at_150() -> Flyout {
        Flyout::under(rect([348.0, 6.0, 402.0, 54.0]), 1.5)
    }

    fn level_at(x: f32, area: D2D_RECT_F, scale: f32) -> Level {
        Level::along(along(x, area, scale))
    }

    #[test]
    fn the_knob_runs_from_4_to_40_between_the_levels_ends_less_half_a_knob() {
        let flyout = at_100();
        let level = flyout.level;
        let at = |value| Level::new(value).unwrap();
        let knob = |area, value, scale| knob(area, at(value).along_slider(), scale);
        // An 18-pixel knob, so its centre goes from 9 in at 4 to 9 in from the right at 40.
        assert_eq!(knob(level, 4, 1.0), 249.0);
        assert_eq!(knob(level, 22, 1.0), 300.0);
        assert_eq!(knob(level, 40, 1.0), 351.0);
        // And back: where a press or drag puts it, at the nearest level, the ends beyond them.
        for (x, value) in [
            (249.0, 4),
            (300.0, 22),
            (351.0, 40),
            (240.0, 4),
            (0.0, 4),
            (360.0, 40),
            (2000.0, 40),
            // 21 of 102 pixels in is 7.4 of 36 levels.
            (270.0, 11),
        ] {
            assert_eq!(level_at(x, level, 1.0), at(value), "{x}");
        }
        for value in 4..=40 {
            assert_eq!(level_at(knob(level, value, 1.0), level, 1.0), at(value));
        }

        // At 150 percent, a 27-pixel knob.
        let flyout = at_150();
        let level = flyout.level;
        assert_eq!(knob(level, 4, 1.5), 373.5);
        assert_eq!(knob(level, 40, 1.5), 526.5);
        assert_eq!(level_at(526.0, level, 1.5), at(40));
    }

    #[test]
    fn a_press_on_the_knob_holds_it_where_it_is_and_one_on_the_track_jumps_it_there() {
        let flyout = at_100();
        let area = flyout.level;
        let eight = Level::FIRST_RUN.along_slider();
        // The knob at 8 is centred at 260.3, 18 across.
        for x in [252.0, 260.0, 269.0] {
            let held = grab(x, area, eight, 1.0);
            assert_eq!(level_at(x - held, area, 1.0), Level::FIRST_RUN, "{x}");
            // And follows the pointer from there.
            assert_eq!(
                level_at(x + 51.0 - held, area, 1.0),
                Level::new(26).unwrap(),
                "{x}"
            );
        }
        for x in [245.0, 300.0, 351.0] {
            assert_eq!(grab(x, area, eight, 1.0), 0.0, "{x}");
        }
    }

    /// Arrow's and Counter's rows as the bar lays them out at 100 percent, and Rect's at 150.
    fn arrows_row() -> StyleFlyout {
        StyleFlyout::under(rect([42.0, 4.0, 78.0, 36.0]), 1.0, Command::Arrow)
    }

    fn counters_row() -> StyleFlyout {
        StyleFlyout::under(rect([194.0, 4.0, 230.0, 36.0]), 1.0, Command::Counter)
    }

    fn rects_row_at_150() -> StyleFlyout {
        StyleFlyout::under(rect([63.0, 6.0, 117.0, 54.0]), 1.5, Command::Rect)
    }

    #[test]
    fn the_size_knob_runs_over_a_strokes_widths_and_a_badges_diameters() {
        let size_at = |x, area, span, scale| Size::along(along(x, area, scale), span);
        let size = |value, span| Size::new(value, span).unwrap();
        let knob =
            |area, value, span, scale| knob(area, size(value, span).along_slider(span), scale);

        // Arrow's slider at 100 percent, 265 to 385: the knob's centre from 274 to 376.
        let area = arrows_row().slider.unwrap();
        let strokes = span(Command::Arrow);
        assert_eq!(knob(area, 1, strokes, 1.0), 274.0);
        assert_eq!(knob(area, 12, strokes, 1.0), 376.0);
        for (x, value) in [
            (274.0, 1),
            (376.0, 12),
            (0.0, 1),
            (2000.0, 12),
            // 26 of 102 pixels in is 2.8 of 11 widths.
            (300.0, 4),
        ] {
            assert_eq!(size_at(x, area, strokes, 1.0), size(value, strokes), "{x}");
        }

        // Counter's, as wide: halfway is 40.
        let area = counters_row().slider.unwrap();
        let badges = span(Command::Counter);
        assert_eq!(knob(area, 16, badges, 1.0), area.left + 9.0);
        assert_eq!(knob(area, 64, badges, 1.0), area.right - 9.0);
        assert_eq!(
            size_at(area.left + 60.0, area, badges, 1.0),
            size(40, badges)
        );

        // Rect's at 150 percent, 397 to 577, a 27-pixel knob.
        let area = rects_row_at_150().slider.unwrap();
        assert_eq!(knob(area, 1, strokes, 1.5), 410.5);
        assert_eq!(knob(area, 12, strokes, 1.5), 563.5);

        // Every size reads back from where its knob sits, at either scale.
        for (area, scale) in [
            (arrows_row().slider.unwrap(), 1.0),
            (rects_row_at_150().slider.unwrap(), 1.5),
        ] {
            for (span, values) in [(Span::STROKES, 1..=12), (Span::BADGES, 16..=64)] {
                for value in values {
                    let x = knob(area, value, span, scale);
                    assert_eq!(size_at(x, area, span, scale), size(value, span), "{value}");
                }
            }
        }
    }

    #[test]
    fn counters_slider_sets_a_badges_size_and_the_others_a_strokes() {
        assert_eq!(span(Command::Counter), Span::BADGES);
        for tool in [Command::Arrow, Command::Rect, Command::Pen] {
            assert_eq!(span(tool), Span::STROKES, "{tool:?}");
        }
    }

    /// What may be selected when an event arrives: nothing, or an annotation the tool placed.
    const SELECTED: [Option<Command>; 7] = [
        None,
        Some(Command::Arrow),
        Some(Command::Rect),
        Some(Command::Pen),
        Some(Command::Text),
        Some(Command::Counter),
        Some(Command::Blur),
    ];

    #[test]
    fn a_tools_click_or_key_keeps_its_own_open_only_with_one_of_its_annotations_selected() {
        for tool in STYLED.into_iter().chain([Command::Blur]) {
            // With nothing selected it opens nothing; with its own it leaves its own open; with
            // another tool's it closes that one, and the editor deselects it.
            for selected in SELECTED {
                let event = Event::Tool { tool, selected };
                let expected = (selected == Some(tool)).then_some(tool);
                assert_eq!(open_after(event), expected, "{event:?}");
            }
        }
    }

    #[test]
    fn a_tool_with_no_flyout_closes_it() {
        for tool in [Command::Select, Command::Picker, Command::Crop] {
            for selected in SELECTED {
                let event = Event::Tool { tool, selected };
                assert_eq!(open_after(event), None, "{event:?}");
            }
        }
    }

    #[test]
    fn an_annotation_becoming_selected_opens_the_flyout_of_the_tool_that_places_it() {
        // Drawn, which selects it on release, committed with Enter, or clicked under Select.
        for tool in [
            Command::Arrow,
            Command::Rect,
            Command::Pen,
            Command::Text,
            Command::Counter,
            Command::Blur,
        ] {
            let event = Event::Selection {
                selected: Some(tool),
            };
            assert_eq!(open_after(event), Some(tool), "{event:?}");
        }
    }

    /// Blue at a size each span holds.
    fn blue() -> Style {
        Style {
            colour: Colour::Blue,
            size: nine(),
        }
    }

    fn nine() -> Size {
        Size::new(9, Span::STROKES).unwrap()
    }

    fn at(x: f32, y: f32) -> Vector2 {
        Vector2 { X: x, Y: y }
    }

    fn drawn(kind: Kind, style: Style, fill: Fill) -> Annotation {
        let level = Level::new(30).unwrap();
        // An arrow with both heads, so it shows other than the first run's.
        Annotation::spanned(
            kind,
            at(0.0, 0.0),
            at(40.0, 30.0),
            style,
            Head::Both,
            fill,
            level,
        )
    }

    fn badge() -> Annotation {
        let badge = Badge {
            centre: at(20.0, 20.0),
            count: 3,
        };
        Annotation::Badge(badge, blue())
    }

    fn stroke() -> Annotation {
        Annotation::Stroke(
            Stroke::smoothed(&[at(0.0, 0.0), at(20.0, 20.0)], 3.0),
            blue(),
        )
    }

    fn style(colour: Colour, size: Size, head: Option<Head>, fill: Option<Fill>) -> Option<Shown> {
        Some(Shown::Style {
            colour,
            size: Some(size),
            head,
            fill,
        })
    }

    #[test]
    fn with_an_annotation_of_its_tool_selected_it_shows_that_annotations_style() {
        let arrow = drawn(Kind::Arrow, blue(), Fill::Outline);
        let rectangle = drawn(Kind::Rectangle, blue(), Fill::Tint);
        assert_eq!(
            shown(Command::Arrow, Some(&arrow)),
            style(Colour::Blue, nine(), Some(Head::Both), None)
        );
        assert_eq!(
            shown(Command::Rect, Some(&rectangle)),
            style(Colour::Blue, nine(), None, Some(Fill::Tint))
        );
        assert_eq!(
            shown(Command::Counter, Some(&badge())),
            style(Colour::Blue, nine(), None, None)
        );
        assert_eq!(
            shown(Command::Pen, Some(&stroke())),
            style(Colour::Blue, nine(), None, None)
        );
    }

    #[test]
    fn with_nothing_of_its_tool_selected_no_flyout_shows() {
        // Nothing selected, as after an undo took it away, or another tool's annotation.
        let rectangle = drawn(Kind::Rectangle, blue(), Fill::Outline);
        for under in STYLED.into_iter().chain([Command::Blur]) {
            assert_eq!(shown(under, None), None, "{under:?}");
            if under != Command::Rect {
                assert_eq!(shown(under, Some(&rectangle)), None, "{under:?}");
            }
        }
    }

    #[test]
    fn texts_flyout_shows_the_selected_texts_colour_and_no_size() {
        // Committed with Enter, or clicked under Select.
        let text = Annotation::Text(Text::hi(at(0.0, 0.0), 20.0), Colour::Blue);
        assert_eq!(
            shown(Command::Text, Some(&text)),
            Some(Shown::Style {
                colour: Colour::Blue,
                size: None,
                head: None,
                fill: None,
            })
        );
    }

    #[test]
    fn blurs_flyout_shows_the_selected_blurs_level() {
        let blur = drawn(Kind::Blur, blue(), Fill::Outline);
        assert_eq!(
            shown(Command::Blur, Some(&blur)),
            Some(Shown::Level(Level::new(30).unwrap()))
        );
    }

    #[test]
    fn each_annotation_is_placed_by_its_tool() {
        assert_eq!(
            placed_by(&drawn(Kind::Arrow, blue(), Fill::Outline)),
            Command::Arrow
        );
        assert_eq!(
            placed_by(&drawn(Kind::Rectangle, blue(), Fill::Outline)),
            Command::Rect
        );
        assert_eq!(
            placed_by(&drawn(Kind::Blur, blue(), Fill::Outline)),
            Command::Blur
        );
        assert_eq!(placed_by(&stroke()), Command::Pen);
        assert_eq!(placed_by(&badge()), Command::Counter);
        let text = Annotation::Text(Text::hi(at(0.0, 0.0), 20.0), Colour::Blue);
        assert_eq!(placed_by(&text), Command::Text);
    }

    fn edges(rect: D2D_RECT_F) -> [f32; 4] {
        [rect.left, rect.top, rect.right, rect.bottom]
    }

    fn rect([left, top, right, bottom]: [f32; 4]) -> D2D_RECT_F {
        D2D_RECT_F {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn at_100_percent_the_panel_hangs_8_below_blur_from_its_left_edge_around_a_120_pixel_slider() {
        let flyout = at_100();
        // 8 below the button, 4 below the bar's 40, and 8 either side of the slider and 4 above
        // and below it.
        assert_eq!(edges(flyout.panel), [232.0, 44.0, 368.0, 76.0]);
        assert_eq!(edges(flyout.level), [240.0, 48.0, 360.0, 72.0]);
    }

    fn button_edges(row: &StyleFlyout) -> Vec<(Command, [f32; 4])> {
        row.buttons
            .iter()
            .map(|(command, rect)| (*command, edges(*rect)))
            .collect()
    }

    /// The eight swatches of a row from `left`, at 100 percent.
    fn swatches(left: f32) -> Vec<(Command, [f32; 4])> {
        Colour::ALL
            .iter()
            .enumerate()
            .map(|(at, colour)| {
                let left = left + 26.0 * at as f32;
                (Command::Colour(*colour), [left, 48.0, left + 24.0, 72.0])
            })
            .collect()
    }

    #[test]
    fn texts_row_holds_the_eight_swatches_alone() {
        // Text's button as the bar lays it out at 100 percent.
        let row = StyleFlyout::under(rect([156.0, 4.0, 192.0, 36.0]), 1.0, Command::Text);
        // 8 in, eight 24-pixel swatches 2 apart, 8 in: no divider and no slider.
        assert_eq!(edges(row.panel), [156.0, 44.0, 378.0, 76.0]);
        assert_eq!(button_edges(&row), swatches(164.0));
        assert!(row.dividers.is_empty());
        assert_eq!(row.slider, None);
    }

    #[test]
    fn arrows_row_has_a_divider_and_the_three_heads_after_the_slider() {
        let row = arrows_row();
        // The slider ends at 385, then 4, the divider, 4, and three 24-pixel buttons 2 apart,
        // one head, both and none, then 8.
        assert_eq!(edges(row.panel), [42.0, 44.0, 478.0, 76.0]);
        assert_eq!(row.slider.map(edges), Some([265.0, 48.0, 385.0, 72.0]));
        assert_eq!(edges(row.dividers[1]), [389.0, 52.0, 390.0, 68.0]);
        let mut expected = swatches(50.0);
        expected.extend([
            (Command::Head(Head::One), [394.0, 48.0, 418.0, 72.0]),
            (Command::Head(Head::Both), [420.0, 48.0, 444.0, 72.0]),
            (Command::Head(Head::None), [446.0, 48.0, 470.0, 72.0]),
        ]);
        assert_eq!(button_edges(&row), expected);
        // Hit where they are, and not in the gaps between them.
        assert_eq!(row.hit(430.0, 60.0), Some(Command::Head(Head::Both)));
        assert_eq!(row.hit(419.0, 60.0), None);
    }

    #[test]
    fn at_150_percent_a_rows_sizes_and_gaps_are_scaled_and_rounded() {
        // 12 in, 36-pixel swatches 3 apart, 6 either side of a divider, a 180-pixel slider.
        let row = rects_row_at_150();
        assert_eq!(edges(row.panel), [63.0, 66.0, 716.0, 114.0]);
        let buttons = button_edges(&row);
        assert_eq!(buttons[0].1, [75.0, 72.0, 111.0, 108.0]);
        assert_eq!(buttons[7].1, [348.0, 72.0, 384.0, 108.0]);
        assert_eq!(edges(row.dividers[0]), [390.0, 78.0, 391.0, 102.0]);
        assert_eq!(row.slider.map(edges), Some([397.0, 72.0, 577.0, 108.0]));
        assert_eq!(edges(row.dividers[1]), [583.0, 78.0, 584.0, 102.0]);
        assert_eq!(
            buttons[8..],
            [
                (Command::Fill(Fill::Outline), [590.0, 72.0, 626.0, 108.0]),
                (Command::Fill(Fill::Tint), [629.0, 72.0, 665.0, 108.0]),
                (Command::Fill(Fill::Solid), [668.0, 72.0, 704.0, 108.0]),
            ]
        );
    }

    #[test]
    fn a_rows_buttons_and_slider_are_hit_where_they_are_and_nothing_between_them() {
        let row = arrows_row();
        assert_eq!(row.hit(62.0, 60.0), Some(Command::Colour(Colour::Red)));
        for x in [265.0, 330.0, 384.0] {
            assert_eq!(row.hit(x, 60.0), Some(Command::Size), "{x}");
        }
        // Between two swatches, on the divider, in the padding, and beyond the slider.
        assert_eq!(row.hit(75.0, 60.0), None);
        assert_eq!(row.hit(260.0, 60.0), None);
        assert_eq!(row.hit(62.0, 46.0), None);
        assert_eq!(row.hit(330.0, 46.0), None);
        assert_eq!(row.hit(388.0, 60.0), None);
    }

    #[test]
    fn the_dimensions_flyout_hangs_8_below_the_readout_from_its_left_edge_holding_the_resizes() {
        // "2560 × 1440" as the bar lays it out at 100 percent, its labels 25 wide.
        let row = StyleFlyout::resizes(rect([870.0, 4.0, 961.0, 36.0]), 1.0, 25.0);
        // 8 in, three buttons 24 tall with 8 either side of their label, 2 apart, 8 in.
        assert_eq!(edges(row.panel), [870.0, 44.0, 1013.0, 76.0]);
        assert_eq!(
            button_edges(&row),
            [
                (Command::Resize(Resize::Quarter), [878.0, 48.0, 919.0, 72.0]),
                (Command::Resize(Resize::Third), [921.0, 48.0, 962.0, 72.0]),
                (Command::Resize(Resize::Half), [964.0, 48.0, 1005.0, 72.0]),
            ]
        );
        assert!(row.dividers.is_empty());
        assert_eq!(row.slider, None);
        // Hit where they are, and not between them or in the padding.
        assert_eq!(row.hit(940.0, 60.0), Some(Command::Resize(Resize::Third)));
        assert_eq!(row.hit(920.0, 60.0), None);
        assert_eq!(row.hit(874.0, 60.0), None);
    }

    #[test]
    fn a_click_on_the_readout_opens_its_flyout_and_another_closes_it() {
        for open in [None, Some(Command::Arrow), Some(Command::Blur)] {
            assert_eq!(
                open_after(Event::Readout { open }),
                Some(Command::Dimensions),
                "{open:?}"
            );
        }
        let open = Some(Command::Dimensions);
        assert_eq!(open_after(Event::Readout { open }), None);
        // It shows the Resizes whatever is selected.
        let arrow = drawn(Kind::Arrow, blue(), Fill::Outline);
        for selected in [None, Some(&arrow)] {
            assert_eq!(shown(Command::Dimensions, selected), Some(Shown::Resizes));
        }
    }

    #[test]
    fn the_dimensions_flyout_closes_on_a_press_outside_it_esc_and_a_tool_change() {
        let open = Some(Command::Dimensions);
        assert_eq!(open_after(Event::Press { open }), None);
        assert_eq!(open_after(Event::Escape), None);
        assert_eq!(open_after(Event::Selection { selected: None }), None);
        for selected in SELECTED {
            let event = Event::Tool {
                tool: Command::Crop,
                selected,
            };
            assert_eq!(open_after(event), None, "{event:?}");
        }
        // A press outside a style flyout leaves it to the selection, as before.
        for open in [None, Some(Command::Arrow), Some(Command::Blur)] {
            assert_eq!(open_after(Event::Press { open }), open, "{open:?}");
        }
    }
}
