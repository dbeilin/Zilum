//! Light or dark, and the Windows accent colour: the palette Zilum's own windows draw from, its
//! colours named by the role they play.

use windows::{
    Win32::{
        Foundation::LPARAM,
        System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW},
        UI::WindowsAndMessaging::WM_SETTINGCHANGE,
    },
    core::{PCWSTR, w},
};

/// Sent to every top-level window when the accent colour changes. Only the `Graphics_Dwm`
/// feature's headers name it.
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;

/// The theme the settings file asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// Light or dark as Windows' app mode is.
    System,
    Light,
    Dark,
}

impl Theme {
    /// Every theme, in the order the settings file's values are listed.
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];
}

/// The Windows accent colour in the two shades Windows' own apps draw it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Accent {
    /// On light greys.
    on_light: u32,
    /// On dark greys.
    on_dark: u32,
}

impl Accent {
    /// The accent in the shades the `AccentPalette` registry value lists, as Windows writes it.
    fn from_palette(bytes: &[u8]) -> Option<Accent> {
        // Eight colours of four bytes, red, green, blue and a spare, from light 3 to dark 3.
        let shade = |index: usize| {
            let at = bytes.get(index * 4..index * 4 + 3)?;
            Some((at[0] as u32) << 16 | (at[1] as u32) << 8 | at[2] as u32)
        };
        Some(Accent {
            on_light: shade(4)?,
            on_dark: shade(1)?,
        })
    }

    /// Windows' default blue, for a system that says no other.
    const DEFAULT: Accent = Accent {
        on_light: 0x0067C0,
        on_dark: 0x4CC2FF,
    };
}

/// Every colour Zilum draws its own windows in, as sRGB, by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Whether these are the dark greys.
    pub dark: bool,
    /// The toolbar.
    pub bar: u32,
    /// Behind the image.
    pub backdrop: u32,
    /// Labels.
    pub text: u32,
    /// Quieter text: the status and the zoom level.
    pub secondary: u32,
    /// A control that can do nothing now, such as a hotkey field's clear control when the field
    /// holds None, as Windows draws disabled text.
    pub disabled: u32,
    /// Text that says something failed.
    pub failed: u32,
    /// Behind a button the pointer is on, is pressing, or that is the active tool or the chosen
    /// size.
    pub hover: u32,
    pub pressed: u32,
    pub chosen: u32,
    /// A hairline between parts of a window.
    pub divider: u32,
    /// Around each colour swatch, so the white one shows on the bar.
    pub swatch_edge: u32,
    /// The inside of a handle.
    pub handle: u32,
    /// The active tool's bar, the chosen swatch's ring, the chosen size dot, Apply's fill and
    /// the edge of handles.
    pub accent: u32,
    /// A label on a button filled with the accent colour, such as Apply.
    pub on_accent: u32,
    /// Behind Close while the pointer is on it and while it is pressed, and its icon then, as
    /// on a Windows caption close button, the same in both themes.
    pub close: u32,
    pub close_pressed: u32,
    pub on_close: u32,
    /// Behind a tooltip's text, and the hairline around it, which keeps it apart from whatever
    /// image is under it.
    pub tooltip: u32,
    pub tooltip_border: u32,
    /// Inside a field text is typed into, and behind a button such as Browse, as Windows fills
    /// its text boxes and buttons.
    pub field: u32,
    /// How opaque the black laid over the image outside a crop is.
    pub dim: f32,
}

/// Windows' caption close button red, and a darker red for while it is pressed.
const CLOSE: u32 = 0xC42B1C;
const CLOSE_PRESSED: u32 = 0xA3241A;

