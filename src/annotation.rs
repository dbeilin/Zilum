//! The annotations a user places on a capture, kept in capture pixels, and how they are drawn at
//! the capture's scale, the same on screen at any zoom as in the output.

use std::f32::consts::TAU;

use windows::{
    Win32::Graphics::{
        Direct2D::{
            Common::{
                D2D_RECT_F, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW,
                D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
            },
            D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
            D2D1_DRAW_TEXT_OPTIONS_NO_SNAP, D2D1_LINE_JOIN_ROUND, D2D1_QUADRATIC_BEZIER_SEGMENT,
            D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
            ID2D1DeviceContext, ID2D1Factory, ID2D1PathGeometry, ID2D1SolidColorBrush,
            ID2D1StrokeStyle,
        },
        DirectWrite::{
            DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_WEIGHT_BOLD, DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS,
            DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory,
            IDWriteTextFormat, IDWriteTextLayout,
        },
    },
    core::{Result, w},
};
use windows_numerics::Vector2;

use crate::{
    caret::{Cluster, Laid, Line},
    image::Region,
    style::{Choice, Colour, Fill, Head, Level, Size, Style},
    view::View,
};

/// A drag shorter than this, in capture pixels, places nothing.
const LEAST_DRAG: f32 = 4.0;

/// How far apart the points a stroke is taken at are, in screen pixels.
pub const STROKE_SPACING: f32 = 3.0;
/// How many of the points a stroke is taken at are averaged into each of its points.
const SMOOTHING: usize = 5;

/// The halo on each side of a stroke, in capture pixels at 100 percent scale.
const HALO: f32 = 1.0;
/// How near the line through its ends an arrow's middle handle dropped straightens it, in screen
/// pixels: half a handle's side, as far as the handle reaches from its centre as drawn.
const STRAIGHTENING_REACH: f32 = HANDLE_SIDE / 2.0;
/// How many straight pieces an arrow's curve is taken as where it is hit or cropped.
const CURVE_PIECES: usize = 64;
/// How many times the piece of a curve a point lies in is halved to find the point: to well
/// under a millionth of the curve.
const HALVINGS: usize = 24;

/// The head's length and half its width at the base, in strokes.
const HEAD_LENGTH: f32 = 5.0;
const HEAD_HALF_WIDTH: f32 = 2.25;

/// The em of a badge's digits, in its diameters.
const DIGIT_EM: f32 = 0.55;
/// How wide each of a badge's digits is, in ems: Segoe UI's bold figures are all as wide.
const DIGIT_ADVANCE: f32 = 0.575;

/// How near an arrow's line or a stroke the pointer selects it, in screen pixels, where it is
/// drawn thinner.
const LINE_TOLERANCE: f32 = 6.0;
/// How near a handle's centre the pointer picks it up, across and down, in screen pixels: a
/// little beyond the handle as drawn.
const HANDLE_REACH: f32 = 6.0;
/// The side of a handle as drawn, in screen pixels.
const HANDLE_SIDE: f32 = 8.0;
/// How tall each line of the smallest text is, in capture pixels.
const SMALLEST_LINE: f32 = 6.0;
/// How far outside a selected text's halo its outline is drawn, in screen pixels.
const OUTLINE_GAP: f32 = 2.0;

/// What a drawing tool places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Arrow,
    Rectangle,
    Blur,
}

/// Each in its own style, drawn at the capture's scale.
#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Arrow(Arrow, Style),
    /// An outline, tinted inside with its Fill a Tint, or with its Fill Solid, a block of its
    /// colour with no outline and no halo, which Size does not change.
    Rectangle(Bounds, Style, Fill),
    /// In bold Segoe UI in its colour, at its own font.
    Text(Text, Colour),
    /// Drawn freehand with the Pen, with round ends and joins.
    Stroke(Stroke, Style),
    /// A filled circle holding its count, as wide as its size makes it.
    Badge(Badge, Style),
    /// Hides the capture beneath it with coarse blocks as large as its level makes them, and
    /// nothing else: it has no colour and no size, and is baked into the capture, beneath every
    /// arrow, rectangle and text.
    Blur(Bounds, Level),
}

impl Annotation {
    /// What a drag with `kind` from `from` to `to` shows in `style`, an arrow with `head`, a
    /// rectangle with `fill` and a blur at `level`, while it is being made, however short or
    /// thin.
    pub fn spanned(
        kind: Kind,
        from: Vector2,
        to: Vector2,
        style: Style,
        head: Head,
        fill: Fill,
        level: Level,
    ) -> Annotation {
        match kind {
            Kind::Arrow => Annotation::Arrow(Arrow::straight(from, to).headed(head), style),
            Kind::Rectangle => Annotation::Rectangle(Bounds::spanned(from, to), style, fill),
            Kind::Blur => Annotation::Blur(Bounds::spanned(from, to), level),
        }
    }

    /// What a drag with `kind` from `from` to `to` places in `style`, an arrow with `head`, a
    /// rectangle with `fill` and a blur at `level`, if it is large enough to place anything.
    pub fn from_drag(
        kind: Kind,
        from: Vector2,
        to: Vector2,
        style: Style,
        head: Head,
        fill: Fill,
        level: Level,
    ) -> Option<Annotation> {
        match kind {
            Kind::Arrow => {
                Arrow::from_drag(from, to).map(|arrow| Annotation::Arrow(arrow.headed(head), style))
            }
            Kind::Rectangle => {
                Bounds::from_drag(from, to).map(|bounds| Annotation::Rectangle(bounds, style, fill))
            }
            Kind::Blur => Bounds::from_drag(from, to).map(|bounds| Annotation::Blur(bounds, level)),
        }
    }

    /// With `choice` in place of its colour or its size. Text is resized by its handles alone, so
    /// a size leaves it as it was, and a blur has neither, so anything does.
    pub fn restyled(&self, choice: Choice) -> Annotation {
        match (self, choice) {
            (Annotation::Arrow(arrow, style), _) => Annotation::Arrow(*arrow, style.with(choice)),
            (Annotation::Rectangle(bounds, style, fill), _) => {
                Annotation::Rectangle(*bounds, style.with(choice), *fill)
            }
            (Annotation::Text(text, _), Choice::Colour(colour)) => {
                Annotation::Text(text.clone(), colour)
            }
            (Annotation::Stroke(stroke, style), _) => {
                Annotation::Stroke(stroke.clone(), style.with(choice))
            }
            (Annotation::Badge(badge, style), _) => Annotation::Badge(*badge, style.with(choice)),
            (Annotation::Text(..) | Annotation::Blur(..), _) => self.clone(),
        }
    }

    /// A blur at `level`. Anything else has no level and is left as it was.
    pub fn with_level(&self, level: Level) -> Annotation {
        match self {
            Annotation::Blur(bounds, _) => Annotation::Blur(*bounds, level),
            _ => self.clone(),
        }
    }

    /// An arrow with `head`. Anything else has no head and is left as it was.
    pub fn with_head(&self, head: Head) -> Annotation {
        match self {
            Annotation::Arrow(arrow, style) => Annotation::Arrow(arrow.headed(head), *style),
            _ => self.clone(),
        }
    }

    /// A rectangle with `fill`. Anything else has no Fill and is left as it was.
    pub fn with_fill(&self, fill: Fill) -> Annotation {
        match self {
            Annotation::Rectangle(bounds, style, _) => Annotation::Rectangle(*bounds, *style, fill),
            _ => self.clone(),
        }
    }

    /// The width of an arrow's shaft, a rectangle's outline or a stroke, for a capture `scale`
    /// physical pixels per logical pixel. Text, a badge, a Solid rectangle and a blur have no
    /// stroke.
    fn stroke(&self, scale: f32) -> f32 {
        match self {
            Annotation::Arrow(_, style)
            | Annotation::Rectangle(_, style, Fill::Outline | Fill::Tint)
            | Annotation::Stroke(_, style) => style.size.pixels(scale),
            Annotation::Rectangle(_, _, Fill::Solid)
            | Annotation::Text(..)
            | Annotation::Badge(..)
            | Annotation::Blur(..) => 0.0,
        }
    }

    /// What a selected annotation shows an outline around, if anything: a text's box, or a
    /// stroke's or badge's extent, with what is drawn beyond it, for a capture `scale` physical
    /// pixels per logical pixel.
    pub fn outlined(&self, scale: f32) -> Option<Bounds> {
        match self {
            Annotation::Text(text, _) => Some(text.drawn(scale)),
            Annotation::Stroke(stroke, _) => {
                let reach = self.stroke(scale) / 2.0 + HALO * scale;
                Some(stroke.extent().grown(reach))
            }
            Annotation::Badge(badge, style) => Some(badge.drawn(style.size, scale)),
            _ => None,
        }
    }
}

/// Text from its top-left corner, laid out as one block, a line for each line break.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub at: Vector2,
    /// With each line break as `\n`.
    pub text: String,
    /// Its em in capture pixels: a size's font as placed, or any once resized by its handles.
    pub font: f32,
    /// How wide and tall it is laid out with an em of one capture pixel, as it scales with its
    /// font, so it can be hit, cropped, outlined and resized without laying it out again.
    per_em: Vector2,
    /// How many lines it is laid out in.
    lines: u32,
}

impl Text {
    /// The box it is laid out in.
    fn bounds(&self) -> Bounds {
        Bounds {
            min: self.at,
            max: self.at + self.per_em * self.font,
        }
    }

    /// The box it is drawn in with its halo, for a capture `scale` physical pixels per logical
    /// pixel.
    fn drawn(&self, scale: f32) -> Bounds {
        self.bounds().grown(HALO * scale)
    }

    /// With the corner of its box `across` and `down` moved `delta`, scaled from the opposite
    /// corner, which stays where it is: by as far as the corner is taken along the line from the
    /// opposite corner through it, so it keeps its aspect and its lines. No line is made less
    /// than 6 pixels tall.
    fn resized(&self, across: Side, down: Side, delta: Vector2) -> Text {
        let Bounds { min, max } = self.bounds();
        // Along one axis, where the corner taken is and where the opposite one is.
        let ends = |side: Side, min: f32, max: f32| match side {
            Side::Min => (min, max),
            _ => (max, min),
        };
        let ((corner_x, still_x), (corner_y, still_y)) =
            (ends(across, min.X, max.X), ends(down, min.Y, max.Y));
        let diagonal = Vector2 {
            X: corner_x - still_x,
            Y: corner_y - still_y,
        };
        let along = (diagonal + delta).dot(&diagonal) / diagonal.length_squared();
        let smallest = SMALLEST_LINE * self.lines as f32 / self.per_em.Y;
        let font = (self.font * along).max(smallest);
        // Its font as it was, it is left exactly where it was, with no rounding from the far
        // corner and back.
        if font == self.font {
            return self.clone();
        }
        let extent = self.per_em * font;
        // The top-left, from the opposite corner where it was.
        let start = |side: Side, still: f32, extent: f32| match side {
            Side::Min => still - extent,
            _ => still,
        };
        Text {
            at: Vector2 {
                X: start(across, still_x, extent.X),
                Y: start(down, still_y, extent.Y),
            },
            font,
            ..self.clone()
        }
    }

    /// "Hi" on one line at `at` with an em of `font`, laid out 2 ems wide and one and a half
    /// tall, for tests outside this module, which cannot lay text out.
    #[cfg(test)]
    pub fn hi(at: Vector2, font: f32) -> Text {
        Text {
            at,
            text: "Hi".to_owned(),
            font,
            per_em: Vector2 { X: 2.0, Y: 1.5 },
            lines: 1,
        }
    }
}

/// A filled circle centred on `centre`, holding `count`, its place among the badges on the
/// capture in the order they were placed, from 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Badge {
    pub centre: Vector2,
    pub count: u32,
}

impl Badge {
    /// How wide and tall it is in `size`, before the halo, for a capture `scale` physical pixels
    /// per logical pixel: round, with room for two digits, and wider by a digit for each beyond.
    pub fn extent(&self, size: Size, scale: f32) -> Vector2 {
        let diameter = size.pixels(scale);
        let digits = self.count.to_string().len();
        let beyond = digits.saturating_sub(2) as f32;
        Vector2 {
            X: diameter + beyond * DIGIT_ADVANCE * Badge::font(size, scale),
            Y: diameter,
        }
    }

    /// The em of its digits in `size`, for a capture `scale` physical pixels per logical pixel.
    fn font(size: Size, scale: f32) -> f32 {
        size.pixels(scale) * DIGIT_EM
    }

    /// The centres of the round ends it is drawn between in `size`, for a capture `scale`
    /// physical pixels per logical pixel: both its centre while it is round.
    fn ends(&self, size: Size, scale: f32) -> (Vector2, Vector2) {
        let extent = self.extent(size, scale);
        let half = Vector2 {
            X: (extent.X - extent.Y) / 2.0,
            Y: 0.0,
        };
        (self.centre - half, self.centre + half)
    }

    /// The box it fills in `size`, before the halo, for a capture `scale` physical pixels per
    /// logical pixel.
    fn bounds(&self, size: Size, scale: f32) -> Bounds {
        let half = self.extent(size, scale) / 2.0;
        Bounds {
            min: self.centre - half,
            max: self.centre + half,
        }
    }

    /// The box it is drawn in at `size` with its halo, for a capture `scale` physical pixels per
    /// logical pixel.
    fn drawn(&self, size: Size, scale: f32) -> Bounds {
        self.bounds(size, scale).grown(HALO * scale)
    }
}

