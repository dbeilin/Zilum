//! How Zilum draws a bar of buttons in the theme: the bar, the tints behind its buttons, their
//! icons and labels, the dividers between them and its text. It knows nothing of what any button
//! does, so every Zilum window with buttons draws them alike.

use windows::{
    Win32::Graphics::{
        Direct2D::{
            Common::{
                D2D_RECT_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_GRADIENT_STOP,
            },
            D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
            D2D1_COLOR_SPACE_SRGB, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
            D2D1_EXTEND_MODE_CLAMP, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_ROUNDED_RECT,
            ID2D1DeviceContext, ID2D1SolidColorBrush,
        },
        DirectWrite::{
            DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP,
            DWriteCreateFactory, IDWriteFactory, IDWriteFont, IDWriteFontCollection,
            IDWriteTextFormat, IDWriteTextLayout,
        },
    },
    core::{BOOL, HSTRING, Result, w},
};
use windows_numerics::Vector2;

use crate::{annotation::color, style::Fill, theme::Palette};

/// Sizes in logical pixels: labels, icons, and the corners of a button's tint.
const FONT_SIZE: f32 = 13.0;
/// A tooltip's text, a little smaller than a label, as Windows 11's tooltips are.
const TIP_SIZE: f32 = 12.0;
/// A group's heading in a window such as Settings, larger than a label and semibold.
const HEADING_SIZE: f32 = 14.0;
const ICON_SIZE: f32 = 16.0;
/// A small icon inside a control, such as a hotkey field's clear control.
const SMALL_ICON_SIZE: f32 = 12.0;
/// Counter's icon, drawn rather than a glyph: a disc the icon's size holding a bold digit, as a
/// badge does.
const COUNTER_DIAMETER: f32 = ICON_SIZE;
const COUNTER_DIGIT_SIZE: f32 = 11.0;
/// An arrow's icon, drawn rather than a glyph: how far its ends are from the icon's centre across
/// and down, the width of its line, and its heads' length and half their width at the base.
const ARROW_REACH: f32 = 6.0;
const ARROW_STROKE: f32 = 1.5;
const ARROW_HEAD_LENGTH: f32 = 6.0;
const ARROW_HEAD_HALF_WIDTH: f32 = 3.5;
/// A rectangle's icon, drawn rather than a glyph: its side, and the width of its outline.
const RECTANGLE_SIDE: f32 = 12.0;
const RECTANGLE_STROKE: f32 = 1.5;
pub const RADIUS: f32 = 4.0;
/// The accent-coloured mark under a chosen button's icon, and how far above the button's bottom
/// it sits.
const MARK_WIDTH: f32 = 14.0;
const MARK_HEIGHT: f32 = 3.0;
const MARK_INSET: f32 = 2.0;
/// The fonts icons come from, best first. Segoe Fluent Icons is Windows 11's; Segoe MDL2 Assets,
/// Windows 10's, has most of the same glyphs at the same code points.
const ICON_FONTS: [&str; 2] = ["Segoe Fluent Icons", "Segoe MDL2 Assets"];
/// A colour swatch's spot, and the ring around the chosen one: the gap between them, and its
/// width.
const SWATCH_SPOT: f32 = 14.0;
const RING_GAP: f32 = 2.0;
const RING_WIDTH: f32 = 1.5;
/// The outline around the control that has the keyboard: its gap from the control, and its
/// width.
const FOCUS_GAP: f32 = 1.0;
const FOCUS_WIDTH: f32 = 2.0;
/// The knob of a switch.
const KNOB: f32 = 12.0;
/// A slider's track, and the accent dot in its knob, larger while the pointer is on it.
const TRACK: f32 = 4.0;
const SLIDER_DOT: f32 = 10.0;
const SLIDER_DOT_HOVER: f32 = 12.0;
/// The small window drawn in a window background's tile, its title strip and its corners.
const PICTURE_WIDTH: f32 = 56.0;
const PICTURE_HEIGHT: f32 = 36.0;
const PICTURE_TITLE: f32 = 7.0;
const PICTURE_RADIUS: f32 = 3.0;
/// The shadow under it in a tile that shows one: how far down it is, and its layers of black,
/// each a pixel further out, and how opaque each is, which stack into a soft edge.
const PICTURE_SHADOW_OFFSET: f32 = 2.0;
const PICTURE_SHADOW_LAYERS: u32 = 6;
const PICTURE_SHADOW_LAYER_OPACITY: f32 = 0.07;
/// The wallpaper drawn in a tile that shows one, inset from the tile's edge, from its top-left
/// colour to its bottom-right one.
const PICTURE_WALLPAPER_INSET: f32 = 6.0;
const PICTURE_WALLPAPER_FROM: u32 = 0x9DCBF6;
const PICTURE_WALLPAPER_TO: u32 = 0x2F63CF;
/// The accent outline around the chosen tile.
const OUTLINE_WIDTH: f32 = 2.0;

