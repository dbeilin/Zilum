//! A hotkey as data, a set of modifiers and a key, from which its registration, its text in the
//! settings file and its text in a notification are all derived.

use std::fmt;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_F1, VK_SNAPSHOT,
};

/// A combination that starts one mode: any of Ctrl, Alt and Shift, and one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

/// A key a hotkey can end in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// `A` to `Z`.
    Letter(char),
    /// `0` to `9`.
    Digit(char),
    /// F1 to F24.
    F(u8),
    PrintScreen,
}

/// Text that is not a hotkey or `None`.
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid;

impl Hotkey {
    /// PrintScreen alone, the key Snipping Tool can take.
    pub const PRINTSCREEN: Hotkey = Hotkey::bare(Key::PrintScreen);

    /// `key` with no modifier.
    pub const fn bare(key: Key) -> Hotkey {
        Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            key,
        }
    }

    /// The modifiers to register it with, always with MOD_NOREPEAT, or holding the key would
    /// start a stream of captures.
    pub fn modifiers(self) -> HOT_KEY_MODIFIERS {
        let mut modifiers = MOD_NOREPEAT;
        for (held, modifier) in [
            (self.ctrl, MOD_CONTROL),
            (self.alt, MOD_ALT),
            (self.shift, MOD_SHIFT),
        ] {
            if held {
                modifiers |= modifier;
            }
        }
        modifiers
    }

    /// The virtual key to register it with.
    pub fn virtual_key(self) -> u32 {
        match self.key {
            // The virtual keys of letters and digits are their ASCII codes.
            Key::Letter(c) | Key::Digit(c) => c as u32,
            Key::F(n) => VK_F1.0 as u32 + n as u32 - 1,
            Key::PrintScreen => VK_SNAPSHOT.0 as u32,
        }
    }

    /// The hotkey `text` names, or `None` for the text `None`. Modifiers are read in any order
    /// and case, and so are keys.
    pub fn parse(text: &str) -> Result<Option<Hotkey>, Invalid> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("None") {
            return Ok(None);
        }
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = Key::parse(parts.pop().ok_or(Invalid)?)?;
        let mut hotkey = Hotkey::bare(key);
        for part in parts {
            let modifier = match part.to_ascii_lowercase().as_str() {
                "ctrl" => &mut hotkey.ctrl,
                "alt" => &mut hotkey.alt,
                "shift" => &mut hotkey.shift,
                _ => return Err(Invalid),
            };
            if *modifier {
                return Err(Invalid);
            }
            *modifier = true;
        }
        if hotkey.needs_modifier() {
            return Err(Invalid);
        }
        Ok(Some(hotkey))
    }

    /// True for a bare letter or digit, which would fire whenever the user types it. PrintScreen
    /// and the F keys may be bare.
    fn needs_modifier(self) -> bool {
        let bare = !(self.ctrl || self.alt || self.shift);
        bare && matches!(self.key, Key::Letter(_) | Key::Digit(_))
    }
}

/// Why a combination pressed in a mode's hotkey field is refused, and the field keeps what it
/// held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A bare letter or digit.
    NoModifier,
    /// The combination is in that mode's field.
    UsedFor(Mode),
    /// Another app owns the combination, or Snipping Tool takes PrintScreen.
    InUse,
}

/// Why `pressed` cannot go in `mode`'s field while the fields hold `fields`, one per mode in
/// `Mode::ALL`, or `None` when it can. `None`, a cleared field, is always allowed.
pub fn refusal(
    fields: [Option<Hotkey>; 4],
    mode: Mode,
    pressed: Option<Hotkey>,
) -> Option<Refusal> {
    let pressed = pressed?;
    if pressed.needs_modifier() {
        return Some(Refusal::NoModifier);
    }
    Mode::ALL
        .into_iter()
        .find(|other| *other != mode && fields[*other as usize] == Some(pressed))
        .map(Refusal::UsedFor)
}

/// `hotkey` as the settings file writes it, `None` for no hotkey.
pub fn text(hotkey: Option<Hotkey>) -> String {
    hotkey.map_or("None".into(), |hotkey| hotkey.to_string())
}