/// The count the next badge placed among `annotations` holds: one more than the highest there.
pub fn next_count(annotations: &[Annotation]) -> u32 {
    annotations
        .iter()
        .filter_map(|annotation| match annotation {
            Annotation::Badge(badge, _) => Some(badge.count),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        + 1
}

/// Removes the annotation at `index` from `annotations`, renumbering the badges placed after it
/// so they still read 1 to n.
pub fn remove(annotations: &mut Vec<Annotation>, index: usize) {
    annotations.remove(index);
    renumber(annotations);
}

/// Gives the badges among `annotations` the counts 1 to n in the order they were placed.
fn renumber(annotations: &mut [Annotation]) {
    let badges = annotations
        .iter_mut()
        .filter_map(|annotation| match annotation {
            Annotation::Badge(badge, _) => Some(badge),
            _ => None,
        });
    for (badge, count) in badges.zip(1..) {
        badge.count = count;
    }
}

/// A freehand stroke, as the points it passes through in turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    points: Vec<Vector2>,
}

impl Stroke {
    /// Each pair of its points in turn, as segments, or its one point as a segment of no length.
    fn segments(&self) -> impl Iterator<Item = (Vector2, Vector2)> {
        let only = (self.points.len() == 1).then(|| (self.points[0], self.points[0]));
        self.points
            .windows(2)
            .map(|pair| (pair[0], pair[1]))
            .chain(only)
    }

    /// From the least of its points' coordinates to the greatest.
    fn extent(&self) -> Bounds {
        let first = self.points[0];
        self.points.iter().fold(
            Bounds {
                min: first,
                max: first,
            },
            |bounds, point| Bounds {
                min: Vector2 {
                    X: bounds.min.X.min(point.X),
                    Y: bounds.min.Y.min(point.Y),
                },
                max: Vector2 {
                    X: bounds.max.X.max(point.X),
                    Y: bounds.max.Y.max(point.Y),
                },
            },
        )
    }

    /// The stroke a drag along `raw` places, smoothed over points `spacing` capture pixels apart,
    /// once the pointer has gone 4 pixels from where it started.
    pub fn from_drag(raw: &[Vector2], spacing: f32) -> Option<Stroke> {
        let &first = raw.first()?;
        raw.iter()
            .any(|&point| (point - first).length() >= LEAST_DRAG)
            .then(|| Stroke::smoothed(raw, spacing))
    }

    /// The stroke the pointer's path `raw` draws, from its first point to its last: the path
    /// taken at points `spacing` capture pixels apart, each averaged with the few before it. The
    /// last few average fewer and fewer, so the stroke ends where the pointer is.
    pub fn smoothed(raw: &[Vector2], spacing: f32) -> Stroke {
        let samples = resampled(raw, spacing);
        let mean = |points: &[Vector2]| {
            points
                .iter()
                .fold(Vector2::zero(), |sum, &point| sum + point)
                / points.len() as f32
        };
        let full =
            (0..samples.len()).map(|end| mean(&samples[(end + 1).saturating_sub(SMOOTHING)..=end]));
        let last = SMOOTHING.min(samples.len());
        let ending = (1..last)
            .rev()
            .map(|count| mean(&samples[samples.len() - count..]));
        Stroke {
            points: full.chain(ending).collect(),
        }
    }
}

/// The path through `raw` taken at points `spacing` apart along it, from its first point to its
/// last. The last is between one and one and a half spacings after the one before it, so the
/// path ends where `raw` does with no stub.
fn resampled(raw: &[Vector2], spacing: f32) -> Vec<Vector2> {
    let Some(&first) = raw.first() else {
        return Vec::new();
    };
    let mut samples = vec![first];
    // How far the last sample is behind the end of the segment just walked.
    let mut since = 0.0;
    for pair in raw.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let length = (end - start).length();
        let mut along = spacing - since;
        while along <= length {
            samples.push(start + (end - start) * (along / length));
            along += spacing;
        }
        since = length - (along - spacing);
    }
    let end = raw[raw.len() - 1];
    if since > 0.0 {
        if since < spacing / 2.0 && samples.len() > 1 {
            samples.pop();
        }
        samples.push(end);
    }
    samples
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrow {
    pub tail: Vector2,
    /// Where the head's tip is, or with no head, where the line ends.
    pub head: Vector2,
    bend: Bend,
    /// Which of its ends carry a head.
    heads: Head,
}

/// Where an arrow's middle handle is from its tail, as fractions of the line from its tail to its
/// head, so moving an end keeps the bend in proportion.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bend {
    /// Along the line, towards the head.
    along: f32,
    /// Across the line, to its right, looking from the tail.
    across: f32,
}

impl Bend {
    /// A straight arrow's: halfway along, and not across.
    const STRAIGHT: Bend = Bend {
        along: 0.5,
        across: 0.0,
    };
}

impl Arrow {
    /// From `tail` to `head`, with its middle on the line halfway between them and a head at the
    /// tip.
    pub fn straight(tail: Vector2, head: Vector2) -> Arrow {
        Arrow {
            tail,
            head,
            bend: Bend::STRAIGHT,
            heads: Head::One,
        }
    }

    /// The arrow a drag from `tail` to `head` places, if it is long enough to place one.
    pub fn from_drag(tail: Vector2, head: Vector2) -> Option<Arrow> {
        ((head - tail).length() >= LEAST_DRAG).then(|| Arrow::straight(tail, head))
    }

    /// Which of its ends carry a head.
    pub fn heads(&self) -> Head {
        self.heads
    }

    /// With `heads` at its ends.
    fn headed(self, heads: Head) -> Arrow {
        Arrow { heads, ..self }
    }

    /// Where its middle handle is: the point its curve passes through halfway.
    pub fn middle(&self) -> Vector2 {
        let line = self.head - self.tail;
        self.tail + line * self.bend.along + right(line) * self.bend.across
    }

    /// Bent so its curve passes through `middle` halfway. One with no length has no bend to keep,
    /// and is left as it was.
    fn bent_through(&self, middle: Vector2) -> Arrow {
        let line = self.head - self.tail;
        let length_squared = line.length_squared();
        if length_squared == 0.0 {
            return *self;
        }
        let off = middle - self.tail;
        Arrow {
            bend: Bend {
                along: off.dot(&line) / length_squared,
                across: off.dot(&right(line)) / length_squared,
            },
            ..*self
        }
    }

    /// Straight if its middle is within 4 screen pixels of the line through its ends at `zoom`
    /// screen pixels per capture pixel, even beyond an end, and otherwise as it is.
    fn straightened(&self, zoom: f32) -> Arrow {
        let off = self.bend.across.abs() * (self.head - self.tail).length();
        if off * zoom <= STRAIGHTENING_REACH {
            Arrow::straight(self.tail, self.head).headed(self.heads)
        } else {
            *self
        }
    }

    /// The curve it is drawn along, from its tail through its middle to its head's tip.
    fn curve(&self) -> Curve {
        Curve {
            start: self.tail,
            control: self.middle() * 2.0 - (self.tail + self.head) / 2.0,
            end: self.head,
        }
    }

    /// How it is drawn with a stroke `stroke` wide, before the halo, with each head pointing the
    /// way its curve heads at that end: the tip's first. None if it has no length, and so draws
    /// nothing.
    fn outline(&self, stroke: f32) -> Option<Outline> {
        let curve = self.curve();
        let (at_tip, from_tail) = (curve.end_direction()?, curve.start_direction()?);
        let mut ends = Vec::new();
        if self.heads.at_tip() {
            ends.push((self.head, at_tip));
        }
        if self.heads.at_tail() {
            ends.push((self.tail, Vector2::zero() - from_tail));
        }
        // How far from the tip the curve gets, which for a straight arrow is its length.
        let farthest = curve
            .points()
            .map(|point| (point - self.head).length())
            .fold(0.0, f32::max);
        // A short arrow is all head, or with two, two heads meeting halfway.
        let head_length = (HEAD_LENGTH * stroke).min(farthest / ends.len().max(1) as f32);
        let half_width = head_length * HEAD_HALF_WIDTH / HEAD_LENGTH;
        let heads = ends
            .iter()
            .map(|&(tip, direction)| {
                let base = tip - direction * head_length;
                let across = right(direction);
                HeadOutline {
                    tip,
                    corners: [base + across * half_width, base - across * half_width],
                }
            })
            .collect();
        // The shaft runs halfway into each head, so no seam shows where they meet.
        let into = head_length / 2.0;
        let mut shaft = curve;
        if self.heads.at_tip() {
            shaft = shaft.until(shaft.last_at(into));
        }
        if self.heads.at_tail() {
            let back = shaft.reversed();
            shaft = back.until(back.last_at(into)).reversed();
        }
        // An arrow all head has none.
        let shaft = (farthest > head_length * ends.len() as f32).then_some(shaft);
        Some(Outline { heads, shaft })
    }
}

/// An arrow as it is drawn: a shaft along its curve, and a head at each end that carries one.
struct Outline {
    heads: Vec<HeadOutline>,
    /// The part of the curve the shaft is drawn along, if the arrow is long enough to have one
    /// beside its heads.
    shaft: Option<Curve>,
}

/// A head as it is drawn: from its tip to two corners.
struct HeadOutline {
    tip: Vector2,
    corners: [Vector2; 2],
}

/// A quadratic Bézier from `start` to `end`, pulled towards `control`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Curve {
    start: Vector2,
    control: Vector2,
    end: Vector2,
}

impl Curve {
    /// Where it is at `t`, from 0 at its start to 1 at its end.
    fn at(&self, t: f32) -> Vector2 {
        // In powers of t, so the points of a curve along a line lie exactly on it.
        let pull = self.control - self.start;
        let bow = self.start - self.control * 2.0 + self.end;
        self.start + pull * (2.0 * t) + bow * (t * t)
    }

    /// Its part from its start to `t`.
    fn until(&self, t: f32) -> Curve {
        Curve {
            start: self.start,
            control: self.start + (self.control - self.start) * t,
            end: self.at(t),
        }
    }

    /// The points it is taken at as straight pieces, from its start to its end, near enough
    /// together that the pieces stray from it by under a pixel for any bend up to thousands of
    /// pixels.
    fn points(&self) -> impl Iterator<Item = Vector2> {
        let curve = *self;
        (0..=CURVE_PIECES).map(move |piece| curve.at(piece as f32 / CURVE_PIECES as f32))
    }

    /// The same curve run from its end to its start.
    fn reversed(&self) -> Curve {
        Curve {
            start: self.end,
            control: self.control,
            end: self.start,
        }
    }

    /// Its points in turn, as segments.
    fn segments(&self) -> impl Iterator<Item = (Vector2, Vector2)> {
        self.points().zip(self.points().skip(1))
    }

    /// The way it heads at its start, one pixel long. None if it has no length.
    fn start_direction(&self) -> Option<Vector2> {
        // Towards its control point, or where that is on its start, straight towards its end.
        unit([self.control - self.start, self.end - self.start])
    }

    /// The way it heads at its end, one pixel long. None if it has no length.
    fn end_direction(&self) -> Option<Vector2> {
        unit([self.end - self.control, self.end - self.start])
    }

    /// The last `t` at which it is `distance` from its end, or 0 if it is never that far.
    fn last_at(&self, distance: f32) -> f32 {
        let far = |t: f32| (self.at(t) - self.end).length() >= distance;
        let pieces = CURVE_PIECES as f32;
        let Some(piece) = (0..CURVE_PIECES)
            .rev()
            .find(|&piece| far(piece as f32 / pieces))
        else {
            return 0.0;
        };
        // Halving the piece that starts that far away and ends nearer.
        let (mut low, mut high) = (piece as f32 / pieces, (piece + 1) as f32 / pieces);
        for _ in 0..HALVINGS {
            let middle = (low + high) / 2.0;
            if far(middle) {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }
}

/// The first of `ways` with any length, one pixel long: each after the first is the way to fall
/// back on where the one before it has none.
fn unit(ways: [Vector2; 2]) -> Option<Vector2> {
    ways.into_iter()
        .map(|way| (way, way.length()))
        .find(|&(_, length)| length > 0.0)
        .map(|(way, length)| way / length)
}

/// A rectangle in capture pixels, from its top-left corner to its bottom-right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vector2,
    pub max: Vector2,
}

impl Bounds {
    /// The rectangle a drag between two opposite corners spans, in either direction, if it is at
    /// least 4 pixels across and 4 down.
    pub fn from_drag(from: Vector2, to: Vector2) -> Option<Bounds> {
        let bounds = Bounds::spanned(from, to);
        let size = bounds.max - bounds.min;
        (size.X >= LEAST_DRAG && size.Y >= LEAST_DRAG).then_some(bounds)
    }

    /// The rectangle a drag between two opposite corners spans, in either direction, however
    /// thin.
    pub fn spanned(from: Vector2, to: Vector2) -> Bounds {
        Bounds {
            min: Vector2 {
                X: from.X.min(to.X),
                Y: from.Y.min(to.Y),
            },
            max: Vector2 {
                X: from.X.max(to.X),
                Y: from.Y.max(to.Y),
            },
        }
    }

    fn moved(&self, delta: Vector2) -> Bounds {
        Bounds {
            min: self.min + delta,
            max: self.max + delta,
        }
    }

    /// With `by` more on every side.
    fn grown(&self, by: f32) -> Bounds {
        let by = Vector2 { X: by, Y: by };
        Bounds {
            min: self.min - by,
            max: self.max + by,
        }
    }

    /// With the edges a handle `across` and `down` is on moved `delta`, flipped back to
    /// top-left and bottom-right if that turned it inside out.
    pub fn adjusted(&self, across: Side, down: Side, delta: Vector2) -> Bounds {
        let (mut min, mut max) = (self.min, self.max);
        match across {
            Side::Min => min.X += delta.X,
            Side::Middle => {}
            Side::Max => max.X += delta.X,
        }
        match down {
            Side::Min => min.Y += delta.Y,
            Side::Middle => {}
            Side::Max => max.Y += delta.Y,
        }
        Bounds {
            min: Vector2 {
                X: min.X.min(max.X),
                Y: min.Y.min(max.Y),
            },
            max: Vector2 {
                X: min.X.max(max.X),
                Y: min.Y.max(max.Y),
            },
        }
    }

    /// Its corners and edge midpoints, clockwise from the top-left, with where each is across and
    /// down.
    pub fn handles(&self) -> impl Iterator<Item = (Side, Side, Vector2)> {
        let middle = (self.min + self.max) / 2.0;
        let at = |side: Side, min: f32, middle: f32, max: f32| match side {
            Side::Min => min,
            Side::Middle => middle,
            Side::Max => max,
        };
        let bounds = *self;
        [
            (Side::Min, Side::Min),
            (Side::Middle, Side::Min),
            (Side::Max, Side::Min),
            (Side::Max, Side::Middle),
            (Side::Max, Side::Max),
            (Side::Middle, Side::Max),
            (Side::Min, Side::Max),
            (Side::Min, Side::Middle),
        ]
        .into_iter()
        .map(move |(across, down)| {
            let position = Vector2 {
                X: at(across, bounds.min.X, middle.X, bounds.max.X),
                Y: at(down, bounds.min.Y, middle.Y, bounds.max.Y),
            };
            (across, down, position)
        })
    }