/// Labels and icons at one DPI.
pub struct Text {
    writer: IDWriteFactory,
    label: IDWriteTextFormat,
    tip: IDWriteTextFormat,
    heading: IDWriteTextFormat,
    /// The digit in Counter's icon.
    digit: IDWriteTextFormat,
    /// Each icon font this system has, best first, with its format and its small format.
    icons: Vec<(IDWriteFont, IDWriteTextFormat, IDWriteTextFormat)>,
}

impl Text {
    /// Text at `scale`, physical pixels per logical pixel.
    pub fn new(scale: f32) -> Result<Text> {
        unsafe {
            let writer: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let weighted = |family: &HSTRING, size: f32, weight: DWRITE_FONT_WEIGHT| {
                writer.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size * scale,
                    w!("en-us"),
                )
            };
            let format =
                |family: &HSTRING, size: f32| weighted(family, size, DWRITE_FONT_WEIGHT_NORMAL);
            let mut fonts: Option<IDWriteFontCollection> = None;
            writer.GetSystemFontCollection(&mut fonts, false)?;
            let mut icons = Vec::new();
            for family in ICON_FONTS {
                let family = HSTRING::from(family);
                let (mut index, mut exists) = (0, BOOL::default());
                let Some(fonts) = &fonts else { break };
                fonts.FindFamilyName(&family, &mut index, &mut exists)?;
                if !exists.as_bool() {
                    continue;
                }
                let font = fonts.GetFontFamily(index)?.GetFirstMatchingFont(
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                )?;
                icons.push((
                    font,
                    format(&family, ICON_SIZE)?,
                    format(&family, SMALL_ICON_SIZE)?,
                ));
            }
            let digit = weighted(
                &HSTRING::from("Segoe UI"),
                COUNTER_DIGIT_SIZE,
                DWRITE_FONT_WEIGHT_BOLD,
            )?;
            digit.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(Text {
                label: format(&HSTRING::from("Segoe UI"), FONT_SIZE)?,
                tip: format(&HSTRING::from("Segoe UI"), TIP_SIZE)?,
                heading: weighted(
                    &HSTRING::from("Segoe UI"),
                    HEADING_SIZE,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                )?,
                digit,
                writer,
                icons,
            })
        }
    }

    /// `text` as a label.
    pub fn label(&self, text: &str) -> Result<IDWriteTextLayout> {
        self.layout(text, &self.label)
    }

    /// `text` as a tooltip shows it.
    pub fn tip(&self, text: &str) -> Result<IDWriteTextLayout> {
        self.layout(text, &self.tip)
    }

    /// `text` as a group's heading.
    pub fn heading(&self, text: &str) -> Result<IDWriteTextLayout> {
        self.layout(text, &self.heading)
    }

    /// `text` as a message under a field says it, as small as a tooltip's text, broken into lines
    /// no wider than `width`.
    pub fn message(&self, text: &str, width: f32) -> Result<IDWriteTextLayout> {
        let text: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.writer
                .CreateTextLayout(&text, &self.tip, width, f32::MAX)
        }
    }

    /// How wide `text` is as a label.
    pub fn width(&self, text: &str) -> Result<f32> {
        Ok(size(&self.label(text)?)?.X)
    }

    /// The icon `glyph` from the best icon font that has it, or `fallback` as a label where none
    /// does, so a button never shows an empty box.
    pub fn icon(&self, glyph: char, fallback: &str) -> Result<IDWriteTextLayout> {
        self.icon_in(glyph, fallback, false)
    }

    /// `icon` at the small size.
    pub fn small_icon(&self, glyph: char, fallback: &str) -> Result<IDWriteTextLayout> {
        self.icon_in(glyph, fallback, true)
    }

    /// The digit Counter's icon holds: "1", a badge's first count, so the bar never changes as
    /// badges are placed.
    pub fn counter_digit(&self) -> Result<IDWriteTextLayout> {
        let text: Vec<u16> = "1".encode_utf16().collect();
        unsafe { self.writer.CreateTextLayout(&text, &self.digit, 0.0, 0.0) }
    }

    fn icon_in(&self, glyph: char, fallback: &str, small: bool) -> Result<IDWriteTextLayout> {
        for (font, format, small_format) in &self.icons {
            if unsafe { font.HasCharacter(glyph as u32)? }.as_bool() {
                let format = if small { small_format } else { format };
                return self.layout(&glyph.to_string(), format);
            }
        }
        self.label(fallback)
    }

    fn layout(&self, text: &str, format: &IDWriteTextFormat) -> Result<IDWriteTextLayout> {
        let text: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.writer
                .CreateTextLayout(&text, format, f32::MAX, f32::MAX)
        }
    }
}