impl Palette {
    /// Windows' light or dark greys, with `accent` in the shade for them. The tints are Windows'
    /// translucent fills as they come out over the bar.
    fn new(dark: bool, accent: Accent) -> Palette {
        if dark {
            Palette {
                dark,
                bar: 0x202020,
                backdrop: 0x161616,
                text: 0xFFFFFF,
                secondary: 0xCFCFCF,
                disabled: 0x787878,
                failed: 0xFF99A4,
                hover: 0x2D2D2D,
                pressed: 0x292929,
                chosen: 0x333333,
                divider: 0x333333,
                swatch_edge: 0x636363,
                // A white handle would show its light accent edge too faintly.
                handle: 0x202020,
                accent: accent.on_dark,
                // Windows' light shades of the accent take black text, and its dark shades white.
                on_accent: 0x000000,
                close: CLOSE,
                close_pressed: CLOSE_PRESSED,
                on_close: 0xFFFFFF,
                tooltip: 0x2C2C2C,
                tooltip_border: 0x484848,
                field: 0x2D2D2D,
                dim: 0.55,
            }
        } else {
            Palette {
                dark,
                bar: 0xF3F3F3,
                backdrop: 0xE3E3E3,
                text: 0x1A1A1A,
                secondary: 0x606060,
                disabled: 0xA0A0A0,
                failed: 0xC42B1C,
                hover: 0xEAEAEA,
                pressed: 0xEDEDED,
                chosen: 0xE4E4E4,
                divider: 0xE0E0E0,
                swatch_edge: 0xC2C2C2,
                handle: 0xFFFFFF,
                accent: accent.on_light,
                on_accent: 0xFFFFFF,
                close: CLOSE,
                close_pressed: CLOSE_PRESSED,
                on_close: 0xFFFFFF,
                tooltip: 0xF9F9F9,
                tooltip_border: 0xD1D1D1,
                field: 0xFBFBFB,
                dim: 0.45,
            }
        }
    }
}

/// The palette for `theme` as Windows is set now: its app mode, for `System`, and its accent.
pub fn palette(theme: Theme) -> Palette {
    let dark = match theme {
        Theme::System => !apps_use_light_theme(),
        Theme::Light => false,
        Theme::Dark => true,
    };
    let accent = accent_palette()
        .and_then(|bytes| Accent::from_palette(&bytes))
        .unwrap_or(Accent::DEFAULT);
    Palette::new(dark, accent)
}

/// Whether the message `msg` with `lparam`, sent to a top-level window, says Windows' app mode
/// or accent colour has changed.
pub fn changed(msg: u32, lparam: LPARAM) -> bool {
    match msg {
        WM_DWMCOLORIZATIONCOLORCHANGED => true,
        WM_SETTINGCHANGE if lparam.0 != 0 => {
            let area = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
            area.is_ok_and(|area| area == "ImmersiveColorSet")
        }
        _ => false,
    }
}

/// Windows' "Choose your mode" for apps. Light when it says nothing.
fn apps_use_light_theme() -> bool {
    let mut value = 1u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_err() || value != 0
}