    /// The handle the pointer at `at` in `view`'s image area is on, if any, as where it is across
    /// and down.
    pub fn handle_at(&self, at: Vector2, view: &View) -> Option<(Side, Side)> {
        nearest_handle(
            self.handles()
                .map(|(across, down, position)| ((across, down), position)),
            at,
            view,
        )
    }

    /// Kept inside an image `size` capture pixels in size.
    pub fn within(&self, size: Vector2) -> Bounds {
        let clamp = |point: Vector2| Vector2 {
            X: point.X.clamp(0.0, size.X),
            Y: point.Y.clamp(0.0, size.Y),
        };
        Bounds {
            min: clamp(self.min),
            max: clamp(self.max),
        }
    }

    /// The whole pixels from the one its top-left corner is on to the one its bottom-right corner
    /// is on, both included: what a blur or a Solid rectangle covers.
    pub fn covered(&self) -> Bounds {
        let floor = |point: Vector2| Vector2 {
            X: point.X.floor(),
            Y: point.Y.floor(),
        };
        Bounds {
            min: floor(self.min),
            max: floor(self.max) + Vector2::one(),
        }
    }

    /// The pixel its top-left corner is on, as a blur's blocks start there, inside the image or
    /// not.
    pub fn first_pixel(&self) -> [i64; 2] {
        [self.min.X.floor() as i64, self.min.Y.floor() as i64]
    }

    /// The pixels of an image `width` by `height` these bounds cover: from the one the top-left
    /// corner is on to the one the bottom-right corner is on, both included, as far as they lie in
    /// the image. None if none do.
    pub fn pixels(&self, width: u32, height: u32) -> Option<Region> {
        let start = |at: f32, end: u32| (at.floor().max(0.0) as u32).min(end);
        let stop = |at: f32, end: u32| ((at.floor() + 1.0).max(0.0) as u32).min(end);
        let region = Region {
            left: start(self.min.X, width),
            top: start(self.min.Y, height),
            right: stop(self.max.X, width),
            bottom: stop(self.max.Y, height),
        };
        (region.left < region.right && region.top < region.bottom).then_some(region)
    }
}

/// `region` of `pixels`, an image `width` pixels wide of four channels each, as blocks `block`
/// pixels square on a grid from `origin`, the pixel a blur's top-left corner is on, each filled
/// with the average of the pixels it covers in the region. The origin may lie outside the image,
/// so a blur the image's edge cuts off keeps its blocks where they were. The channels are averaged
/// as they are, which is exact for premultiplied pixels, so a translucent edge blurs as it looks.
pub fn blur_blocks(
    pixels: &[u8],
    width: u32,
    region: Region,
    origin: [i64; 2],
    block: usize,
) -> Vec<u8> {
    let (across, down) = (region.width() as usize, region.height() as usize);
    let (left, top, width) = (region.left as usize, region.top as usize, width as usize);
    // The blocks along one axis of the region, as ranges of it: the first starts `into` pixels
    // into its block, and the last stops at the region's edge.
    let blocks_along = |length: usize, start: u32, origin: i64| {
        let into = (start as i64 - origin).rem_euclid(block as i64) as usize;
        (0..length + into)
            .step_by(block)
            .map(move |at| at.saturating_sub(into)..(at + block - into).min(length))
    };
    let mut blocks = vec![0; across * down * 4];
    for rows in blocks_along(down, region.top, origin[1]) {
        for columns in blocks_along(across, region.left, origin[0]) {
            let mut sum = [0u32; 4];
            for y in rows.clone() {
                let at = ((top + y) * width + left) * 4;
                for pixel in pixels[at + columns.start * 4..at + columns.end * 4]
                    .as_chunks::<4>()
                    .0
                {
                    for (total, &channel) in sum.iter_mut().zip(pixel) {
                        *total += channel as u32;
                    }
                }
            }
            let count = (rows.len() * columns.len()) as u32;
            let average = sum.map(|total| ((total + count / 2) / count) as u8);
            for y in rows.clone() {
                let at = (y * across + columns.start) * 4;
                for pixel in blocks[at..at + columns.len() * 4].as_chunks_mut::<4>().0 {
                    pixel.copy_from_slice(&average);
                }
            }
        }
    }
    blocks
}

/// Which annotation among `annotations` the pointer at `at` in `view`'s image area is on, as its
/// index, for a capture `scale` physical pixels per logical pixel. The last placed is on top,
/// except that blurs are beneath every arrow, rectangle and text.
pub fn hit(annotations: &[Annotation], at: Vector2, view: &View, scale: f32) -> Option<usize> {
    let topmost = |blurs: bool| {
        annotations.iter().rposition(|annotation| {
            matches!(annotation, Annotation::Blur(..)) == blurs
                && annotation.covers(at, view, scale)
        })
    };
    topmost(false).or_else(|| topmost(true))
}

/// What cropping a capture `scale` physical pixels per logical pixel to `region` leaves of
/// `annotations`, in the order they were placed: each that draws anything inside the region,
/// moved so the region's top-left is the capture's, with the badges left reading 1 to n.
pub fn crop(annotations: &[Annotation], region: Region, scale: f32) -> Vec<Annotation> {
    let origin = Vector2 {
        X: region.left as f32,
        Y: region.top as f32,
    };
    let mut cropped: Vec<Annotation> = annotations
        .iter()
        .filter(|annotation| annotation.draws_in(region, scale))
        .map(|annotation| annotation.moved(Vector2::zero() - origin))
        .collect();
    renumber(&mut cropped);
    cropped
}

/// One of the handles a selected annotation shows: an end or the middle of an arrow, a corner or
/// edge midpoint of a rectangle or blur, or a corner of a text's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Tail,
    Head,
    /// The middle of an arrow, on its curve, which bends it.
    Bend,
    /// Where it is on the bounds across and down.
    Bounds {
        across: Side,
        down: Side,
    },
}

/// Which of `handles`, each at a capture point, the pointer at `at` in `view`'s image area is on:
/// the nearest within the same few screen pixels at any zoom.
fn nearest_handle<H>(
    handles: impl IntoIterator<Item = (H, Vector2)>,
    at: Vector2,
    view: &View,
) -> Option<H> {
    handles
        .into_iter()
        .map(|(handle, position)| {
            let off = view.to_area(position) - at;
            (handle, off.X.abs().max(off.Y.abs()))
        })
        .filter(|&(_, distance)| distance <= HANDLE_REACH)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(handle, _)| handle)
}

/// Where a handle is along one axis of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Min,
    Middle,
    Max,
}

impl Annotation {
    /// The handles it shows when selected, and where they are in capture pixels.
    pub fn handles(&self) -> Vec<(Handle, Vector2)> {
        match self {
            // The ends first, so a short arrow's ends are picked up before its middle.
            Annotation::Arrow(arrow, _) => vec![
                (Handle::Tail, arrow.tail),
                (Handle::Head, arrow.head),
                (Handle::Bend, arrow.middle()),
            ],
            Annotation::Rectangle(bounds, ..) | Annotation::Blur(bounds, _) => bounds
                .handles()
                .map(|(across, down, at)| (Handle::Bounds { across, down }, at))
                .collect(),
            // Only its corners, as it keeps its aspect.
            Annotation::Text(text, _) => text
                .bounds()
                .handles()
                .filter(|&(across, down, _)| across != Side::Middle && down != Side::Middle)
                .map(|(across, down, at)| (Handle::Bounds { across, down }, at))
                .collect(),
            Annotation::Stroke(..) | Annotation::Badge(..) => Vec::new(),
        }
    }

    /// The handle the pointer at `at` in `view`'s image area is on, if any: the nearest within
    /// the same few screen pixels at any zoom.
    pub fn handle_at(&self, at: Vector2, view: &View) -> Option<Handle> {
        nearest_handle(self.handles(), at, view)
    }

    /// Moved `delta` capture pixels.
    pub fn moved(&self, delta: Vector2) -> Annotation {
        match self {
            Annotation::Arrow(arrow, style) => Annotation::Arrow(
                Arrow {
                    tail: arrow.tail + delta,
                    head: arrow.head + delta,
                    ..*arrow
                },
                *style,
            ),
            Annotation::Rectangle(bounds, style, fill) => {
                Annotation::Rectangle(bounds.moved(delta), *style, *fill)
            }
            Annotation::Text(text, colour) => Annotation::Text(
                Text {
                    at: text.at + delta,
                    ..text.clone()
                },
                *colour,
            ),
            Annotation::Stroke(stroke, style) => Annotation::Stroke(
                Stroke {
                    points: stroke.points.iter().map(|&point| point + delta).collect(),
                },
                *style,
            ),
            Annotation::Badge(badge, style) => Annotation::Badge(
                Badge {
                    centre: badge.centre + delta,
                    ..*badge
                },
                *style,
            ),
            Annotation::Blur(bounds, level) => Annotation::Blur(bounds.moved(delta), *level),
        }
    }

    /// As it is on the capture resized `by`, a fraction of its dimensions: every point, size, em
    /// and level shrunk by the same fraction, so it looks as it did on the smaller capture, and a
    /// blur keeps as many blocks.
    pub fn resized(&self, by: f32) -> Annotation {
        let bounds = |bounds: &Bounds| Bounds {
            min: bounds.min * by,
            max: bounds.max * by,
        };
        let style = |style: &Style| Style {
            size: style.size.resized(by),
            ..*style
        };
        match self {
            // Its bend is in proportion to its length, so it stays as bent.
            Annotation::Arrow(arrow, shape) => Annotation::Arrow(
                Arrow {
                    tail: arrow.tail * by,
                    head: arrow.head * by,
                    ..*arrow
                },
                style(shape),
            ),
            Annotation::Rectangle(edges, shape, fill) => {
                Annotation::Rectangle(bounds(edges), style(shape), *fill)
            }
            Annotation::Text(text, colour) => Annotation::Text(
                Text {
                    at: text.at * by,
                    font: text.font * by,
                    ..text.clone()
                },
                *colour,
            ),
            Annotation::Stroke(stroke, shape) => Annotation::Stroke(
                Stroke {
                    points: stroke.points.iter().map(|&point| point * by).collect(),
                },
                style(shape),
            ),
            Annotation::Badge(badge, shape) => Annotation::Badge(
                Badge {
                    centre: badge.centre * by,
                    ..*badge
                },
                style(shape),
            ),
            Annotation::Blur(edges, level) => Annotation::Blur(bounds(edges), level.resized(by)),
        }
    }

    /// With `handle` moved `delta` capture pixels: that end of an arrow, its middle, which bends
    /// it through where the middle is moved to, the edges of a rectangle or blur the handle is
    /// on, or that corner of a text, which scales it from the opposite corner. A rectangle
    /// dragged inside out flips.
    pub fn adjusted(&self, handle: Handle, delta: Vector2) -> Annotation {
        match (self, handle) {
            (&Annotation::Arrow(arrow, style), Handle::Tail) => Annotation::Arrow(
                Arrow {
                    tail: arrow.tail + delta,
                    ..arrow
                },
                style,
            ),
            (&Annotation::Arrow(arrow, style), Handle::Head) => Annotation::Arrow(
                Arrow {
                    head: arrow.head + delta,
                    ..arrow
                },
                style,
            ),
            // Not moved, it is left exactly as it was, with no rounding of its bend.
            (&Annotation::Arrow(arrow, style), Handle::Bend) if delta != Vector2::zero() => {
                Annotation::Arrow(arrow.bent_through(arrow.middle() + delta), style)
            }
            (Annotation::Rectangle(bounds, style, fill), Handle::Bounds { across, down }) => {
                Annotation::Rectangle(bounds.adjusted(across, down, delta), *style, *fill)
            }
            (Annotation::Blur(bounds, level), Handle::Bounds { across, down }) => {
                Annotation::Blur(bounds.adjusted(across, down, delta), *level)
            }
            // Not moved, it is left exactly as it was, with no rounding of where it is.
            (Annotation::Text(text, colour), Handle::Bounds { across, down })
                if delta != Vector2::zero() =>
            {
                Annotation::Text(text.resized(across, down, delta), *colour)
            }
            _ => self.clone(),
        }
    }

    /// With `handle` dropped `delta` capture pixels from where it was picked up, at `zoom` screen
    /// pixels per capture pixel: as `adjusted` leaves it, except that an arrow's middle dropped
    /// within 4 screen pixels of the line through its ends straightens it. A drop where it was
    /// picked up leaves it exactly as it was, however slightly bent.
    pub fn dropped(&self, handle: Handle, delta: Vector2, zoom: f32) -> Annotation {
        match (self.adjusted(handle, delta), handle) {
            (Annotation::Arrow(arrow, style), Handle::Bend) if delta != Vector2::zero() => {
                Annotation::Arrow(arrow.straightened(zoom), style)
            }
            (adjusted, _) => adjusted,
        }
    }