/// How large `layout` is.
pub fn size(layout: &IDWriteTextLayout) -> Result<Vector2> {
    let metrics = metrics(layout)?;
    Ok(Vector2 {
        X: metrics.width,
        Y: metrics.height,
    })
}

/// How many lines `layout` takes.
pub fn lines(layout: &IDWriteTextLayout) -> Result<u32> {
    Ok(metrics(layout)?.lineCount)
}

/// What DirectWrite measured of `layout`.
fn metrics(layout: &IDWriteTextLayout) -> Result<DWRITE_TEXT_METRICS> {
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics)? };
    Ok(metrics)
}

/// What fills behind a button.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    Hover,
    Pressed,
    /// The active tool, or another choice that is in effect.
    Chosen,
}

/// Which colour text is drawn in.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Text,
    /// Quieter text, such as a status.
    Secondary,
    /// A control that can do nothing now.
    Disabled,
    /// Text that says something failed.
    Failed,
    /// A label on a button filled with the accent colour.
    OnAccent,
    /// Close's icon while Close is red.
    OnClose,
}

/// The palette's colours for bars, as `context` draws them. Every method draws only between
/// `BeginDraw` and `EndDraw`.
pub struct Paint {
    context: ID2D1DeviceContext,
    bar: ID2D1SolidColorBrush,
    hover: ID2D1SolidColorBrush,
    pressed: ID2D1SolidColorBrush,
    chosen: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    secondary: ID2D1SolidColorBrush,
    disabled: ID2D1SolidColorBrush,
    failed: ID2D1SolidColorBrush,
    divider: ID2D1SolidColorBrush,
    accent: ID2D1SolidColorBrush,
    on_accent: ID2D1SolidColorBrush,
    close: ID2D1SolidColorBrush,
    close_pressed: ID2D1SolidColorBrush,
    on_close: ID2D1SolidColorBrush,
    swatch_edge: ID2D1SolidColorBrush,
    field: ID2D1SolidColorBrush,
    /// A drawn shadow, made translucent as it is drawn.
    black: ID2D1SolidColorBrush,
}

impl Paint {
    pub fn new(context: &ID2D1DeviceContext, palette: &Palette) -> Result<Paint> {
        let brush = |rgb| unsafe { context.CreateSolidColorBrush(&color(rgb), None) };
        Ok(Paint {
            context: context.clone(),
            bar: brush(palette.bar)?,
            hover: brush(palette.hover)?,
            pressed: brush(palette.pressed)?,
            chosen: brush(palette.chosen)?,
            text: brush(palette.text)?,
            secondary: brush(palette.secondary)?,
            disabled: brush(palette.disabled)?,
            failed: brush(palette.failed)?,
            divider: brush(palette.divider)?,
            accent: brush(palette.accent)?,
            on_accent: brush(palette.on_accent)?,
            close: brush(palette.close)?,
            close_pressed: brush(palette.close_pressed)?,
            on_close: brush(palette.on_close)?,
            swatch_edge: brush(palette.swatch_edge)?,
            field: brush(palette.field)?,
            black: brush(0x000000)?,
        })
    }