/// The shades of the accent colour, as Windows lists them for its own apps.
fn accent_palette() -> Option<Vec<u8>> {
    let mut bytes = vec![0u8; 32];
    let mut size = bytes.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(bytes.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| {
        bytes.truncate(size as usize);
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows' default blue, as `AccentPalette` holds it: light 3 to dark 3, then a spare, each
    /// as red, green, blue and an unused byte.
    const BLUE: [u8; 32] = [
        0x99, 0xEB, 0xFF, 0x00, 0x4C, 0xC2, 0xFF, 0x00, 0x00, 0x91, 0xF8, 0x00, 0x00, 0x78, 0xD4,
        0x00, 0x00, 0x67, 0xC0, 0x00, 0x00, 0x3E, 0x92, 0x00, 0x00, 0x1A, 0x68, 0x00, 0xF7, 0x63,
        0x0C, 0x00,
    ];

    /// The relative luminance WCAG defines.
    fn luminance(rgb: u32) -> f64 {
        let linear = |shift: u32| {
            let c = ((rgb >> shift) & 0xFF) as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn both() -> [Palette; 2] {
        let accent = Accent::from_palette(&BLUE).unwrap();
        [Palette::new(false, accent), Palette::new(true, accent)]
    }

    #[test]
    fn the_accent_is_its_darker_shade_on_light_greys_and_its_lighter_on_dark() {
        let [light, dark] = both();
        assert_eq!(light.accent, 0x0067C0);
        assert_eq!(dark.accent, 0x4CC2FF);
        assert!(!light.dark && dark.dark);
    }

    #[test]
    fn a_palette_value_too_short_to_hold_the_shades_gives_no_accent() {
        assert_eq!(Accent::from_palette(&BLUE[..18]), None);
        assert_eq!(Accent::from_palette(&[]), None);
    }

    #[test]
    fn light_greys_are_light_and_dark_greys_dark() {
        let [light, dark] = both();
        for surface in [
            light.bar,
            light.backdrop,
            light.hover,
            light.pressed,
            light.chosen,
        ] {
            assert!(luminance(surface) > 0.6, "{surface:06X}");
        }
        for surface in [
            dark.bar,
            dark.backdrop,
            dark.hover,
            dark.pressed,
            dark.chosen,
        ] {
            assert!(luminance(surface) < 0.05, "{surface:06X}");
        }
        assert!(light.dim < dark.dim + f32::EPSILON && light.dim > 0.0 && dark.dim < 1.0);
    }

    #[test]
    fn text_reads_on_every_surface_it_is_drawn_on() {
        for palette in both() {
            for surface in [
                palette.bar,
                palette.hover,
                palette.pressed,
                palette.chosen,
                palette.tooltip,
                palette.field,
            ] {
                for text in [palette.text, palette.secondary] {
                    assert!(
                        contrast(text, surface) >= 4.5,
                        "{text:06X} on {surface:06X} in {palette:?}"
                    );
                }
            }
            assert!(contrast(palette.failed, palette.bar) >= 4.5, "{palette:?}");
        }
    }

    #[test]
    fn a_disabled_control_shows_on_a_field_but_quieter_than_secondary_text() {
        for palette in both() {
            let disabled = contrast(palette.disabled, palette.field);
            assert!(disabled > 2.0, "{palette:?}");
            assert!(
                disabled < contrast(palette.secondary, palette.field) - 1.0,
                "{palette:?}"
            );
        }
    }

    #[test]
    fn a_tooltip_s_hairline_shows_against_the_tooltip_and_the_bar() {
        for palette in both() {
            for surface in [palette.tooltip, palette.bar] {
                assert!(
                    contrast(palette.tooltip_border, surface) > 1.2,
                    "{surface:06X} in {palette:?}"
                );
            }
        }
    }

    #[test]
    fn a_label_reads_on_a_button_filled_with_the_accent() {
        for palette in both() {
            assert!(
                contrast(palette.on_accent, palette.accent) >= 4.5,
                "{palette:?}"
            );
        }
    }

    #[test]
    fn close_is_the_same_red_in_both_themes_darker_while_pressed_and_its_icon_reads_on_it() {
        let [light, dark] = both();
        assert_eq!(
            (light.close, light.close_pressed, light.on_close),
            (dark.close, dark.close_pressed, dark.on_close)
        );
        for palette in both() {
            assert!(luminance(palette.close_pressed) < luminance(palette.close));
            for fill in [palette.close, palette.close_pressed] {
                assert!(contrast(palette.on_close, fill) >= 4.5, "{fill:06X}");
            }
        }
    }

    #[test]
    fn a_white_swatch_and_a_handle_stand_out_from_what_is_behind_them() {
        for palette in both() {
            assert!(contrast(palette.swatch_edge, 0xFFFFFF) > 1.5, "{palette:?}");
            assert!(
                contrast(palette.swatch_edge, palette.bar) > 1.2,
                "{palette:?}"
            );
            assert!(
                contrast(palette.accent, palette.handle) >= 3.0,
                "{palette:?}"
            );
        }
    }
}