    /// Whether the pointer at `at` in `view`'s image area is on this annotation, in screen pixels.
    fn covers(&self, at: Vector2, view: &View, scale: f32) -> bool {
        // How far the stroke and halo reach beyond the annotation's line, on screen.
        let reach = (self.stroke(scale) / 2.0 + HALO * scale) * view.scale();
        match self {
            // Every pixel it covers, Solid.
            Annotation::Rectangle(bounds, _, Fill::Solid) => {
                let covered = bounds.covered();
                let (min, max) = (view.to_area(covered.min), view.to_area(covered.max));
                at.X >= min.X && at.X < max.X && at.Y >= min.Y && at.Y < max.Y
            }
            // Anywhere inside it, out to its halo, tinted or not.
            Annotation::Rectangle(bounds, _, Fill::Outline | Fill::Tint) => {
                let (min, max) = (view.to_area(bounds.min), view.to_area(bounds.max));
                at.X >= min.X - reach
                    && at.X <= max.X + reach
                    && at.Y >= min.Y - reach
                    && at.Y <= max.Y + reach
            }
            // Along its curve, not the straight line between its ends.
            Annotation::Arrow(arrow, _) => near_line(
                arrow.curve().segments(),
                at,
                view,
                reach.max(LINE_TOLERANCE),
            ),
            Annotation::Stroke(stroke, _) => {
                near_line(stroke.segments(), at, view, reach.max(LINE_TOLERANCE))
            }
            // Anywhere in its box with its halo, between the letters too.
            Annotation::Text(text, _) => {
                let drawn = text.drawn(scale);
                let (min, max) = (view.to_area(drawn.min), view.to_area(drawn.max));
                at.X >= min.X && at.X <= max.X && at.Y >= min.Y && at.Y <= max.Y
            }
            // Anywhere inside it and its halo.
            Annotation::Badge(badge, style) => {
                let (start, end) = badge.ends(style.size, scale);
                let radius = badge.extent(style.size, scale).Y / 2.0 + HALO * scale;
                let (start, end) = (view.to_area(start), view.to_area(end));
                distance_to_segment(at, start, end) <= radius * view.scale()
            }
            // Every pixel from the one its top-left corner is on to the one its bottom-right
            // corner is on, as it is drawn.
            Annotation::Blur(bounds, _) => {
                let covered = bounds.covered();
                let (min, max) = (view.to_area(covered.min), view.to_area(covered.max));
                at.X >= min.X && at.X < max.X && at.Y >= min.Y && at.Y < max.Y
            }
        }
    }

    /// Whether any of what it draws, for a capture `scale` physical pixels per logical pixel,
    /// lies inside `region`.
    fn draws_in(&self, region: Region, scale: f32) -> bool {
        let min = Vector2 {
            X: region.left as f32,
            Y: region.top as f32,
        };
        let max = Vector2 {
            X: region.right as f32,
            Y: region.bottom as f32,
        };
        let overlaps = |from: Vector2, to: Vector2| {
            from.X < max.X && to.X > min.X && from.Y < max.Y && to.Y > min.Y
        };
        let grown = |by: f32| Vector2 { X: by, Y: by };
        let (stroke, halo) = (self.stroke(scale), HALO * scale);
        match self {
            // Every pixel it covers, Solid.
            Annotation::Rectangle(bounds, _, Fill::Solid) => {
                let covered = bounds.covered();
                overlaps(covered.min, covered.max)
            }
            // Its outline, with the stroke and halo either side of it, and its tint inside it.
            Annotation::Rectangle(bounds, _, Fill::Tint) => {
                let drawn = bounds.grown(stroke / 2.0 + halo);
                overlaps(drawn.min, drawn.max)
            }
            // Its outline, with the stroke and halo either side of it: a rectangle around the
            // whole region draws nothing in it.
            Annotation::Rectangle(bounds, _, Fill::Outline) => {
                let reach = grown(stroke / 2.0 + halo);
                let (outer_min, outer_max) = (bounds.min - reach, bounds.max + reach);
                let (inner_min, inner_max) = (bounds.min + reach, bounds.max - reach);
                let around = inner_min.X <= min.X
                    && inner_min.Y <= min.Y
                    && inner_max.X >= max.X
                    && inner_max.Y >= max.Y;
                overlaps(outer_min, outer_max) && !around
            }
            // Its shaft along its curve with the stroke and halo either side, or a head with the
            // halo round it.
            Annotation::Arrow(arrow, _) => {
                let Some(outline) = arrow.outline(stroke) else {
                    return false;
                };
                let reach = grown(stroke / 2.0 + halo);
                let shaft = outline.shaft.is_some_and(|shaft| {
                    shaft
                        .segments()
                        .any(|(start, end)| segment_meets(start, end, min - reach, max + reach))
                });
                // The box around a head's tip and corners.
                let head = outline.heads.iter().any(|head| {
                    let (mut low, mut high) = (head.tip, head.tip);
                    for corner in head.corners {
                        low = Vector2 {
                            X: low.X.min(corner.X),
                            Y: low.Y.min(corner.Y),
                        };
                        high = Vector2 {
                            X: high.X.max(corner.X),
                            Y: high.Y.max(corner.Y),
                        };
                    }
                    overlaps(low - grown(halo), high + grown(halo))
                });
                shaft || head
            }
            Annotation::Text(text, _) => {
                let drawn = text.drawn(scale);
                overlaps(drawn.min, drawn.max)
            }
            Annotation::Badge(badge, style) => {
                let drawn = badge.drawn(style.size, scale);
                overlaps(drawn.min, drawn.max)
            }
            // Any of its segments with the stroke and halo either side.
            Annotation::Stroke(path, _) => {
                let reach = grown(stroke / 2.0 + halo);
                path.segments()
                    .any(|(start, end)| segment_meets(start, end, min - reach, max + reach))
            }
            // Every pixel from the one its top-left corner is on to the one its bottom-right
            // corner is on.
            Annotation::Blur(bounds, _) => {
                let covered = bounds.covered();
                overlaps(covered.min, covered.max)
            }
        }
    }
}

/// Whether any of the segment from `start` to `end` lies inside the rectangle from `min` to
/// `max`.
fn segment_meets(start: Vector2, end: Vector2, min: Vector2, max: Vector2) -> bool {
    // The part of the segment, from 0 at its start to 1 at its end, on the inner side of each
    // edge in turn.
    let along = end - start;
    let (mut from, mut to) = (0.0f32, 1.0f32);
    for (towards, room) in [
        (-along.X, start.X - min.X),
        (along.X, max.X - start.X),
        (-along.Y, start.Y - min.Y),
        (along.Y, max.Y - start.Y),
    ] {
        if towards == 0.0 {
            // Parallel to this edge, so wholly on one side of it.
            if room <= 0.0 {
                return false;
            }
        } else if towards < 0.0 {
            from = from.max(room / towards);
        } else {
            to = to.min(room / towards);
        }
    }
    from < to
}

/// Whether the pointer at `at` in `view`'s image area is within `within` screen pixels of any of
/// `segments`, in capture pixels.
fn near_line(
    mut segments: impl Iterator<Item = (Vector2, Vector2)>,
    at: Vector2,
    view: &View,
    within: f32,
) -> bool {
    segments.any(|(start, end)| {
        distance_to_segment(at, view.to_area(start), view.to_area(end)) <= within
    })
}

/// `along` turned a quarter to the right, as the screen shows it, with y down.
fn right(along: Vector2) -> Vector2 {
    Vector2 {
        X: -along.Y,
        Y: along.X,
    }
}

/// How far `at` is from the nearest point of the segment from `start` to `end`.
fn distance_to_segment(at: Vector2, start: Vector2, end: Vector2) -> f32 {
    let along = end - start;
    let length_squared = along.length_squared();
    let t = if length_squared == 0.0 {
        0.0
    } else {
        ((at - start).dot(&along) / length_squared).clamp(0.0, 1.0)
    };
    (at - (start + along * t)).length()
}

/// What annotations are drawn with, made once for a device context.
pub struct Pens {
    factory: ID2D1Factory,
    /// One for each colour, indexed by it, as `Colour::ALL` lists them in the order they are
    /// declared.
    brushes: Vec<ID2D1SolidColorBrush>,
    /// One for each colour as `brushes` are, translucent, for a Tint.
    tints: Vec<ID2D1SolidColorBrush>,
    /// Rounds the halo's corners, so it does not spike past the head's tip.
    round: ID2D1StrokeStyle,
    /// Rounds a stroke's ends and joins, and its halo's with them.
    round_ends: ID2D1StrokeStyle,
    writer: IDWriteFactory,
}

impl Pens {
    pub fn new(context: &ID2D1DeviceContext) -> Result<Pens> {
        unsafe {
            let factory: ID2D1Factory = context.GetFactory()?;
            let joined = |cap| {
                factory.CreateStrokeStyle(
                    &D2D1_STROKE_STYLE_PROPERTIES {
                        startCap: cap,
                        endCap: cap,
                        dashCap: cap,
                        lineJoin: D2D1_LINE_JOIN_ROUND,
                        miterLimit: 1.0,
                        dashStyle: D2D1_DASH_STYLE_SOLID,
                        dashOffset: 0.0,
                    },
                    None,
                )
            };
            let round = joined(D2D1_CAP_STYLE_FLAT)?;
            let round_ends = joined(D2D1_CAP_STYLE_ROUND)?;
            let brushes = Colour::ALL
                .iter()
                .map(|colour| context.CreateSolidColorBrush(&color(colour.rgb()), None))
                .collect::<Result<_>>()?;
            let tints = Colour::ALL
                .iter()
                .map(|colour| {
                    let tint = D2D1_COLOR_F {
                        a: Fill::TINT_OPACITY,
                        ..color(colour.rgb())
                    };
                    context.CreateSolidColorBrush(&tint, None)
                })
                .collect::<Result<_>>()?;
            Ok(Pens {
                brushes,
                tints,
                factory,
                round,
                round_ends,
                writer: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            })
        }
    }

    /// The brush that paints `colour`.
    pub fn brush(&self, colour: Colour) -> &ID2D1SolidColorBrush {
        &self.brushes[colour as usize]
    }

    /// The brush that lays `colour` translucently, as a Tint does.
    fn tint(&self, colour: Colour) -> &ID2D1SolidColorBrush {
        &self.tints[colour as usize]
    }

    /// `text` laid out in bold Segoe UI with an em `font` pixels high, from its top-left corner,
    /// left to right however its runs read, a line for each line break.
    fn layout(&self, text: &str, font: f32) -> Result<IDWriteTextLayout> {
        let text: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.writer
                .CreateTextLayout(&text, &self.bold(font)?, f32::MAX, f32::MAX)
        }
    }

    /// `count` laid out in bold Segoe UI with an em `font` pixels high, from the top-left corner
    /// of a box of no size, so its overhang is where its ink is from that corner.
    fn count_layout(&self, count: u32, font: f32) -> Result<IDWriteTextLayout> {
        let text: Vec<u16> = count.to_string().encode_utf16().collect();
        unsafe {
            let format = self.bold(font)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            self.writer.CreateTextLayout(&text, &format, 0.0, 0.0)
        }
    }

    /// Bold Segoe UI with an em `font` pixels high.
    fn bold(&self, font: f32) -> Result<IDWriteTextFormat> {
        unsafe {
            self.writer.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_BOLD,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font,
                w!("en-us"),
            )
        }
    }

    /// How `text` lays out with an em `font` pixels high.
    pub fn metrics(&self, text: &str, font: f32) -> Result<DWRITE_TEXT_METRICS> {
        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { self.layout(text, font)?.GetMetrics(&mut metrics)? };
        Ok(metrics)
    }

    /// How `text` lays out with an em `font` pixels high, line by line and letter by letter, from
    /// its top-left corner.
    pub fn laid(&self, text: &str, font: f32) -> Result<Laid> {
        let layout = self.layout(text, font)?;
        // As many lines as DirectWrite breaks it into, which pasted text can make more than its
        // line breaks.
        let mut whole = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut whole)? };
        let mut metrics = vec![DWRITE_LINE_METRICS::default(); whole.lineCount as usize];
        let mut count = 0;
        unsafe { layout.GetLineMetrics(Some(&mut metrics), &mut count)? };
        metrics.truncate(count as usize);
        let (mut start, mut top) = (0, 0.0);
        let mut lines = Vec::new();
        for line in metrics {
            lines.push(Line {
                start,
                length: (line.length - line.newlineLength) as usize,
                top,
                height: line.height,
            });
            start += line.length as usize;
            top += line.height;
        }
        let length = text.encode_utf16().count();
        let mut clusters = Vec::new();
        let mut index = 0;
        while index < length {
            let (mut x, mut y) = (0.0, 0.0);
            let mut hit = DWRITE_HIT_TEST_METRICS::default();
            unsafe { layout.HitTestTextPosition(index as u32, false, &mut x, &mut y, &mut hit)? };
            let (start, length) = (hit.textPosition as usize, hit.length as usize);
            clusters.push(Cluster {
                start,
                length: length.max(1),
                left: hit.left,
                width: hit.width,
                rtl: hit.bidiLevel % 2 == 1,
            });
            index = (start + length).max(index + 1);
        }
        Ok(Laid { lines, clusters })
    }

    /// `text` with its top-left corner `at`, laid out with an em `font` capture pixels high.
    pub fn text(&self, at: Vector2, text: String, font: f32) -> Result<Text> {
        let metrics = self.metrics(&text, font)?;
        let per_em = Vector2 {
            X: metrics.width,
            Y: metrics.height,
        } / font;
        Ok(Text {
            at,
            text,
            font,
            per_em,
            lines: metrics.lineCount,
        })
    }
}