    /// The bar covering `area`, with a hairline on its last row between it and what is below.
    pub fn bar(&self, area: &D2D_RECT_F) {
        unsafe {
            self.context.FillRectangle(area, &self.bar);
            self.context.FillRectangle(
                &D2D_RECT_F {
                    top: area.bottom - 1.0,
                    ..*area
                },
                &self.divider,
            );
        }
    }

    /// A flyout's panel covering `area`, in the bar's colour inside a hairline, its corners
    /// rounded by `radius` physical pixels. It casts no shadow.
    pub fn panel(&self, area: &D2D_RECT_F, radius: f32) {
        unsafe {
            self.context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: *area,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.bar,
            );
            self.context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: inset(area, 0.5),
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.divider,
                1.0,
                None,
            );
        }
    }

    /// `tint` behind a button covering `area`, its corners rounded by `radius` physical pixels.
    pub fn tint(&self, area: &D2D_RECT_F, tint: Tint, radius: f32) {
        let brush = match tint {
            Tint::Hover => &self.hover,
            Tint::Pressed => &self.pressed,
            Tint::Chosen => &self.chosen,
        };
        unsafe {
            self.context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: *area,
                    radiusX: radius,
                    radiusY: radius,
                },
                brush,
            )
        };
    }

    /// A hairline between groups, covering `area`.
    pub fn divider(&self, area: &D2D_RECT_F) {
        unsafe { self.context.FillRectangle(area, &self.divider) };
    }

    /// The short accent-coloured mark under the icon of a chosen button covering `area`, at
    /// `scale`.
    pub fn mark(&self, area: &D2D_RECT_F, scale: f32) {
        let (width, height) = ((MARK_WIDTH * scale).round(), (MARK_HEIGHT * scale).round());
        let left = ((area.left + area.right - width) / 2.0).round();
        let bottom = area.bottom - (MARK_INSET * scale).round();
        unsafe {
            self.context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left,
                        top: bottom - height,
                        right: left + width,
                        bottom,
                    },
                    radiusX: height / 2.0,
                    radiusY: height / 2.0,
                },
                &self.accent,
            )
        };
    }

    /// Counter's icon centred in `area`, at `scale`: a disc in the icon colour with `digit` cut
    /// out of it, like the badge Counter places, so the digit is the bar's colour, or `tint`'s
    /// on a tinted button. Centred on the digit's ink rather than its line, which leaves room
    /// below the baseline that figures never use.
    pub fn counter(
        &self,
        area: &D2D_RECT_F,
        digit: &IDWriteTextLayout,
        tint: Option<Tint>,
        scale: f32,
    ) -> Result<()> {
        let hole = match tint {
            Some(Tint::Hover) => &self.hover,
            Some(Tint::Pressed) => &self.pressed,
            Some(Tint::Chosen) => &self.chosen,
            None => &self.bar,
        };
        let radius = COUNTER_DIAMETER * scale / 2.0;
        let middle = centre(area);
        // With its layout's box empty, the overhangs are the ink's edges from where it is drawn.
        let ink = unsafe { digit.GetOverhangMetrics()? };
        let at = middle
            - Vector2 {
                X: (ink.right - ink.left) / 2.0,
                Y: (ink.bottom - ink.top) / 2.0,
            };
        unsafe {
            self.context.FillEllipse(
                &D2D1_ELLIPSE {
                    point: middle,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.text,
            );
            self.context
                .DrawTextLayout(at, digit, hole, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// An arrow's icon centred in `area`, at `scale`, drawn rather than a glyph: a line across
    /// the icon from its bottom-left to its top-right, with a filled head at the top-right end if
    /// `at_tip` and at the bottom-left end if `at_tail`, like the arrow Zilum draws.
    pub fn arrow(&self, area: &D2D_RECT_F, at_tip: bool, at_tail: bool, scale: f32) -> Result<()> {
        let middle = centre(area);
        let reach = ARROW_REACH * scale;
        let tip = middle
            + Vector2 {
                X: reach,
                Y: -reach,
            };
        let tail = middle
            + Vector2 {
                X: -reach,
                Y: reach,
            };
        let way = (tip - tail) / (tip - tail).length();
        let across = Vector2 {
            X: -way.Y,
            Y: way.X,
        };
        let (length, half_width) = (ARROW_HEAD_LENGTH * scale, ARROW_HEAD_HALF_WIDTH * scale);
        unsafe {
            let factory = self.context.GetFactory()?;
            // From `point`, the head pointing along `along`.
            let head = |point: Vector2, along: Vector2| -> Result<()> {
                let base = point - along * length;
                let geometry = factory.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                sink.BeginFigure(point, D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(&[base + across * half_width, base - across * half_width]);
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                self.context.FillGeometry(&geometry, &self.text, None);
                Ok(())
            };
            // The line runs halfway into a head, so no seam shows where they meet.
            let into = way * (length / 2.0);
            let start = if at_tail { tail + into } else { tail };
            let end = if at_tip { tip - into } else { tip };
            self.context
                .DrawLine(start, end, &self.text, ARROW_STROKE * scale, None);
            if at_tip {
                head(tip, way)?;
            }
            if at_tail {
                head(tail, Vector2::zero() - way)?;
            }
        }
        Ok(())
    }

    /// A rectangle's icon centred in `area`, at `scale`, drawn rather than a glyph, as Zilum draws
    /// a rectangle with `fill`: an outline, an outline over a translucent inside, or a block.
    pub fn rectangle(&self, area: &D2D_RECT_F, fill: Fill, scale: f32) {
        let middle = centre(area);
        let half = RECTANGLE_SIDE * scale / 2.0;
        let square = D2D_RECT_F {
            left: (middle.X - half).round(),
            top: (middle.Y - half).round(),
            right: (middle.X + half).round(),
            bottom: (middle.Y + half).round(),
        };
        let stroke = RECTANGLE_STROKE * scale;
        // The outline inside the square, so a Solid one is no smaller.
        let line = inset(&square, stroke / 2.0);
        unsafe {
            match fill {
                Fill::Solid => self.context.FillRectangle(&square, &self.text),
                Fill::Tint => {
                    self.text.SetOpacity(Fill::TINT_OPACITY);
                    self.context.FillRectangle(&square, &self.text);
                    self.text.SetOpacity(1.0);
                    self.context.DrawRectangle(&line, &self.text, stroke, None);
                }
                Fill::Outline => self.context.DrawRectangle(&line, &self.text, stroke, None),
            }
        }
    }

    /// A colour swatch centred in `area`, at `scale`: a spot filled with `fill`, edged so a white
    /// one shows on the bar, and ringed in the accent colour when `chosen`.
    pub fn swatch(&self, area: &D2D_RECT_F, fill: &ID2D1SolidColorBrush, chosen: bool, scale: f32) {
        let radius = SWATCH_SPOT * scale / 2.0;
        let spot = D2D1_ELLIPSE {
            point: centre(area),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.context.FillEllipse(&spot, fill);
            self.context
                .DrawEllipse(&spot, &self.swatch_edge, 1.0, None);
            if chosen {
                let ring = radius + (RING_GAP + RING_WIDTH / 2.0) * scale;
                self.context.DrawEllipse(
                    &D2D1_ELLIPSE {
                        radiusX: ring,
                        radiusY: ring,
                        ..spot
                    },
                    &self.accent,
                    RING_WIDTH * scale,
                    None,
                );
            }
        }
    }

    /// A button covering `area` that does what its group is for, such as Apply: filled with the
    /// accent colour, its corners rounded by `radius` physical pixels, with `label` on it. With
    /// the pointer on it or pressing it, the fill lets a little of the bar through, as Windows'
    /// accent buttons do.
    pub fn accent_button(
        &self,
        area: &D2D_RECT_F,
        label: &IDWriteTextLayout,
        tint: Option<Tint>,
        radius: f32,
    ) -> Result<()> {
        let opacity = match tint {
            Some(Tint::Hover) => 0.9,
            Some(Tint::Pressed) => 0.8,
            Some(Tint::Chosen) | None => 1.0,
        };
        unsafe {
            self.accent.SetOpacity(opacity);
            self.context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: *area,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.accent,
            );
            self.accent.SetOpacity(1.0);
        }
        self.centred(label, area, Tone::OnAccent)
    }

    /// Close covering `area` with its `icon`, as a caption close button: red down to the bar's
    /// hairline while the pointer is on it, a darker red while it is pressed, the icon white on
    /// either.
    pub fn close_button(
        &self,
        area: &D2D_RECT_F,
        tint: Option<Tint>,
        icon: &IDWriteTextLayout,
    ) -> Result<()> {
        let fill = match tint {
            Some(Tint::Hover) => &self.close,
            Some(Tint::Pressed) => &self.close_pressed,
            Some(Tint::Chosen) | None => return self.centred(icon, area, Tone::Text),
        };
        let above = D2D_RECT_F {
            bottom: area.bottom - 1.0,
            ..*area
        };
        unsafe { self.context.FillRectangle(&above, fill) };
        self.centred(icon, area, Tone::OnClose)
    }

    /// A field or a button such as Browse covering `area`, as Windows draws a text box or a
    /// standard button: filled, inside a hairline, its corners rounded by `radius` physical
    /// pixels.
    pub fn field(&self, area: &D2D_RECT_F, radius: f32) {
        let edge = inset(area, 0.5);
        unsafe {
            self.context.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: *area,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.field,
            );
            self.context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: edge,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.divider,
                1.0,
                None,
            );
        }
    }

    /// The accent outline just outside the control covering `area` that has the keyboard, at
    /// `scale`, following its corners rounded by `radius` physical pixels.
    pub fn focus(&self, area: &D2D_RECT_F, radius: f32, scale: f32) {
        let (gap, width) = ((FOCUS_GAP * scale).round(), (FOCUS_WIDTH * scale).round());
        let out = gap + width / 2.0;
        unsafe {
            self.context.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: inset(area, -out),
                    radiusX: radius + out,
                    radiusY: radius + out,
                },
                &self.accent,
                width,
                None,
            )
        };
    }

    /// A switch covering `area` at `scale`, as Windows 11 draws one: on, an accent-filled pill
    /// with its knob at the right; off, an outlined pill with its knob at the left, filled with
    /// `tint` while the pointer is on it or pressing it.
    pub fn toggle(&self, area: &D2D_RECT_F, on: bool, tint: Option<Tint>, scale: f32) {
        let half = (area.bottom - area.top) / 2.0;
        let pill = D2D1_ROUNDED_RECT {
            rect: *area,
            radiusX: half,
            radiusY: half,
        };
        let knob = KNOB * scale / 2.0;
        let centre_y = area.top + half;
        let (centre_x, fill) = if on {
            (area.right - half, &self.on_accent)
        } else {
            (area.left + half, &self.secondary)
        };
        unsafe {
            if on {
                self.context.FillRoundedRectangle(&pill, &self.accent);
            } else {
                if let Some(tint) = tint {
                    self.tint(area, tint, half);
                }
                let edge = D2D1_ROUNDED_RECT {
                    rect: inset(area, 0.5),
                    radiusX: half - 0.5,
                    radiusY: half - 0.5,
                };
                self.context
                    .DrawRoundedRectangle(&edge, &self.secondary, 1.0, None);
            }
            self.context.FillEllipse(
                &D2D1_ELLIPSE {
                    point: Vector2 {
                        X: centre_x,
                        Y: centre_y,
                    },
                    radiusX: knob,
                    radiusY: knob,
                },
                fill,
            );
        }
    }

    /// A slider covering `area` at `scale`, as Windows 11 draws one: a thin track between the
    /// knob's ends of travel, `knob` logical pixels across, the accent colour up to its centre at
    /// `at` and the secondary text's beyond, and the knob, an accent dot inside a ring of the
    /// field's colour, the dot a little larger while the pointer is on it.
    pub fn slider(&self, area: &D2D_RECT_F, knob: f32, at: f32, hover: bool, scale: f32) {
        let half = knob * scale / 2.0;
        let middle = (area.top + area.bottom) / 2.0;
        let track = |left: f32, right: f32| D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left,
                top: middle - TRACK / 2.0 * scale,
                right,
                bottom: middle + TRACK / 2.0 * scale,
            },
            radiusX: TRACK / 2.0 * scale,
            radiusY: TRACK / 2.0 * scale,
        };
        let centre = Vector2 { X: at, Y: middle };
        let circle = |radius: f32| D2D1_ELLIPSE {
            point: centre,
            radiusX: radius,
            radiusY: radius,
        };
        let dot = if hover { SLIDER_DOT_HOVER } else { SLIDER_DOT };
        unsafe {
            self.context
                .FillRoundedRectangle(&track(at, area.right - half), &self.secondary);
            self.context
                .FillRoundedRectangle(&track(area.left + half, at), &self.accent);
            self.context.FillEllipse(&circle(half), &self.field);
            self.context
                .DrawEllipse(&circle(half - 0.5), &self.swatch_edge, 1.0, None);
            self.context
                .FillEllipse(&circle(dot * scale / 2.0), &self.accent);
        }
    }

    /// A window background's tile covering `area`, at `scale`: a small window in the middle of a
    /// field, with a soft shadow under it when `shadow` and a wallpaper behind both when
    /// `wallpaper`, the field filled with `tint` while the pointer is on the tile or pressing it,
    /// and outlined in the accent colour when `chosen`.
    pub fn tile(
        &self,
        area: &D2D_RECT_F,
        shadow: bool,
        wallpaper: bool,
        chosen: bool,
        tint: Option<Tint>,
        scale: f32,
    ) -> Result<()> {
        let radius = RADIUS * scale;
        self.field(area, radius);
        if let Some(tint) = tint {
            self.tint(&inset(area, 1.0), tint, radius);
        }
        if wallpaper {
            let backdrop = inset(area, (PICTURE_WALLPAPER_INSET * scale).round());
            let stop = |position, rgb| D2D1_GRADIENT_STOP {
                position,
                color: color(rgb),
            };
            unsafe {
                let stops = self.context.CreateGradientStopCollection(
                    &[
                        stop(0.0, PICTURE_WALLPAPER_FROM),
                        stop(1.0, PICTURE_WALLPAPER_TO),
                    ],
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_COLOR_SPACE_SRGB,
                    D2D1_BUFFER_PRECISION_8BPC_UNORM,
                    D2D1_EXTEND_MODE_CLAMP,
                    D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT,
                )?;
                let brush = self.context.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 {
                            X: backdrop.left,
                            Y: backdrop.top,
                        },
                        endPoint: Vector2 {
                            X: backdrop.right,
                            Y: backdrop.bottom,
                        },
                    },
                    None,
                    &stops,
                )?;
                self.context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: backdrop,
                        radiusX: PICTURE_RADIUS * scale,
                        radiusY: PICTURE_RADIUS * scale,
                    },
                    &brush,
                );
            }
        }
        let middle = centre(area);
        let (x, y) = (middle.X.round(), middle.Y.round());
        let (half_width, half_height) = (
            (PICTURE_WIDTH * scale / 2.0).round(),
            (PICTURE_HEIGHT * scale / 2.0).round(),
        );
        let window = D2D_RECT_F {
            left: x - half_width,
            top: y - half_height,
            right: x + half_width,
            bottom: y + half_height,
        };
        let rounded = |rect: D2D_RECT_F, radius: f32| D2D1_ROUNDED_RECT {
            rect,
            radiusX: radius,
            radiusY: radius,
        };
        let corner = PICTURE_RADIUS * scale;
        unsafe {
            if shadow {
                let down = PICTURE_SHADOW_OFFSET * scale;
                let under = D2D_RECT_F {
                    top: window.top + down,
                    bottom: window.bottom + down,
                    ..window
                };
                self.black.SetOpacity(PICTURE_SHADOW_LAYER_OPACITY);
                for layer in 1..=PICTURE_SHADOW_LAYERS {
                    let out = layer as f32 * scale;
                    self.context.FillRoundedRectangle(
                        &rounded(inset(&under, -out), corner + out),
                        &self.black,
                    );
                }
                self.black.SetOpacity(1.0);
            }
            self.context
                .FillRoundedRectangle(&rounded(window, corner), &self.bar);
            // The title strip, rounded only at the top.
            let title = D2D_RECT_F {
                bottom: window.top + (PICTURE_TITLE * scale).round(),
                ..window
            };
            self.context
                .FillRoundedRectangle(&rounded(title, corner), &self.chosen);
            self.context.FillRectangle(
                &D2D_RECT_F {
                    top: title.top + corner,
                    ..title
                },
                &self.chosen,
            );
            self.context.DrawRoundedRectangle(
                &rounded(inset(&window, 0.5), corner),
                &self.swatch_edge,
                1.0,
                None,
            );
            if chosen {
                let width = (OUTLINE_WIDTH * scale).round();
                self.context.DrawRoundedRectangle(
                    &rounded(inset(area, width / 2.0), radius - width / 2.0),
                    &self.accent,
                    width,
                    None,
                );
            }
        }
        Ok(())
    }

    /// `layout`, a label or an icon, centred in `area` on whole pixels.
    pub fn centred(&self, layout: &IDWriteTextLayout, area: &D2D_RECT_F, tone: Tone) -> Result<()> {
        let size = size(layout)?;
        let at = Vector2 {
            X: (area.left + (area.right - area.left - size.X) / 2.0).round(),
            Y: (area.top + (area.bottom - area.top - size.Y) / 2.0).round(),
        };
        self.text(layout, at, tone);
        Ok(())
    }

    /// `layout` with its top-left corner at `at`.
    pub fn text(&self, layout: &IDWriteTextLayout, at: Vector2, tone: Tone) {
        let brush = match tone {
            Tone::Text => &self.text,
            Tone::Secondary => &self.secondary,
            Tone::Disabled => &self.disabled,
            Tone::Failed => &self.failed,
            Tone::OnAccent => &self.on_accent,
            Tone::OnClose => &self.on_close,
        };
        unsafe {
            self.context
                .DrawTextLayout(at, layout, brush, D2D1_DRAW_TEXT_OPTIONS_NONE)
        };
    }
}