/// What a hotkey field holding `hotkey` shows: "Press a key…" while it is `listening` for the
/// next combination, or else the hotkey as the settings file writes it.
pub fn field_text(hotkey: Option<Hotkey>, listening: bool) -> String {
    if listening {
        "Press a key…".into()
    } else {
        text(hotkey)
    }
}

/// What Zilum says when Snipping Tool may own PrintScreen.
const SNIPPING_TOOL: &str = "PrintScreen is in use. If Windows Snipping Tool owns it, turn off 'Use the Print screen key to open screen capture' under Settings > Accessibility > Keyboard";

/// The notification for `hotkey` when another app owns it.
pub fn in_use(hotkey: Hotkey) -> String {
    if hotkey == Hotkey::PRINTSCREEN {
        format!("{SNIPPING_TOOL}, or change Zilum's hotkey in Settings.")
    } else {
        format!("{hotkey} is in use by another app. Close that app or change it in Settings.")
    }
}

/// As in the settings file: the modifiers in the order Ctrl, Alt, Shift, then the key, joined by
/// `+`.
impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (held, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::Letter(c) | Key::Digit(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::PrintScreen => write!(f, "PrintScreen"),
        }
    }
}

/// What a hotkey starts: the three capture modes and Copy text, in the order of their hotkeys'
/// fields; an earlier field keeps a combination a later one repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Capture,
    Screen,
    Window,
    CopyText,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Capture, Mode::Screen, Mode::Window, Mode::CopyText];

    /// The hotkeys each mode has when the settings file does not say otherwise.
    pub const DEFAULTS: [Option<Hotkey>; 4] = [
        Some(Hotkey::PRINTSCREEN),
        Some(Hotkey {
            shift: true,
            ..Hotkey::PRINTSCREEN
        }),
        // Clicking a window on the overlay captures it, so the window hotkey is for those who
        // want one.
        None,
        None,
    ];
}

/// `hotkeys`, one per mode in `Mode::ALL`, with each combination that repeats an earlier mode's
/// replaced by `None`, so Zilum never competes with itself, and whether any was.
pub fn without_repeats(mut hotkeys: [Option<Hotkey>; 4]) -> ([Option<Hotkey>; 4], bool) {
    let mut repeated = false;
    for later in 1..hotkeys.len() {
        if hotkeys[later].is_some() && hotkeys[..later].contains(&hotkeys[later]) {
            hotkeys[later] = None;
            repeated = true;
        }
    }
    (hotkeys, repeated)
}

impl Mode {
    /// As messages name it, "Screen capture" in "Already used for Screen capture". The Capture
    /// hotkey's mode is plain "Capture", not "Capture capture".
    pub fn name(self) -> &'static str {
        match self {
            Mode::Capture => "Capture",
            Mode::Screen => "Screen capture",
            Mode::Window => "Window capture",
            Mode::CopyText => "Copy text",
        }
    }
}

/// What a field says under it when `refusal` keeps it from taking `pressed`.
pub fn message(refusal: Refusal, pressed: Option<Hotkey>) -> String {
    match refusal {
        Refusal::NoModifier => "Add Ctrl, Alt or Shift".into(),
        Refusal::UsedFor(other) => format!("Already used for {}", other.name()),
        Refusal::InUse if pressed == Some(Hotkey::PRINTSCREEN) => in_use(Hotkey::PRINTSCREEN),
        Refusal::InUse => "In use by another app".into(),
    }
}

impl Key {
    /// The key whose virtual key is `vk`, if a hotkey can end in it.
    pub fn from_virtual_key(vk: u32) -> Option<Key> {
        let f1 = VK_F1.0 as u32;
        match vk {
            // The virtual keys of letters and digits are their ASCII codes.
            0x41..=0x5A => Some(Key::Letter(vk as u8 as char)),
            0x30..=0x39 => Some(Key::Digit(vk as u8 as char)),
            _ if (f1..f1 + 24).contains(&vk) => Some(Key::F((vk - f1 + 1) as u8)),
            _ if vk == VK_SNAPSHOT.0 as u32 => Some(Key::PrintScreen),
            _ => None,
        }
    }