/// An sRGB colour as Direct2D takes it.
pub fn color(rgb: u32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// Draws the arrows, rectangles and text among `annotations` in the order they were placed,
/// through `context`'s transform from capture pixels, for a capture `scale` physical pixels per
/// logical pixel. Blurs are baked into the capture beneath them instead. Only between `BeginDraw`
/// and `EndDraw`.
pub fn draw(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    annotations: &[Annotation],
    scale: f32,
) -> Result<()> {
    in_grey(context, || draw_each(context, pens, annotations, scale))
}

/// Draws `text` being typed in `colour` exactly as `draw` draws it placed, with `highlight`, bands
/// from its top-left corner, filled with `lit` over its halo and behind its letters.
pub fn draw_typed(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    text: &Text,
    colour: Colour,
    scale: f32,
    highlight: &[Bounds],
    lit: &ID2D1SolidColorBrush,
) -> Result<()> {
    in_grey(context, || {
        draw_text(
            context,
            pens,
            text,
            colour,
            HALO * scale,
            Some((highlight, lit)),
        )
    })
}

/// Runs `draw` with text drawn in shades of its colour, as the output can only hold, and with no
/// coloured fringes when zoomed.
fn in_grey(context: &ID2D1DeviceContext, draw: impl FnOnce() -> Result<()>) -> Result<()> {
    let antialias = unsafe { context.GetTextAntialiasMode() };
    unsafe { context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    let drawn = draw();
    unsafe { context.SetTextAntialiasMode(antialias) };
    drawn
}

/// The drawing `draw` does, between setting the text antialiasing and restoring it.
fn draw_each(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    annotations: &[Annotation],
    scale: f32,
) -> Result<()> {
    let halo = HALO * scale;
    for annotation in annotations {
        let stroke = annotation.stroke(scale);
        match annotation {
            Annotation::Arrow(arrow, style) => {
                draw_arrow(context, pens, arrow, style.colour, stroke, halo)?
            }
            Annotation::Rectangle(bounds, style, fill) => {
                // Solid, whole pixels, so its edges are as sharp as a blur's.
                let drawn = if *fill == Fill::Solid {
                    bounds.covered()
                } else {
                    *bounds
                };
                let rect = D2D_RECT_F {
                    left: drawn.min.X,
                    top: drawn.min.Y,
                    right: drawn.max.X,
                    bottom: drawn.max.Y,
                };
                match fill {
                    Fill::Solid => {
                        unsafe { context.FillRectangle(&rect, pens.brush(style.colour)) };
                        continue;
                    }
                    // Beneath the outline, so the outline keeps its halo and full colour.
                    Fill::Tint => unsafe { context.FillRectangle(&rect, pens.tint(style.colour)) },
                    Fill::Outline => {}
                }
                // Square corners, so the halo is the same width all round.
                unsafe {
                    context.DrawRectangle(
                        &rect,
                        pens.brush(style.colour.halo()),
                        stroke + 2.0 * halo,
                        None,
                    );
                    context.DrawRectangle(&rect, pens.brush(style.colour), stroke, None);
                }
            }
            Annotation::Text(text, colour) => draw_text(context, pens, text, *colour, halo, None)?,
            Annotation::Stroke(path, style) => {
                draw_stroke(context, pens, path, style.colour, stroke, halo)?
            }
            Annotation::Badge(badge, style) => {
                draw_badge(context, pens, badge, *style, scale, halo)?
            }
            Annotation::Blur(..) => {}
        }
    }
    Ok(())
}

/// Draws `text` in `colour`, with copies in the halo's colour all round it as far as the halo
/// reaches, `halo`, beneath it, so it has an edge of that colour, and any `highlight`'s bands
/// from its top-left corner between the two in their brush.
fn draw_text(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    text: &Text,
    colour: Colour,
    halo: f32,
    highlight: Option<(&[Bounds], &ID2D1SolidColorBrush)>,
) -> Result<()> {
    let layout = pens.layout(&text.text, text.font)?;
    // Unsnapped, so each copy is exactly as far off as the halo reaches.
    let options = D2D1_DRAW_TEXT_OPTIONS_NO_SNAP;
    // About a pixel apart round the circle, so the edge has no gaps.
    let copies = (TAU * halo).ceil().max(8.0) as usize;
    for copy in 0..copies {
        let angle = TAU * copy as f32 / copies as f32;
        let off = Vector2 {
            X: angle.cos() * halo,
            Y: angle.sin() * halo,
        };
        unsafe {
            context.DrawTextLayout(text.at + off, &layout, pens.brush(colour.halo()), options)
        };
    }
    if let Some((bands, lit)) = highlight {
        for band in bands {
            let (min, max) = (text.at + band.min, text.at + band.max);
            let rect = D2D_RECT_F {
                left: min.X,
                top: min.Y,
                right: max.X,
                bottom: max.Y,
            };
            unsafe { context.FillRectangle(&rect, lit) };
        }
    }
    unsafe { context.DrawTextLayout(text.at, &layout, pens.brush(colour), options) };
    Ok(())
}

/// Draws a thin outline around the screen points `min` to `max`, a little outside them, in `edge`,
/// the handles' colour, with `context` drawing in screen pixels. Only between `BeginDraw` and
/// `EndDraw`.
pub fn draw_outline(
    context: &ID2D1DeviceContext,
    edge: &ID2D1SolidColorBrush,
    min: Vector2,
    max: Vector2,
) {
    // Through the middle of whole pixels, so it is a crisp single pixel.
    let outline = D2D_RECT_F {
        left: (min.X - OUTLINE_GAP).floor() - 0.5,
        top: (min.Y - OUTLINE_GAP).floor() - 0.5,
        right: (max.X + OUTLINE_GAP).ceil() + 0.5,
        bottom: (max.Y + OUTLINE_GAP).ceil() + 0.5,
    };
    unsafe { context.DrawRectangle(&outline, edge, 1.0, None) };
}

/// Draws handles centred on `points` as squares the same size on screen at every zoom, `fill`
/// inside an `edge`, with `context` drawing in screen pixels. Only between `BeginDraw` and
/// `EndDraw`.
pub fn draw_handles(
    context: &ID2D1DeviceContext,
    fill: &ID2D1SolidColorBrush,
    edge: &ID2D1SolidColorBrush,
    points: impl IntoIterator<Item = Vector2>,
) {
    let half = HANDLE_SIDE / 2.0;
    for at in points {
        // On whole pixels, so the edge is a crisp single pixel.
        let (left, top) = ((at.X - half).round(), (at.Y - half).round());
        let square = D2D_RECT_F {
            left,
            top,
            right: left + HANDLE_SIDE,
            bottom: top + HANDLE_SIDE,
        };
        let outline = D2D_RECT_F {
            left: left + 0.5,
            top: top + 0.5,
            right: square.right - 0.5,
            bottom: square.bottom - 0.5,
        };
        unsafe {
            context.FillRectangle(&square, fill);
            context.DrawRectangle(&outline, edge, 1.0, None);
        }
    }
}

/// Draws `badge` in `style`: a filled circle, wider with round ends once it holds three digits,
/// inside its halo, with its count centred in it in the halo's colour, so it reads on any colour.
fn draw_badge(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    badge: &Badge,
    style: Style,
    scale: f32,
    halo: f32,
) -> Result<()> {
    let bounds = badge.bounds(style.size, scale);
    // Its ends are half circles.
    let shape = |grown: f32| {
        let Bounds { min, max } = bounds.grown(grown);
        let radius = (max.Y - min.Y) / 2.0;
        D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: min.X,
                top: min.Y,
                right: max.X,
                bottom: max.Y,
            },
            radiusX: radius,
            radiusY: radius,
        }
    };
    let layout = pens.count_layout(badge.count, Badge::font(style.size, scale))?;
    // Centred on its ink, the digits as drawn, rather than on its line, which leaves room below
    // the baseline that figures never use.
    let ink = unsafe { layout.GetOverhangMetrics()? };
    let at = badge.centre
        - Vector2 {
            X: (ink.right - ink.left) / 2.0,
            Y: (ink.bottom - ink.top) / 2.0,
        };
    unsafe {
        context.FillRoundedRectangle(&shape(halo), pens.brush(style.colour.halo()));
        context.FillRoundedRectangle(&shape(0.0), pens.brush(style.colour));
        context.DrawTextLayout(
            at,
            &layout,
            pens.brush(style.colour.halo()),
            D2D1_DRAW_TEXT_OPTIONS_NO_SNAP,
        );
    }
    Ok(())
}