/// `area` with each edge moved `by` physical pixels inwards, or outwards when `by` is negative.
pub fn inset(area: &D2D_RECT_F, by: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: area.left + by,
        top: area.top + by,
        right: area.right - by,
        bottom: area.bottom - by,
    }
}

/// The middle of `area`.
fn centre(area: &D2D_RECT_F) -> Vector2 {
    Vector2 {
        X: (area.left + area.right) / 2.0,
        Y: (area.top + area.bottom) / 2.0,
    }
}

/// `text` as it fits in `width`, as `measure` measures it: whole if it fits, or else its longest
/// beginning that fits with an ellipsis after it.
pub fn trim(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    // A space before the ellipsis would only push it away from the text.
    let trimmed = |chars: usize| {
        let end = text
            .char_indices()
            .nth(chars)
            .map_or(text.len(), |(at, _)| at);
        format!("{}…", text[..end].trim_end())
    };
    // The longest beginning that fits, found by halving: a longer beginning is never narrower.
    let (mut fits, mut too_long) = (0, text.chars().count());
    while too_long - fits > 1 {
        let middle = (fits + too_long) / 2;
        if measure(&trimmed(middle)) <= width {
            fits = middle;
        } else {
            too_long = middle;
        }
    }
    let shown = trimmed(fits);
    if measure(&shown) <= width {
        shown
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 10 pixels wide.
    fn ten_each(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn a_text_that_fits_is_untouched() {
        assert_eq!(trim("Saved as a.png", 140.0, ten_each), "Saved as a.png");
        assert_eq!(trim("Copied", 200.0, ten_each), "Copied");
    }

    #[test]
    fn a_text_too_wide_keeps_the_longest_beginning_that_fits_with_the_ellipsis() {
        assert_eq!(trim("Saved as long name.png", 90.0, ten_each), "Saved as…");
        assert_eq!(
            trim("Saved as long name.png", 139.0, ten_each),
            "Saved as lon…"
        );
    }

    #[test]
    fn a_space_before_the_ellipsis_is_left_out() {
        // "Saved as " and the ellipsis are exactly 100 pixels.
        assert_eq!(trim("Saved as long name.png", 100.0, ten_each), "Saved as…");
    }

    #[test]
    fn nothing_fitting_leaves_the_ellipsis_alone_or_nothing() {
        assert_eq!(trim("Saved as a.png", 15.0, ten_each), "…");
        assert_eq!(trim("Saved as a.png", 5.0, ten_each), "");
    }
}