    fn parse(text: &str) -> Result<Key, Invalid> {
        let upper = text.to_ascii_uppercase();
        let mut chars = upper.chars();
        match (chars.next(), chars.next()) {
            (Some(c @ 'A'..='Z'), None) => return Ok(Key::Letter(c)),
            (Some(c @ '0'..='9'), None) => return Ok(Key::Digit(c)),
            _ => {}
        }
        if upper == "PRINTSCREEN" {
            return Ok(Key::PrintScreen);
        }
        match upper.strip_prefix('F') {
            Some(number) if !number.starts_with('0') => match number.parse() {
                Ok(n @ 1..=24) => Ok(Key::F(n)),
                _ => Err(Invalid),
            },
            _ => Err(Invalid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkeys(texts: [&str; 4]) -> [Option<Hotkey>; 4] {
        texts.map(|text| Hotkey::parse(text).unwrap())
    }

    #[test]
    fn a_modifier_and_a_key_read_back_as_written() {
        let hotkey = Hotkey::parse("Alt+F3").unwrap().unwrap();
        assert_eq!(hotkey.to_string(), "Alt+F3");
    }

    #[test]
    fn every_modifier_combination_with_every_key_reads_back_as_written() {
        let mut keys: Vec<String> = ('A'..='Z').chain('0'..='9').map(String::from).collect();
        keys.extend((1..=24).map(|n| format!("F{n}")));
        keys.push("PrintScreen".into());
        let modifiers = [
            "Ctrl+",
            "Alt+",
            "Shift+",
            "Ctrl+Alt+",
            "Ctrl+Shift+",
            "Alt+Shift+",
            "Ctrl+Alt+Shift+",
        ];
        for key in &keys {
            for modifier in modifiers {
                let text = format!("{modifier}{key}");
                let hotkey = Hotkey::parse(&text).unwrap().unwrap();
                assert_eq!(hotkey.to_string(), text);
            }
        }
        for key in keys.iter().filter(|key| key.len() > 1) {
            assert_eq!(Hotkey::parse(key).unwrap().unwrap().to_string(), *key);
        }
    }

    #[test]
    fn a_field_shows_every_combination_as_the_file_writes_it_or_asks_for_a_key() {
        let keys = ('A'..='Z')
            .map(|c| (Key::Letter(c), c.to_string()))
            .chain(('0'..='9').map(|c| (Key::Digit(c), c.to_string())))
            .chain((1..=24).map(|n| (Key::F(n), format!("F{n}"))))
            .chain([(Key::PrintScreen, "PrintScreen".to_owned())]);
        for (key, name) in keys {
            for held in 0..8 {
                let (ctrl, alt, shift) = (held & 1 != 0, held & 2 != 0, held & 4 != 0);
                let hotkey = Hotkey {
                    ctrl,
                    alt,
                    shift,
                    key,
                };
                let mut shown = String::new();
                for (on, prefix) in [(ctrl, "Ctrl+"), (alt, "Alt+"), (shift, "Shift+")] {
                    if on {
                        shown.push_str(prefix);
                    }
                }
                shown.push_str(&name);
                assert_eq!(field_text(Some(hotkey), false), shown);
                assert_eq!(field_text(Some(hotkey), true), "Press a key…");
            }
        }
        assert_eq!(field_text(None, false), "None");
        assert_eq!(field_text(None, true), "Press a key…");
        assert_eq!(field_text(Some(Hotkey::PRINTSCREEN), false), "PrintScreen");
    }

    #[test]
    fn none_is_no_hotkey() {
        assert_eq!(Hotkey::parse("None"), Ok(None));
        assert_eq!(Hotkey::parse("none"), Ok(None));
        assert_eq!(text(None), "None");
    }

    #[test]
    fn modifiers_in_any_order_and_case_are_written_ctrl_alt_shift() {
        for text in [
            "shift+alt+ctrl+r",
            "SHIFT+CTRL+ALT+R",
            "Alt+Shift+Ctrl+R",
            "cTrL+sHiFt+aLt+r",
        ] {
            let hotkey = Hotkey::parse(text).unwrap().unwrap();
            assert_eq!(hotkey.to_string(), "Ctrl+Alt+Shift+R", "{text}");
        }
        assert_eq!(
            Hotkey::parse("shift+printscreen")
                .unwrap()
                .unwrap()
                .to_string(),
            "Shift+PrintScreen"
        );
    }

    #[test]
    fn a_combination_repeating_an_earlier_modes_leaves_the_later_mode_with_none() {
        assert_eq!(
            without_repeats(hotkeys([
                "PrintScreen",
                "PrintScreen",
                "Ctrl+Shift+PrintScreen",
                "None"
            ])),
            (
                hotkeys(["PrintScreen", "None", "Ctrl+Shift+PrintScreen", "None"]),
                true
            )
        );
        assert_eq!(
            without_repeats(hotkeys(["Alt+F3", "F12", "alt+f3", "F9"])),
            (hotkeys(["Alt+F3", "F12", "None", "F9"]), true)
        );
        assert_eq!(
            without_repeats(hotkeys(["F12", "F12", "F12", "F12"])),
            (hotkeys(["F12", "None", "None", "None"]), true)
        );
    }

    #[test]
    fn different_combinations_and_repeated_nones_are_kept() {
        let different = hotkeys([
            "PrintScreen",
            "Shift+PrintScreen",
            "Ctrl+Shift+PrintScreen",
            "Ctrl+Alt+T",
        ]);
        assert_eq!(without_repeats(different), (different, false));
        let nones = hotkeys(["None", "None", "Ctrl+R", "None"]);
        assert_eq!(without_repeats(nones), (nones, false));
    }

    fn pressed(text: &str) -> Option<Hotkey> {
        Hotkey::parse(text).unwrap()
    }

    #[test]
    fn a_bare_letter_or_digit_in_a_field_needs_a_modifier() {
        let fields = [None; 4];
        for key in [Key::Letter('R'), Key::Digit('5')] {
            assert_eq!(
                refusal(fields, Mode::Capture, Some(Hotkey::bare(key))),
                Some(Refusal::NoModifier),
                "{key:?}"
            );
        }
        for text in ["F12", "F1", "PrintScreen", "Ctrl+R", "Alt+5", "Shift+R"] {
            assert_eq!(refusal(fields, Mode::Screen, pressed(text)), None, "{text}");
        }
    }

    #[test]
    fn a_combination_in_another_field_is_refused_naming_that_mode() {
        let fields = hotkeys(["PrintScreen", "Shift+PrintScreen", "Alt+F3", "None"]);
        assert_eq!(
            refusal(fields, Mode::Window, pressed("Shift+PrintScreen")),
            Some(Refusal::UsedFor(Mode::Screen))
        );
        assert_eq!(
            refusal(fields, Mode::Screen, pressed("Alt+F3")),
            Some(Refusal::UsedFor(Mode::Window))
        );
        assert_eq!(
            refusal(fields, Mode::Window, pressed("PrintScreen")),
            Some(Refusal::UsedFor(Mode::Capture))
        );
    }

    /// What `mode`'s field says under it when `text` is pressed in it while the fields hold
    /// `fields`, or `None` when it takes the combination. A bare letter or digit can be pressed
    /// though the file cannot hold it.
    fn says(fields: [&str; 4], mode: Mode, text: &str) -> Option<String> {
        let pressed = Hotkey::parse(text)
            .unwrap_or_else(|Invalid| Some(Hotkey::bare(Key::parse(text).unwrap())));
        refusal(hotkeys(fields), mode, pressed).map(|refusal| message(refusal, pressed))
    }

    #[test]
    fn a_refused_combination_says_why_under_its_field() {
        let fields = ["Ctrl+Alt+R", "Shift+PrintScreen", "None", "None"];
        assert_eq!(
            says(fields, Mode::Capture, "R").as_deref(),
            Some("Add Ctrl, Alt or Shift")
        );
        assert_eq!(
            says(fields, Mode::Window, "5").as_deref(),
            Some("Add Ctrl, Alt or Shift")
        );
        assert_eq!(
            says(fields, Mode::Screen, "Ctrl+Alt+R").as_deref(),
            Some("Already used for Capture")
        );
        assert_eq!(
            says(fields, Mode::Window, "Shift+PrintScreen").as_deref(),
            Some("Already used for Screen capture")
        );
        assert_eq!(
            says(
                ["F12", "Shift+PrintScreen", "Alt+F3", "None"],
                Mode::Capture,
                "Alt+F3"
            )
            .as_deref(),
            Some("Already used for Window capture")
        );
        // A field set to None holds nothing another can repeat.
        assert_eq!(says(fields, Mode::Capture, "F12"), None);
        assert_eq!(says(fields, Mode::Window, "PrintScreen"), None);
        assert_eq!(says(fields, Mode::Window, "None"), None);
    }

    #[test]
    fn the_copy_text_field_refuses_and_is_refused_like_the_other_three() {
        let fields = ["PrintScreen", "Shift+PrintScreen", "F9", "Ctrl+Alt+T"];
        assert_eq!(
            says(fields, Mode::Capture, "Ctrl+Alt+T").as_deref(),
            Some("Already used for Copy text")
        );
        assert_eq!(
            says(fields, Mode::Window, "Ctrl+Alt+T").as_deref(),
            Some("Already used for Copy text")
        );
        assert_eq!(
            says(fields, Mode::CopyText, "F9").as_deref(),
            Some("Already used for Window capture")
        );
        assert_eq!(
            says(fields, Mode::CopyText, "PrintScreen").as_deref(),
            Some("Already used for Capture")
        );
        assert_eq!(
            says(fields, Mode::CopyText, "T").as_deref(),
            Some("Add Ctrl, Alt or Shift")
        );
        assert_eq!(says(fields, Mode::CopyText, "Ctrl+Alt+T"), None);
        assert_eq!(says(fields, Mode::CopyText, "F11"), None);
        assert_eq!(says(fields, Mode::CopyText, "None"), None);
    }

    #[test]
    fn a_combination_another_app_owns_says_so_and_printscreen_names_snipping_tool() {
        assert_eq!(
            message(Refusal::InUse, pressed("Alt+F3")),
            "In use by another app"
        );
        assert_eq!(
            message(Refusal::InUse, pressed("Shift+PrintScreen")),
            "In use by another app"
        );
        assert_eq!(
            message(Refusal::InUse, pressed("PrintScreen")),
            "PrintScreen is in use. If Windows Snipping Tool owns it, turn off 'Use the Print \
             screen key to open screen capture' under Settings > Accessibility > Keyboard, or \
             change Zilum's hotkey in Settings."
        );
    }

    #[test]
    fn a_fields_own_combination_and_none_are_never_repeats() {
        let fields = hotkeys(["None", "Shift+PrintScreen", "None", "None"]);
        assert_eq!(
            refusal(fields, Mode::Screen, pressed("Shift+PrintScreen")),
            None
        );
        assert_eq!(refusal(fields, Mode::Capture, None), None);
        assert_eq!(refusal(fields, Mode::Screen, None), None);
        assert_eq!(
            refusal(fields, Mode::Window, pressed("Ctrl+Shift+PrintScreen")),
            None
        );
    }

    #[test]
    fn unknown_names_bare_letters_and_bare_digits_are_invalid() {
        for text in [
            "R",
            "5",
            "Banana",
            "Ctrl+Banana",
            "Win+R",
            "Meta+F3",
            "Ctrl+F25",
            "Ctrl+F0",
            "Ctrl+F01",
            "Ctrl+Ctrl+R",
            "Ctrl+",
            "Ctrl+Shift",
            "",
            "Ctrl+RR",
        ] {
            assert_eq!(Hotkey::parse(text), Err(Invalid), "{text:?}");
        }
    }
}