fn draw_stroke(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    stroke: &Stroke,
    colour: Colour,
    width: f32,
    halo: f32,
) -> Result<()> {
    let geometry = unsafe { pens.factory.CreatePathGeometry()? };
    unsafe {
        let sink = geometry.Open()?;
        sink.BeginFigure(stroke.points[0], D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddLines(&stroke.points[1..]);
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        context.DrawGeometry(
            &geometry,
            pens.brush(colour.halo()),
            width + 2.0 * halo,
            &pens.round_ends,
        );
        context.DrawGeometry(&geometry, pens.brush(colour), width, &pens.round_ends);
    }
    Ok(())
}

fn draw_arrow(
    context: &ID2D1DeviceContext,
    pens: &Pens,
    arrow: &Arrow,
    colour: Colour,
    stroke: f32,
    halo: f32,
) -> Result<()> {
    let (fill, edge) = (pens.brush(colour), pens.brush(colour.halo()));
    let Some(Outline { heads, shaft }) = arrow.outline(stroke) else {
        return Ok(());
    };

    let heads = heads
        .iter()
        .map(|head| unsafe {
            let geometry = pens.factory.CreatePathGeometry()?;
            let sink = geometry.Open()?;
            sink.BeginFigure(head.tip, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&head.corners);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            Ok(geometry)
        })
        .collect::<Result<Vec<_>>>()?;
    // The shaft's halo reaches as far beyond an end with no head as it does either side.
    let shafts = match shaft {
        Some(shaft) => {
            let behind = (!arrow.heads.at_tail())
                .then(|| shaft.start_direction().map(|way| shaft.start - way * halo))
                .flatten();
            let beyond = (!arrow.heads.at_tip())
                .then(|| shaft.end_direction().map(|way| shaft.end + way * halo))
                .flatten();
            Some((
                curve_geometry(pens, behind, shaft, beyond)?,
                curve_geometry(pens, None, shaft, None)?,
            ))
        }
        None => None,
    };
    unsafe {
        if let Some((halo_shaft, _)) = &shafts {
            context.DrawGeometry(halo_shaft, edge, stroke + 2.0 * halo, &pens.round);
        }
        for head in &heads {
            context.DrawGeometry(head, edge, 2.0 * halo, &pens.round);
        }
        if let Some((_, shaft)) = &shafts {
            context.DrawGeometry(shaft, fill, stroke, &pens.round);
        }
        for head in &heads {
            context.FillGeometry(head, fill, None);
        }
    }
    Ok(())
}

/// `curve` as a path to stroke, led into in a straight line from `from` if any, and out of it in
/// a straight line to `to` if any.
fn curve_geometry(
    pens: &Pens,
    from: Option<Vector2>,
    curve: Curve,
    to: Option<Vector2>,
) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geometry = pens.factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.BeginFigure(from.unwrap_or(curve.start), D2D1_FIGURE_BEGIN_HOLLOW);
        if from.is_some() {
            sink.AddLine(curve.start);
        }
        sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
            point1: curve.control,
            point2: curve.end,
        });
        if let Some(to) = to {
            sink.AddLine(to);
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        Ok(geometry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Fill, Head, Sizes, Span};

    fn v(x: f32, y: f32) -> Vector2 {
        Vector2 { X: x, Y: y }
    }

    /// A view of a 200 × 150 capture at 100 percent, with the image area's top-left at the
    /// capture's.
    fn actual_size() -> View {
        View::new(v(200.0, 150.0), v(200.0, 150.0))
    }

    /// Red at a stroke's size on first run.
    const RED: Style = Style {
        colour: Colour::Red,
        size: Sizes::FIRST_RUN.arrow,
    };

    /// A stroke `value` pixels wide at 100 percent.
    fn stroke_size(value: u32) -> Size {
        Size::new(value, Span::STROKES).unwrap()
    }

    /// Red text in `lines` lines from (100, 100) with an em of 20, laid out 2 ems wide and one
    /// and a half tall a line: 40 × 30 for one line.
    fn text(lines: u32) -> Annotation {
        Annotation::Text(
            Text {
                at: v(100.0, 100.0),
                text: vec!["Hi"; lines as usize].join("\n"),
                font: 20.0,
                per_em: v(2.0, 1.5 * lines as f32),
                lines,
            },
            Colour::Red,
        )
    }

    /// Where a text's top-left corner is and its em.
    fn placed(annotation: &Annotation) -> (Vector2, f32) {
        let Annotation::Text(text, _) = annotation else {
            panic!("{annotation:?} is not text");
        };
        (text.at, text.font)
    }

    #[test]
    fn a_size_leaves_text_as_it_was_and_a_colour_recolours_it() {
        let hi = text(1);
        for size in [stroke_size(1), stroke_size(3), stroke_size(12)] {
            assert_eq!(hi.restyled(Choice::Size(size)), hi, "{size:?}");
        }
        let Annotation::Text(laid, _) = &hi else {
            unreachable!()
        };
        assert_eq!(
            hi.restyled(Choice::Colour(Colour::Blue)),
            Annotation::Text(laid.clone(), Colour::Blue)
        );
    }

    /// The handle at the corner of a text's box `across` and `down`.
    fn corner(across: Side, down: Side) -> Handle {
        Handle::Bounds { across, down }
    }

    #[test]
    fn text_has_a_handle_at_each_corner_of_its_box_and_none_on_its_edges() {
        assert_eq!(
            text(1).handles(),
            [
                (corner(Side::Min, Side::Min), v(100.0, 100.0)),
                (corner(Side::Max, Side::Min), v(140.0, 100.0)),
                (corner(Side::Max, Side::Max), v(140.0, 130.0)),
                (corner(Side::Min, Side::Max), v(100.0, 130.0)),
            ]
        );
    }

    #[test]
    fn dragging_a_texts_corner_scales_its_font_with_the_opposite_corner_still() {
        let (min, max) = (Side::Min, Side::Max);
        for (handle, by, at, font) in [
            // Out along the diagonal to twice the size, the top-left where it was.
            (corner(max, max), v(40.0, 30.0), v(100.0, 100.0), 40.0),
            // Off the diagonal, as far as the pointer goes along it.
            (corner(max, max), v(35.0, -5.0), v(100.0, 100.0), 30.0),
            // In to half the size, the bottom-right where it was, at (140, 130).
            (corner(min, min), v(20.0, 15.0), v(120.0, 115.0), 10.0),
            // The bottom-left where it was, at (100, 130).
            (corner(max, min), v(-20.0, 15.0), v(100.0, 115.0), 10.0),
            (corner(min, max), v(-40.0, 30.0), v(60.0, 100.0), 40.0),
        ] {
            assert_eq!(
                placed(&text(1).adjusted(handle, by)),
                (at, font),
                "{handle:?}"
            );
        }
    }

    #[test]
    fn text_is_made_no_smaller_than_6_pixels_a_line() {
        // A line is one and a half ems, so the smallest em is 4, 6 pixels a line, with the
        // bottom-right at (140, 130) still.
        let past = v(60.0, 45.0);
        let smallest = text(1).adjusted(corner(Side::Min, Side::Min), past);
        assert_eq!(placed(&smallest), (v(132.0, 124.0), 4.0));
        // Two lines are 12 pixels tall, at the same smallest em.
        let smallest = text(2).adjusted(corner(Side::Max, Side::Max), Vector2::zero() - past);
        assert_eq!(placed(&smallest), (v(100.0, 100.0), 4.0));
        // From there it grows again.
        let grown = smallest.adjusted(corner(Side::Max, Side::Max), v(8.0, 12.0));
        assert_eq!(placed(&grown), (v(100.0, 100.0), 8.0));
    }

    #[test]
    fn a_texts_corner_dragged_where_its_font_cannot_change_leaves_it_exactly_as_it_was() {
        // Off whole pixels, where taking its extent from its far corner and back rounds.
        let Annotation::Text(mut moved, colour) = text(1) else {
            unreachable!()
        };
        moved.at = v(100.1, 100.7);
        moved.font = 0.7 * 20.0;
        let text = Annotation::Text(moved, colour);
        let smallest = text.adjusted(corner(Side::Min, Side::Min), v(60.0, 45.0));
        assert_eq!(
            smallest.adjusted(corner(Side::Min, Side::Min), v(9.0, 9.0)),
            smallest
        );
        // Across the diagonal, so not along it at all.
        assert_eq!(
            text.adjusted(corner(Side::Min, Side::Min), v(-3.0, 4.0)),
            text
        );
    }

    #[test]
    fn a_texts_corner_dropped_where_it_was_picked_up_leaves_it_exactly_as_it_was() {
        let shrunk = text(1).adjusted(corner(Side::Min, Side::Min), v(7.0, 3.0));
        assert_eq!(
            shrunk.dropped(corner(Side::Min, Side::Min), Vector2::zero(), 1.0),
            shrunk
        );
    }

    #[test]
    fn an_arrows_head_is_5_strokes_long_and_4_and_a_half_wide_at_every_size_and_scale() {
        let arrow = Arrow::straight(v(10.5, 50.5), v(110.5, 50.5));
        for (scale, heads) in [
            (1.0, [(10.0, 4.5), (15.0, 6.75), (25.0, 11.25)]),
            (2.0, [(20.0, 9.0), (30.0, 13.5), (50.0, 22.5)]),
        ] {
            for (size, (length, half_width)) in [stroke_size(2), stroke_size(3), stroke_size(5)]
                .into_iter()
                .zip(heads)
            {
                let outline = arrow.outline(size.pixels(scale)).unwrap();
                let base = 110.5 - length;
                assert_eq!(
                    outline.heads[0].corners,
                    [v(base, 50.5 + half_width), v(base, 50.5 - half_width)],
                    "{size:?} at {scale}"
                );
            }
        }
    }

    #[test]
    fn a_point_in_a_rectangle_or_on_its_outline_hits_it() {
        let rectangle = Annotation::Rectangle(
            Bounds {
                min: v(20.5, 20.5),
                max: v(120.5, 100.5),
            },
            RED,
            Fill::Outline,
        );
        let view = actual_size();
        // Inside, on each edge, and on the outer edge of the drawn stroke and halo, 2.5 out.
        for at in [
            v(70.0, 60.0),
            v(20.5, 60.0),
            v(120.5, 20.5),
            v(18.0, 60.0),
            v(70.0, 103.0),
        ] {
            assert_eq!(
                hit(std::slice::from_ref(&rectangle), at, &view, 1.0),
                Some(0),
                "at {at:?}"
            );
        }
        for at in [v(17.0, 60.0), v(70.0, 104.0), v(150.0, 60.0), v(5.0, 5.0)] {
            assert_eq!(
                hit(std::slice::from_ref(&rectangle), at, &view, 1.0),
                None,
                "at {at:?}"
            );
        }
    }

    #[test]
    fn a_solid_rectangle_is_hit_on_every_pixel_it_covers_and_nowhere_beyond() {
        let rectangle = Annotation::Rectangle(
            Bounds {
                min: v(20.5, 20.5),
                max: v(120.5, 100.5),
            },
            RED,
            Fill::Solid,
        );
        let view = actual_size();
        // Its corners are on pixels 20 and 120 across, which it covers whole, as a blur does.
        for at in [v(70.0, 60.0), v(20.0, 60.0), v(120.9, 100.9)] {
            assert_eq!(
                hit(std::slice::from_ref(&rectangle), at, &view, 1.0),
                Some(0),
                "at {at:?}"
            );
        }
        // With no halo, nothing outside those pixels.
        for at in [
            v(19.9, 60.0),
            v(121.0, 60.0),
            v(70.0, 101.0),
            v(70.0, 103.0),
        ] {
            assert_eq!(
                hit(std::slice::from_ref(&rectangle), at, &view, 1.0),
                None,
                "at {at:?}"
            );
        }
    }

    #[test]
    fn an_arrow_is_hit_within_6_screen_pixels_of_its_line_at_any_zoom() {
        let arrow = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(150.5, 50.5)), RED);
        // At 100 percent, and at Fit 50 percent, where capture point p is shown at p / 2.
        for (view, zoom) in [
            (actual_size(), 1.0),
            (View::new(v(200.0, 150.0), v(100.0, 75.0)), 0.5),
        ] {
            let on = |x: f32, y: f32| v(x * zoom, y * zoom);
            for at in [
                on(80.0, 50.5),
                on(80.0, 50.5) + v(0.0, 6.0),
                on(80.0, 50.5) - v(0.0, 6.0),
                on(20.5, 50.5) - v(6.0, 0.0),
                on(150.5, 50.5) + v(4.0, 4.0),
            ] {
                assert_eq!(
                    hit(std::slice::from_ref(&arrow), at, &view, 1.0),
                    Some(0),
                    "at {at:?}, {zoom}"
                );
            }
            for at in [
                on(80.0, 50.5) + v(0.0, 7.0),
                on(80.0, 50.5) - v(0.0, 7.0),
                on(20.5, 50.5) - v(7.0, 0.0),
                on(150.5, 50.5) + v(5.0, 5.0),
            ] {
                assert_eq!(
                    hit(std::slice::from_ref(&arrow), at, &view, 1.0),
                    None,
                    "at {at:?}, {zoom}"
                );
            }
        }
    }

    #[test]
    fn a_wide_arrow_is_hit_anywhere_on_its_drawn_stroke() {
        // At 400 percent the stroke and halo reach 10 screen pixels from the line.
        let mut view = actual_size();
        while view.zoom_in(v(0.0, 0.0)) {}
        let arrow = Annotation::Arrow(Arrow::straight(v(5.5, 10.5), v(40.5, 10.5)), RED);
        let on_line = view.to_area(v(20.0, 10.5));
        assert_eq!(
            hit(
                std::slice::from_ref(&arrow),
                on_line + v(0.0, 10.0),
                &view,
                1.0
            ),
            Some(0)
        );
        assert_eq!(
            hit(
                std::slice::from_ref(&arrow),
                on_line + v(0.0, 11.0),
                &view,
                1.0
            ),
            None
        );
    }

    #[test]
    fn the_topmost_annotation_is_hit_with_blurs_beneath_arrows_and_rectangles() {
        let rectangle = |min_x: f32, max_x: f32| Bounds {
            min: v(min_x, 20.5),
            max: v(max_x, 100.5),
        };
        // A blur placed after a rectangle it overlaps, and a later rectangle over both.
        let annotations = [
            Annotation::Rectangle(rectangle(20.5, 80.5), RED, Fill::Outline),
            Annotation::Blur(rectangle(60.5, 180.5), Level::FIRST_RUN),
            Annotation::Rectangle(rectangle(70.5, 120.5), RED, Fill::Outline),
        ];
        let view = actual_size();
        for (at, hits) in [
            (v(40.0, 60.0), Some(0)),
            (v(65.0, 60.0), Some(0)),
            (v(75.0, 60.0), Some(2)),
            (v(100.0, 60.0), Some(2)),
            (v(150.0, 60.0), Some(1)),
            // The blur covers the whole pixel its corner is on.
            (v(180.9, 100.9), Some(1)),
            (v(181.1, 60.0), None),
            (v(10.0, 60.0), None),
        ] {
            assert_eq!(hit(&annotations, at, &view, 1.0), hits, "at {at:?}");
        }
    }

    #[test]
    fn a_rectangle_or_blur_has_a_handle_at_each_corner_and_edge_midpoint() {
        let bounds = Bounds {
            min: v(20.5, 20.5),
            max: v(120.5, 100.5),
        };
        let positions = [
            v(20.5, 20.5),
            v(70.5, 20.5),
            v(120.5, 20.5),
            v(120.5, 60.5),
            v(120.5, 100.5),
            v(70.5, 100.5),
            v(20.5, 100.5),
            v(20.5, 60.5),
        ];
        for annotation in [
            Annotation::Rectangle(bounds, RED, Fill::Outline),
            Annotation::Blur(bounds, Level::FIRST_RUN),
        ] {
            let handles: Vec<Vector2> = annotation.handles().iter().map(|&(_, at)| at).collect();
            assert_eq!(handles, positions);
        }
    }

    #[test]
    fn a_new_arrow_has_a_handle_at_each_end_and_one_halfway_along_it() {
        let arrow = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(150.5, 80.5)), RED);
        assert_eq!(
            arrow.handles(),
            [
                (Handle::Tail, v(20.5, 50.5)),
                (Handle::Head, v(150.5, 80.5)),
                (Handle::Bend, v(85.5, 65.5)),
            ]
        );
    }

    /// An arrow from (20.5, 50.5) to (180.5, 50.5) with its middle handle dragged 80 down, so
    /// its curve passes through (100.5, 130.5), pulled towards (100.5, 210.5).
    fn bent() -> Annotation {
        Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED)
            .adjusted(Handle::Bend, v(0.0, 80.0))
    }

    #[test]
    fn a_resize_shrinks_every_annotation_with_the_capture_and_its_reverse_brings_each_back() {
        let annotations = [
            bent(),
            Annotation::Rectangle(
                Bounds {
                    min: v(10.5, 20.5),
                    max: v(50.5, 60.5),
                },
                RED,
                Fill::Tint,
            ),
            text(2),
            stroke(
                &[v(10.0, 20.0), v(50.0, 5.0), v(30.0, 40.0)],
                stroke_size(5),
            ),
            badge(3),
            Annotation::Blur(
                Bounds {
                    min: v(10.5, 50.5),
                    max: v(109.5, 90.5),
                },
                Level::FIRST_RUN,
            ),
        ];
        let halved: Vec<Annotation> = annotations
            .iter()
            .map(|annotation| annotation.resized(0.5))
            .collect();

        // The bent arrow from (10.25, 25.25) to (90.25, 25.25), still curving through its middle,
        // at half its width of 3.
        let Annotation::Arrow(arrow, _) = &halved[0] else {
            panic!("not an arrow");
        };
        assert_eq!((arrow.tail, arrow.head), (v(10.25, 25.25), v(90.25, 25.25)));
        assert_eq!(arrow.middle(), v(50.25, 65.25));
        assert_eq!(halved[0].stroke(1.0), 1.5);
        // The rectangle from (5.25, 10.25) to (25.25, 30.25), as thin and still tinted.
        assert_eq!(
            halved[1],
            Annotation::Rectangle(
                Bounds {
                    min: v(5.25, 10.25),
                    max: v(25.25, 30.25),
                },
                Style {
                    size: stroke_size(3).resized(0.5),
                    ..RED
                },
                Fill::Tint,
            )
        );
        assert_eq!(halved[1].stroke(1.0), 1.5);
        // The text at (50, 50) at an em of 10, its two lines' box 20 × 30.
        assert_eq!(placed(&halved[2]), (v(50.0, 50.0), 10.0));
        let Annotation::Text(text, _) = &halved[2] else {
            panic!("not text");
        };
        assert_eq!(text.bounds().max, v(70.0, 80.0));
        // The stroke through half its points, 2.5 wide.
        let Annotation::Stroke(shrunk, _) = &halved[3] else {
            panic!("not a stroke");
        };
        assert_eq!(shrunk.points, [v(5.0, 10.0), v(25.0, 2.5), v(15.0, 20.0)]);
        assert_eq!(halved[3].stroke(1.0), 2.5);
        // The badge centred on (15, 10), 16 across where it was 32, still reading 3.
        let Annotation::Badge(shrunk, style) = &halved[4] else {
            panic!("not a badge");
        };
        assert_eq!((shrunk.centre, shrunk.count), (v(15.0, 10.0), 3));
        assert_eq!(shrunk.extent(style.size, 1.0), v(16.0, 16.0));
        // The blur over half its pixels, 50 across, in blocks of 4 where they were 8, so as many
        // of them: 13 across.
        let Annotation::Blur(bounds, level) = &halved[5] else {
            panic!("not a blur");
        };
        assert_eq!((bounds.min, bounds.max), (v(5.25, 25.25), v(54.75, 45.25)));
        assert_eq!(level.block(1.0), 4);
        assert_eq!(
            bounds.pixels(200, 150).map(|region| region.width()),
            Some(50)
        );

        // Twice as large again, each is exactly as it was; and from a quarter.
        for (annotation, halved) in annotations.iter().zip(&halved) {
            assert_eq!(halved.resized(2.0), *annotation);
            assert_eq!(annotation.resized(0.25).resized(4.0), *annotation);
        }
    }

    #[test]
    fn dragging_an_arrows_middle_bends_it_through_the_handle_with_its_tip_where_it_was() {
        let Annotation::Arrow(arrow, _) = bent() else {
            panic!("not an arrow");
        };
        assert_eq!(
            bent().handles(),
            [
                (Handle::Tail, v(20.5, 50.5)),
                (Handle::Head, v(180.5, 50.5)),
                (Handle::Bend, v(100.5, 130.5)),
            ]
        );
        // The curve heads along (80, -160) at its end, from its control point to the tip, so
        // the head's base is 15 back along that, and its corners 6.75 either side of it.
        let corners = arrow.outline(stroke_size(3).pixels(1.0)).unwrap().heads[0].corners;
        let (base, across) = (v(173.792, 63.916), v(6.037, 3.019));
        for (corner, expected) in corners.into_iter().zip([base + across, base - across]) {
            assert!(
                (corner - expected).length() < 0.01,
                "{corner:?}, not {expected:?}"
            );
        }
    }

    #[test]
    fn a_bent_arrow_with_both_heads_has_one_at_each_end_pointing_along_its_curve() {
        let Annotation::Arrow(arrow, _) = bent().with_head(Head::Both) else {
            panic!("not an arrow");
        };
        let outline = arrow.outline(stroke_size(3).pixels(1.0)).unwrap();
        // At the tip as with one head. At the tail the curve leaves along (80, 160), so that
        // head points back along (-80, -160), its base 15 in and its corners 6.75 either side:
        // the tip's mirrored, as the curve is.
        let heads = [
            (v(180.5, 50.5), v(173.792, 63.916), v(6.037, 3.019)),
            (v(20.5, 50.5), v(27.208, 63.916), v(6.037, -3.019)),
        ];
        assert_eq!(outline.heads.len(), 2);
        for (head, (tip, base, across)) in outline.heads.iter().zip(heads) {
            assert_eq!(head.tip, tip);
            for (corner, expected) in head.corners.into_iter().zip([base + across, base - across]) {
                assert!(
                    (corner - expected).length() < 0.01,
                    "{corner:?}, not {expected:?}"
                );
            }
        }
        // The shaft runs halfway into each head.
        let shaft = outline.shaft.unwrap();
        assert!(((shaft.start - v(20.5, 50.5)).length() - 7.5).abs() < 0.01);
        assert!(((shaft.end - v(180.5, 50.5)).length() - 7.5).abs() < 0.01);
    }

    #[test]
    fn an_arrow_with_no_head_is_its_whole_curve_and_keeps_none_when_bent_or_straightened() {
        let line = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED)
            .with_head(Head::None);
        let Annotation::Arrow(bent_line, _) = line.adjusted(Handle::Bend, v(0.0, 80.0)) else {
            panic!("not an arrow");
        };
        let outline = bent_line.outline(stroke_size(3).pixels(1.0)).unwrap();
        assert!(outline.heads.is_empty());
        let shaft = outline.shaft.unwrap();
        assert_eq!((shaft.start, shaft.end), (v(20.5, 50.5), v(180.5, 50.5)));
        // Dropped back on the line, it is straight and still has none.
        assert_eq!(line.dropped(Handle::Bend, v(0.0, 2.0), 1.0), line);
    }

    #[test]
    fn moving_an_end_keeps_the_middle_as_far_along_and_across_in_proportion() {
        // Bent through a quarter of the way along and 40 to the right, a quarter of its length.
        let arrow = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED)
            .adjusted(Handle::Bend, v(-40.0, 40.0));
        let middle = |annotation: Annotation| annotation.handles()[2].1;
        assert_eq!(middle(arrow.clone()), v(60.5, 90.5));
        // Stretched to twice the length.
        assert_eq!(
            middle(arrow.adjusted(Handle::Head, v(160.0, 0.0))),
            v(100.5, 130.5)
        );
        // Turned to point down, so its right is to the left on screen.
        assert_eq!(
            middle(arrow.adjusted(Handle::Head, v(-160.0, 160.0))),
            v(-19.5, 90.5)
        );
        // From the tail's end, the head where it was.
        assert_eq!(
            middle(arrow.adjusted(Handle::Tail, v(-160.0, 0.0))),
            v(-59.5, 130.5)
        );
        // Moved whole, bend and all.
        assert_eq!(middle(arrow.moved(v(50.0, -10.0))), v(110.5, 80.5));
    }

    #[test]
    fn a_middle_dropped_within_4_screen_pixels_of_the_line_straightens_the_arrow_at_any_zoom() {
        let straight = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED);
        // How far off the line the middle is dropped, in capture pixels, at each zoom, and
        // whether that straightens it.
        for (zoom, off, straightens) in [
            (1.0, 3.5, true),
            (1.0, -3.5, true),
            (1.0, 4.5, false),
            (2.0, 1.75, true),
            (2.0, 2.25, false),
            (0.5, 7.0, true),
            (0.5, -9.0, false),
            (4.0, 0.875, true),
            (4.0, 1.125, false),
        ] {
            // Along the line too, which a straightened arrow forgets.
            let by = v(30.0, off);
            let bent = straight.adjusted(Handle::Bend, by);
            let expected = if straightens { &straight } else { &bent };
            assert_eq!(
                &straight.dropped(Handle::Bend, by, zoom),
                expected,
                "{off} off at {zoom}"
            );
        }
    }

    #[test]
    fn a_middle_dropped_on_the_line_beyond_an_end_straightens_the_arrow() {
        let straight = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED);
        for by in [v(150.0, 2.0), v(-200.0, -3.0)] {
            assert_eq!(
                straight.dropped(Handle::Bend, by, 1.0),
                straight,
                "by {by:?}"
            );
        }
    }

    #[test]
    fn a_click_on_the_middle_of_a_slightly_bent_arrow_leaves_it_bent() {
        // Bent 1.5 capture pixels at 400 percent, then seen at 100 percent, where that is under 4
        // screen pixels.
        let bent = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED).dropped(
            Handle::Bend,
            v(0.0, 1.5),
            4.0,
        );
        assert_eq!(bent.handles()[2].1, v(100.5, 52.0));
        assert_eq!(bent.dropped(Handle::Bend, Vector2::zero(), 1.0), bent);
    }

    #[test]
    fn dropping_an_end_or_the_body_leaves_a_slight_bend() {
        let bent = Annotation::Arrow(Arrow::straight(v(20.5, 50.5), v(180.5, 50.5)), RED).dropped(
            Handle::Bend,
            v(0.0, 1.5),
            4.0,
        );
        let stretched = bent.dropped(Handle::Head, v(10.0, 0.0), 1.0);
        assert_eq!(stretched, bent.adjusted(Handle::Head, v(10.0, 0.0)));
    }

    #[test]
    fn a_bent_arrow_is_hit_along_its_curve_not_its_chord_at_any_zoom() {
        let arrow = bent();
        let mut zoomed = actual_size();
        zoomed.zoom_in(v(0.0, 0.0));
        zoomed.zoom_in(v(0.0, 0.0));
        // At Fit 50 percent, 100 percent and 200 percent, where the stroke and halo reach 5 screen
        // pixels from the curve, inside the 6 it is hit within.
        for view in [
            View::new(v(200.0, 150.0), v(100.0, 75.0)),
            actual_size(),
            zoomed,
        ] {
            let zoom = view.label();
            // A quarter of the way along, the curve is at (60.5, 110.5) heading down and right.
            let quarter = view.to_area(v(60.5, 110.5));
            let off = v(1.0, -1.0) / 2.0f32.sqrt();
            for at in [
                view.to_area(v(100.5, 130.5)),
                quarter,
                quarter + off * 5.5,
                quarter - off * 5.5,
            ] {
                assert_eq!(
                    hit(std::slice::from_ref(&arrow), at, &view, 1.0),
                    Some(0),
                    "{at:?}, {zoom}"
                );
            }
            for at in [
                view.to_area(v(100.5, 50.5)),
                view.to_area(v(60.5, 50.5)),
                quarter + off * 7.0,
                quarter - off * 7.0,
            ] {
                assert_eq!(
                    hit(std::slice::from_ref(&arrow), at, &view, 1.0),
                    None,
                    "{at:?}, {zoom}"
                );
            }
        }
    }

    #[test]
    fn a_handle_is_picked_up_within_the_same_screen_distance_at_any_zoom() {
        let rectangle = Annotation::Rectangle(
            Bounds {
                min: v(20.5, 20.5),
                max: v(80.5, 60.5),
            },
            RED,
            Fill::Outline,
        );
        let top_right = Handle::Bounds {
            across: Side::Max,
            down: Side::Min,
        };
        let mut view = actual_size();
        loop {
            let at = view.to_area(v(80.5, 20.5));
            for offset in [v(0.0, 0.0), v(6.0, -6.0), v(-6.0, 6.0)] {
                assert_eq!(
                    rectangle.handle_at(at + offset, &view),
                    Some(top_right),
                    "{offset:?} at {}",
                    view.label()
                );
            }
            for offset in [v(7.0, 0.0), v(0.0, -7.0)] {
                assert_eq!(rectangle.handle_at(at + offset, &view), None, "{offset:?}");
            }
            if !view.zoom_in(v(0.0, 0.0)) {
                break;
            }
        }
    }

    #[test]
    fn a_rectangle_spans_a_drag_in_any_direction_from_top_left_to_bottom_right() {
        let bounds = Some(Bounds {
            min: v(20.5, 20.5),
            max: v(120.5, 100.5),
        });
        for (from, to) in [
            (v(20.5, 20.5), v(120.5, 100.5)),
            (v(120.5, 100.5), v(20.5, 20.5)),
            (v(120.5, 20.5), v(20.5, 100.5)),
            (v(20.5, 100.5), v(120.5, 20.5)),
        ] {
            assert_eq!(
                Bounds::from_drag(from, to),
                bounds,
                "from {from:?} to {to:?}"
            );
        }
    }

    #[test]
    fn a_drag_of_4_by_4_pixels_places_a_rectangle() {
        assert_eq!(
            Bounds::from_drag(v(54.0, 46.0), v(50.0, 50.0)),
            Some(Bounds {
                min: v(50.0, 46.0),
                max: v(54.0, 50.0),
            })
        );
    }

    #[test]
    fn a_drag_under_4_pixels_either_way_places_no_rectangle() {
        let from = v(50.0, 50.0);
        for to in [
            v(53.0, 53.0),
            v(47.0, 47.0),
            v(150.0, 53.9),
            v(46.1, 10.0),
            v(50.0, 50.0),
        ] {
            assert_eq!(Bounds::from_drag(from, to), None, "to {to:?}");
        }
    }

    #[test]
    fn a_crop_keeps_what_draws_inside_it_moved_to_its_origin_and_removes_the_rest() {
        let bounds = |min_x: f32, min_y: f32, max_x: f32, max_y: f32| Bounds {
            min: v(min_x, min_y),
            max: v(max_x, max_y),
        };
        let arrow =
            |tail: Vector2, head: Vector2| Annotation::Arrow(Arrow::straight(tail, head), RED);
        // Of a 200 × 150 capture, cropped to (50,40)-(150,120).
        let region = Region {
            left: 50,
            top: 40,
            right: 150,
            bottom: 120,
        };
        let annotations = [
            // Inside.
            arrow(v(60.5, 50.5), v(100.5, 50.5)),
            // Across the left edge.
            arrow(v(20.5, 60.5), v(100.5, 60.5)),
            // Wholly outside, above.
            arrow(v(60.5, 10.5), v(100.5, 10.5)),
            // Its shaft is above, but its head, 6.75 wide each side with a 1 pixel halo, reaches
            // down to 41.25.
            arrow(v(70.5, 33.5), v(100.5, 33.5)),
            // Its shaft's stroke and halo reach 2.5 down to 39, short of the crop, and its head is
            // far to the left.
            arrow(v(160.5, 36.5), v(-20.5, 36.5)),
            // Its stroke and halo reach 2.5 beyond its edge: 47.5 stops at the crop's edge, 48.5
            // crosses it.
            Annotation::Rectangle(bounds(10.5, 50.5, 47.5, 100.5), RED, Fill::Outline),
            Annotation::Rectangle(bounds(10.5, 50.5, 48.5, 100.5), RED, Fill::Outline),
            // Around the whole crop, so none of its outline is inside.
            Annotation::Rectangle(bounds(10.5, 10.5, 190.5, 140.5), RED, Fill::Outline),
            // Covers the pixel its corner is on: 49.5 covers 49, outside; 50.0 covers 50.
            Annotation::Blur(bounds(10.5, 50.5, 49.5, 100.5), Level::FIRST_RUN),
            Annotation::Blur(bounds(10.5, 50.5, 50.0, 100.5), Level::FIRST_RUN),
            // Below.
            Annotation::Blur(bounds(60.5, 120.0, 100.5, 140.5), Level::FIRST_RUN),
        ];

        assert_eq!(
            crop(&annotations, region, 1.0),
            [
                arrow(v(10.5, 10.5), v(50.5, 10.5)),
                arrow(v(-29.5, 20.5), v(50.5, 20.5)),
                arrow(v(20.5, -6.5), v(50.5, -6.5)),
                Annotation::Rectangle(bounds(-39.5, 10.5, -1.5, 60.5), RED, Fill::Outline),
                Annotation::Blur(bounds(-39.5, 10.5, 0.0, 60.5), Level::FIRST_RUN),
            ]
        );
    }

    #[test]
    fn a_crop_keeps_an_arrow_whose_curve_dips_into_it_bent_as_it_was() {
        let region = Region {
            left: 50,
            top: 40,
            right: 150,
            bottom: 120,
        };
        // Its ends and the line between them are 20 above the crop, but its curve dips 20 in.
        let straight = Annotation::Arrow(Arrow::straight(v(60.5, 20.5), v(140.5, 20.5)), RED);
        let bent = straight.adjusted(Handle::Bend, v(0.0, 40.0));
        assert_eq!(crop(std::slice::from_ref(&straight), region, 1.0), []);
        let cropped = crop(std::slice::from_ref(&bent), region, 1.0);
        assert_eq!(
            cropped[0].handles(),
            [
                (Handle::Tail, v(10.5, -19.5)),
                (Handle::Head, v(90.5, -19.5)),
                (Handle::Bend, v(50.5, 20.5)),
            ]
        );
    }

    #[test]
    fn a_crop_keeps_a_solid_rectangle_inside_it_and_none_beyond_its_edge() {
        let bounds = |min_x: f32, min_y: f32, max_x: f32, max_y: f32| Bounds {
            min: v(min_x, min_y),
            max: v(max_x, max_y),
        };
        let region = Region {
            left: 50,
            top: 40,
            right: 150,
            bottom: 120,
        };
        let annotations = [
            // Around the whole crop, so all of the crop is its block.
            Annotation::Rectangle(bounds(10.5, 10.5, 190.5, 140.5), RED, Fill::Solid),
            // No outline or halo reaches past its edge: 49.5 stops short of the crop, 50.5 is in.
            Annotation::Rectangle(bounds(10.5, 50.5, 49.5, 100.5), RED, Fill::Solid),
            Annotation::Rectangle(bounds(10.5, 50.5, 50.5, 100.5), RED, Fill::Solid),
        ];
        assert_eq!(
            crop(&annotations, region, 1.0),
            [
                Annotation::Rectangle(bounds(-39.5, -29.5, 140.5, 100.5), RED, Fill::Solid),
                Annotation::Rectangle(bounds(-39.5, 10.5, 0.5, 60.5), RED, Fill::Solid),
            ]
        );
    }

    #[test]
    fn a_crop_keeps_a_tinted_rectangle_around_it_as_its_tint_is_inside_and_none_beyond_its_edge() {
        let bounds = |min_x: f32, min_y: f32, max_x: f32, max_y: f32| Bounds {
            min: v(min_x, min_y),
            max: v(max_x, max_y),
        };
        let region = Region {
            left: 50,
            top: 40,
            right: 150,
            bottom: 120,
        };
        let annotations = [
            // Around the whole crop: its outline is outside, but its tint covers the crop.
            Annotation::Rectangle(bounds(10.5, 10.5, 190.5, 140.5), RED, Fill::Tint),
            // Its stroke and halo reach 2.5 beyond its edge, as an outline's do: 47.5 stops at
            // the crop's edge, 48.5 crosses it.
            Annotation::Rectangle(bounds(10.5, 50.5, 47.5, 100.5), RED, Fill::Tint),
            Annotation::Rectangle(bounds(10.5, 50.5, 48.5, 100.5), RED, Fill::Tint),
        ];
        assert_eq!(
            crop(&annotations, region, 1.0),
            [
                Annotation::Rectangle(bounds(-39.5, -29.5, 140.5, 100.5), RED, Fill::Tint),
                Annotation::Rectangle(bounds(-39.5, 10.5, -1.5, 60.5), RED, Fill::Tint),
            ]
        );
    }

    #[test]
    fn a_crop_reaches_as_far_as_strokes_drawn_at_the_capture_scale() {
        // At 200 percent the stroke and halo reach 5 beyond the rectangle's edge.
        let rectangle = Annotation::Rectangle(
            Bounds {
                min: v(10.5, 50.5),
                max: v(46.0, 100.5),
            },
            RED,
            Fill::Outline,
        );
        let region = Region {
            left: 50,
            top: 40,
            right: 150,
            bottom: 120,
        };
        assert_eq!(crop(std::slice::from_ref(&rectangle), region, 1.0), []);
        assert_eq!(crop(std::slice::from_ref(&rectangle), region, 2.0).len(), 1);
    }

    #[test]
    fn a_drag_of_4_pixels_places_an_arrow_in_every_direction() {
        let tail = v(50.0, 50.0);
        for head in [v(54.0, 50.0), v(46.0, 50.0), v(50.0, 54.0), v(50.0, 46.0)] {
            assert_eq!(
                Arrow::from_drag(tail, head),
                Some(Arrow::straight(tail, head))
            );
        }
    }

    #[test]
    fn a_shorter_drag_places_nothing_in_any_direction() {
        let tail = v(50.0, 50.0);
        for head in [
            v(50.0, 50.0),
            v(52.0, 50.0),
            v(46.5, 50.0),
            v(50.0, 53.9),
            v(50.0, 47.0),
            // 3.96 pixels along the diagonals, though nearly 3 across and down.
            v(52.8, 52.8),
            v(47.2, 47.2),
            v(52.8, 47.2),
        ] {
            assert_eq!(Arrow::from_drag(tail, head), None, "to {head:?}");
        }
    }

    /// A zigzag 300 pixels across from (10, 100), in teeth 20 across and 20 up or down, so each
    /// corner turns 90 degrees, as a mouse reports it: in moves of 1 to 4 pixels, unevenly.
    fn zigzag() -> Vec<Vector2> {
        let mut raw = vec![v(10.0, 100.0)];
        let steps = [1.0, 4.0, 2.0, 3.0, 1.5];
        let mut step = 0;
        for tooth in 0..15 {
            let corner = v(
                10.0 + 20.0 * (tooth + 1) as f32,
                100.0 - 20.0 * (tooth % 2) as f32,
            );
            let corner = if tooth % 2 == 0 {
                corner - v(0.0, 20.0)
            } else {
                corner + v(0.0, 20.0)
            };
            loop {
                let last = *raw.last().unwrap();
                let left = corner - last;
                if left.length() <= steps[step % steps.len()] {
                    raw.push(corner);
                    break;
                }
                raw.push(last + left / left.length() * steps[step % steps.len()]);
                step += 1;
            }
        }
        raw
    }

    /// How far each corner of a path turns, in degrees.
    fn turns(points: &[Vector2]) -> Vec<f32> {
        points
            .windows(3)
            .map(|three| {
                let (a, b) = (three[1] - three[0], three[2] - three[1]);
                let cos = a.dot(&b) / (a.length() * b.length());
                cos.clamp(-1.0, 1.0).acos().to_degrees()
            })
            .collect()
    }

    #[test]
    fn a_jagged_stroke_is_smoothed_to_no_corner_over_30_degrees_from_its_start_to_its_end() {
        let raw = zigzag();
        assert!(turns(&raw).iter().any(|&turn| turn > 89.0));
        // Points 3 screen pixels apart at Fit 50 percent, 100 and 400 percent.
        for spacing in [6.0, 3.0, 0.75] {
            let stroke = Stroke::smoothed(&raw, spacing);
            let sharpest = turns(&stroke.points).into_iter().fold(0.0, f32::max);
            assert!(sharpest <= 30.0, "{sharpest} at {spacing}");
            assert_eq!(stroke.points[0], raw[0]);
            let end = *stroke.points.last().unwrap() - *raw.last().unwrap();
            assert!(end.length() <= 1.0, "ends {end:?} off at {spacing}");
        }
    }

    fn stroke(points: &[Vector2], size: Size) -> Annotation {
        Annotation::Stroke(
            Stroke {
                points: points.to_vec(),
            },
            Style {
                colour: Colour::Red,
                size,
            },
        )
    }

    #[test]
    fn a_selected_stroke_is_outlined_around_its_points_with_half_its_stroke_and_its_halo() {
        let points = [v(10.0, 20.0), v(50.0, 5.0), v(30.0, 40.0)];
        for (size, scale, reach) in [
            (stroke_size(3), 1.0, 2.5),
            (stroke_size(2), 1.0, 2.0),
            (stroke_size(5), 1.0, 3.5),
            (stroke_size(12), 1.0, 7.0),
            (stroke_size(3), 2.0, 5.0),
        ] {
            assert_eq!(
                stroke(&points, size).outlined(scale),
                Some(Bounds {
                    min: v(10.0 - reach, 5.0 - reach),
                    max: v(50.0 + reach, 40.0 + reach),
                }),
                "{size:?} at {scale}"
            );
        }
    }

    #[test]
    fn a_stroke_is_hit_within_6_screen_pixels_of_any_segment_or_its_drawn_width_at_every_zoom() {
        // Down, then right, then up and right.
        let points = [v(20.5, 20.5), v(20.5, 80.5), v(90.5, 80.5), v(130.5, 40.5)];
        let medium = stroke(&points, stroke_size(3));
        // Off each segment's middle, square to it, and beyond each end along it.
        let diagonal = v(1.0, 1.0) / 2f32.sqrt();
        let offs = [
            (v(20.5, 50.5), v(1.0, 0.0)),
            (v(20.5, 50.5), v(-1.0, 0.0)),
            (v(55.5, 80.5), v(0.0, 1.0)),
            (v(55.5, 80.5), v(0.0, -1.0)),
            (v(110.5, 60.5), diagonal),
            (v(110.5, 60.5), Vector2::zero() - diagonal),
            (v(20.5, 20.5), v(0.0, -1.0)),
            (v(130.5, 40.5), v(diagonal.X, -diagonal.Y)),
        ];
        let mut view = View::new(v(200.0, 150.0), v(100.0, 75.0));
        loop {
            // The stroke and halo reach 2.5 capture pixels from the line.
            let reach = 6f32.max(2.5 * view.scale());
            for (on, away) in offs {
                let at = view.to_area(on);
                assert_eq!(
                    hit(
                        std::slice::from_ref(&medium),
                        at + away * (reach - 0.1),
                        &view,
                        1.0
                    ),
                    Some(0),
                    "{on:?} at {}",
                    view.label()
                );
                assert_eq!(
                    hit(
                        std::slice::from_ref(&medium),
                        at + away * (reach + 0.1),
                        &view,
                        1.0
                    ),
                    None,
                    "{on:?} at {}",
                    view.label()
                );
            }
            if !view.zoom_in(v(0.0, 0.0)) {
                break;
            }
        }
    }

    #[test]
    fn a_stroke_is_placed_once_the_pointer_has_gone_4_pixels_from_where_it_started() {
        let start = v(50.0, 50.0);
        // A 2-pixel wiggle, however long its path, places nothing.
        let wiggle = [
            start,
            v(52.0, 50.0),
            v(50.0, 51.0),
            v(51.9, 50.0),
            v(50.5, 51.5),
        ];
        assert_eq!(Stroke::from_drag(&wiggle, 3.0), None);
        // Out 4 pixels and back places one.
        let out = [start, v(52.8, 52.9), v(51.0, 51.0)];
        assert_eq!(
            Stroke::from_drag(&out, 3.0),
            Some(Stroke::smoothed(&out, 3.0))
        );
    }

    fn badge(count: u32) -> Annotation {
        Annotation::Badge(
            Badge {
                centre: v(10.0 * count as f32, 20.0),
                count,
            },
            Style {
                size: Sizes::FIRST_RUN.counter,
                ..RED
            },
        )
    }

    #[test]
    fn the_next_badge_is_one_more_than_the_highest_on_the_capture() {
        let arrow = Annotation::Arrow(Arrow::straight(v(1.0, 1.0), v(40.0, 1.0)), RED);
        assert_eq!(next_count(&[]), 1);
        assert_eq!(next_count(std::slice::from_ref(&arrow)), 1);
        assert_eq!(next_count(&[badge(1), arrow.clone(), badge(2)]), 3);
        // One more than the highest, not than how many there are.
        assert_eq!(next_count(&[badge(3), badge(4)]), 5);
    }

    /// What the badges among `annotations` read, in the order they were placed, each with how
    /// far across it is.
    fn counts(annotations: &[Annotation]) -> Vec<(f32, u32)> {
        annotations
            .iter()
            .filter_map(|annotation| match annotation {
                Annotation::Badge(badge, _) => Some((badge.centre.X, badge.count)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn deleting_a_badge_renumbers_those_after_it_and_undoing_brings_its_count_back() {
        let arrow = Annotation::Arrow(Arrow::straight(v(1.0, 1.0), v(40.0, 1.0)), RED);
        // Badges 1, 2 and 3 at 10, 20 and 30 across, with an arrow placed between 1 and 2.
        let placed = vec![badge(1), arrow, badge(2), badge(3)];
        for (index, left) in [
            // The first, the middle and the last badge.
            (0, vec![(20.0, 1), (30.0, 2)]),
            (2, vec![(10.0, 1), (30.0, 2)]),
            (3, vec![(10.0, 1), (20.0, 2)]),
            // The arrow, which leaves the badges as they were.
            (1, vec![(10.0, 1), (20.0, 2), (30.0, 3)]),
        ] {
            let mut history = crate::history::History::new();
            let mut annotations = placed.clone();
            history.record(annotations.clone());
            remove(&mut annotations, index);
            assert_eq!(counts(&annotations), left, "deleting {index}");
            assert_eq!(annotations.len(), 3);
            let undone = history.undo(annotations).unwrap();
            assert_eq!(undone, placed, "undoing deleting {index}");
            assert_eq!(counts(&undone), [(10.0, 1), (20.0, 2), (30.0, 3)]);
        }
    }

    #[test]
    fn a_crop_that_leaves_out_a_badge_renumbers_those_left() {
        // Medium badges 32 across at 10, 20 and 30 across and 20 down: a crop from 27 across
        // leaves out the first, which reaches 27, and keeps the others.
        let region = Region {
            left: 27,
            top: 0,
            right: 100,
            bottom: 100,
        };
        let annotations = [badge(1), badge(2), badge(3)];
        assert_eq!(
            counts(&crop(&annotations, region, 1.0)),
            [(-7.0, 1), (3.0, 2)]
        );
    }

    #[test]
    fn a_badge_is_its_size_across_from_16_to_64_and_widens_by_a_digit_for_each_beyond_two() {
        let across = |count: u32, size: Size, scale: f32| {
            Badge {
                centre: v(100.0, 100.0),
                count,
            }
            .extent(size, scale)
        };
        let sizes = [16, 32, 64].map(|value| Size::new(value, Span::BADGES).unwrap());
        for (scale, diameters) in [(1.0, [16.0, 32.0, 64.0]), (2.0, [32.0, 64.0, 128.0])] {
            for (size, diameter) in sizes.into_iter().zip(diameters) {
                for count in [1, 9, 10, 99] {
                    assert_eq!(
                        across(count, size, scale),
                        v(diameter, diameter),
                        "{count} {size:?} at {scale}"
                    );
                }
            }
        }
        // At medium its digits have an em of 17.6, and each is 0.575 of it, 10.12, wide.
        for (count, width) in [(100, 42.12), (999, 42.12), (1000, 52.24)] {
            let extent = across(count, Sizes::FIRST_RUN.counter, 1.0);
            assert!((extent.X - width).abs() < 0.01, "{count}: {extent:?}");
            assert_eq!(extent.Y, 32.0);
        }
    }

    #[test]
    fn a_diagonal_drag_of_4_pixels_places_an_arrow() {
        // 4.1 pixels long, though under 3 across and down.
        let tail = v(50.0, 50.0);
        for head in [v(52.9, 52.9), v(47.1, 52.9), v(47.1, 47.1), v(52.9, 47.1)] {
            assert!(Arrow::from_drag(tail, head).is_some(), "to {head:?}");
        }
    }
}
