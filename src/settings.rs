//! The settings file, `%APPDATA%\Zilum\settings.ini`: plain text the user can read, edit and
//! delete, one `key=value` per line.

use std::{
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use windows::{
    Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_Pictures, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
        },
    },
    core::GUID,
};

use crate::{
    background::Background,
    hotkey::{self, Hotkey, Mode},
    style::{Colour, Fill, Head, Level, Size, Sizes, Span, TextSize},
    theme::Theme,
};

/// Everything the settings file says.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// One per mode, in the order of `Mode::ALL`; `None` is a mode with no hotkey.
    pub hotkeys: [Option<Hotkey>; 4],
    /// The colour of the next annotation.
    pub colour: Colour,
    /// The size each tool places its next annotation at.
    pub sizes: Sizes,
    /// The em the next text is placed at.
    pub text_size: TextSize,
    /// Which ends of the next arrow carry a head.
    pub head: Head,
    /// How the next rectangle is drawn.
    pub fill: Fill,
    /// The level of the next blur.
    pub blur: Level,
    /// The folder Save writes into, a full path. It may not exist yet.
    pub save_folder: PathBuf,
    /// Whether Zilum starts when the user signs in.
    pub start_with_windows: bool,
    /// Light, dark, or as Windows' app mode is.
    pub theme: Theme,
    /// What the next window capture puts around the window.
    pub window_background: Background,
}

impl Settings {
    /// The settings of a missing file, with the save folder at `save_folder`.
    pub fn defaults(save_folder: PathBuf) -> Settings {
        Settings {
            hotkeys: Mode::DEFAULTS,
            colour: Colour::FIRST_RUN,
            sizes: Sizes::FIRST_RUN,
            text_size: TextSize::FIRST_RUN,
            head: Head::FIRST_RUN,
            fill: Fill::FIRST_RUN,
            blur: Level::FIRST_RUN,
            save_folder,
            start_with_windows: false,
            theme: Theme::System,
            window_background: Background::TrimShadow,
        }
    }
}

/// What the file says: as read at startup, then as last stored. Shared, since saving reads the
/// save folder from its own thread.
static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

/// The settings the file says, and whether any value in it was invalid. With no file, or one
/// that cannot be read, they are the defaults. They are also `current` from now on.
pub fn load() -> (Settings, bool) {
    let read = file().and_then(|file| fs::read(file).ok());
    let default_folder = default_folder();
    let (settings, invalid) = match read {
        Some(bytes) => parse(&String::from_utf8_lossy(&bytes), &default_folder),
        None => (Settings::defaults(default_folder), false),
    };
    *CURRENT.lock().unwrap() = Some(settings.clone());
    (settings, invalid)
}

/// The settings as last loaded or stored, so a change to one of them can store the rest as
/// they were.
pub fn current() -> Settings {
    CURRENT
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| Settings::defaults(default_folder()))
}

/// Writes the settings file to say `settings`, creating it and its folder if they are missing.
/// The new text is on the disk before it replaces the old in one move, so a crash leaves one or
/// the other. A failure leaves the file as it was.
pub fn store(settings: Settings) {
    let text = to_text(&settings);
    *CURRENT.lock().unwrap() = Some(settings);
    let Some(file) = file() else {
        return;
    };
    let temporary = file.with_extension("ini.tmp");
    let written = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| {
            let mut out = File::create(&temporary)?;
            out.write_all(text.as_bytes())?;
            out.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, &file));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

/// `settings.ini` in `Zilum` in the user's roaming application data folder.
fn file() -> Option<PathBuf> {
    Some(
        known_folder(&FOLDERID_RoamingAppData)?
            .join("Zilum")
            .join("settings.ini"),
    )
}

/// The default save folder, `Zilum` in the user's Pictures folder wherever that has been moved
/// to. Empty, so every save fails, in the unlikely case Windows cannot say where that is.
fn default_folder() -> PathBuf {
    known_folder(&FOLDERID_Pictures)
        .map(|pictures| pictures.join("Zilum"))
        .unwrap_or_default()
}

/// Where the known folder `id` is for this user.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: The shell's string is copied, then freed once.
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let folder = path.to_hstring().to_os_string();
        CoTaskMemFree(Some(path.0 as _));
        Some(PathBuf::from(folder))
    }
}

/// The settings `text` says, and whether any value was invalid. A key that is missing or
/// invalid has its default, with the save folder's at `default_folder`, and a hotkey that
/// repeats an earlier mode's is `None`.
fn parse(text: &str, default_folder: &Path) -> (Settings, bool) {
    let mut settings = Settings::defaults(default_folder.to_owned());
    let mut invalid = false;
    // Notepad may start the file with a byte order mark.
    let lines: Vec<(&str, &str)> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect();
    let capture_key = file_key(Mode::Capture);
    let has_capture_key = lines.iter().any(|(key, _)| *key == capture_key);
    for (key, value) in lines {
        // Older files name the Capture hotkey's key after region capture. The new name wins.
        let key = match key {
            "hotkey_region" if has_capture_key => continue,
            "hotkey_region" => capture_key,
            _ => key,
        };
        if let Some(mode) = Mode::ALL.into_iter().find(|m| file_key(*m) == key) {
            let slot = &mut settings.hotkeys[mode as usize];
            match Hotkey::parse(value) {
                Ok(hotkey) => *slot = hotkey,
                Err(_) => {
                    *slot = Mode::DEFAULTS[mode as usize];
                    invalid = true;
                }
            }
            continue;
        }
        match key {
            "color" => match Colour::ALL.into_iter().find(|c| colour_name(*c) == value) {
                Some(colour) => settings.colour = colour,
                None => {
                    settings.colour = Colour::FIRST_RUN;
                    invalid = true;
                }
            },
            // Earlier builds' one size for every tool, thin, medium or thick, which meant
            // something different for each: every tool keeps its own, medium unless its key says
            // otherwise, and the line goes on the next write.
            "size" => {}
            "arrow_size" | "rect_size" | "pen_size" | "counter_size" => {
                let first = Sizes::FIRST_RUN;
                let (slot, first_run, span) = match key {
                    "arrow_size" => (&mut settings.sizes.arrow, first.arrow, Span::STROKES),
                    "rect_size" => (&mut settings.sizes.rect, first.rect, Span::STROKES),
                    "pen_size" => (&mut settings.sizes.pen, first.pen, Span::STROKES),
                    _ => (&mut settings.sizes.counter, first.counter, Span::BADGES),
                };
                match whole(value).and_then(|value| Size::new(value, span)) {
                    Some(size) => *slot = size,
                    None => {
                        *slot = first_run;
                        invalid = true;
                    }
                }
            }
            "text_size" => match whole(value).and_then(TextSize::new) {
                Some(size) => settings.text_size = size,
                None => {
                    settings.text_size = TextSize::FIRST_RUN;
                    invalid = true;
                }
            },
            "save_folder" => match valid_folder(value) {
                Some(folder) => settings.save_folder = folder,
                None => {
                    settings.save_folder = default_folder.to_owned();
                    invalid = true;
                }
            },
            "start_with_windows" => match value {
                "true" => settings.start_with_windows = true,
                "false" => settings.start_with_windows = false,
                _ => {
                    settings.start_with_windows = false;
                    invalid = true;
                }
            },
            "head" => match Head::ALL.into_iter().find(|h| head_name(*h) == value) {
                Some(head) => settings.head = head,
                None => {
                    settings.head = Head::FIRST_RUN;
                    invalid = true;
                }
            },
            "fill" => match value {
                // Earlier builds' Fill toggle, which the next write gives its new name.
                "true" => settings.fill = Fill::Solid,
                "false" => settings.fill = Fill::Outline,
                _ => match Fill::ALL.into_iter().find(|f| fill_name(*f) == value) {
                    Some(fill) => settings.fill = fill,
                    None => {
                        settings.fill = Fill::FIRST_RUN;
                        invalid = true;
                    }
                },
            },
            "blur" => match whole(value).and_then(Level::new) {
                Some(level) => settings.blur = level,
                None => {
                    settings.blur = Level::FIRST_RUN;
                    invalid = true;
                }
            },
            "theme" => match Theme::ALL.into_iter().find(|t| theme_name(*t) == value) {
                Some(theme) => settings.theme = theme,
                None => {
                    settings.theme = Theme::System;
                    invalid = true;
                }
            },
            "window_background" => match Background::ALL
                .into_iter()
                .find(|b| background_name(*b) == value)
            {
                Some(background) => settings.window_background = background,
                None => {
                    settings.window_background = Background::TrimShadow;
                    invalid = true;
                }
            },
            _ => {}
        }
    }
    let (hotkeys, repeated) = hotkey::without_repeats(settings.hotkeys);
    settings.hotkeys = hotkeys;
    (settings, invalid || repeated)
}

/// The whole number `value` is, if it is one in digits only, so a sign or a decimal point is
/// none.
fn whole(value: &str) -> Option<u32> {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse().ok())
        .flatten()
}

/// The folder `text` names, if it can be one: a full path, on a drive or a share, whose names
/// Windows allows. Whether it exists is not asked.
pub fn valid_folder(text: &str) -> Option<PathBuf> {
    let path = PathBuf::from(text.trim());
    let names_allowed = path.components().all(|component| match component {
        Component::Normal(name) => name_allowed(&name.to_string_lossy()),
        _ => true,
    });
    (path.is_absolute() && names_allowed).then_some(path)
}

/// Whether Windows allows `name` for a folder: no forbidden character, no device name such as
/// `CON` or `com1.txt`, and no dot or space at the end, which Windows would drop.
fn name_allowed(name: &str) -> bool {
    let forbidden = |c: char| c < ' ' || r#"<>:"|?*"#.contains(c);
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| ('1'..='9').contains(&c)));
    !name.contains(forbidden) && !device && !name.ends_with(['.', ' '])
}

/// The text of a settings file that says `settings`.
fn to_text(settings: &Settings) -> String {
    let mut text = String::new();
    for mode in Mode::ALL {
        let value = hotkey::text(settings.hotkeys[mode as usize]);
        text += &format!("{}={value}\r\n", file_key(mode));
    }
    text += &format!(
        "save_folder={}\r\nstart_with_windows={}\r\ntheme={}\r\nwindow_background={}\r\n",
        settings.save_folder.display(),
        settings.start_with_windows,
        theme_name(settings.theme),
        background_name(settings.window_background)
    );
    let sizes = settings.sizes;
    text += &format!(
        "color={}\r\narrow_size={}\r\nrect_size={}\r\npen_size={}\r\ncounter_size={}\r\ntext_size={}\r\n",
        colour_name(settings.colour),
        sizes.arrow.get(),
        sizes.rect.get(),
        sizes.pen.get(),
        sizes.counter.get(),
        settings.text_size.get(),
    );
    text += &format!(
        "head={}\r\nfill={}\r\nblur={}\r\n",
        head_name(settings.head),
        fill_name(settings.fill),
        settings.blur.get()
    );
    text
}

/// The key in the settings file that holds `mode`'s hotkey.
fn file_key(mode: Mode) -> &'static str {
    match mode {
        Mode::Capture => "hotkey_capture",
        Mode::Screen => "hotkey_screen",
        Mode::Window => "hotkey_window",
        Mode::CopyText => "hotkey_text",
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn background_name(background: Background) -> &'static str {
    match background {
        Background::TrimShadow => "trim",
        Background::Shadow => "shadow",
        Background::Wallpaper => "wallpaper",
    }
}

fn head_name(head: Head) -> &'static str {
    match head {
        Head::One => "one",
        Head::Both => "both",
        Head::None => "none",
    }
}

fn fill_name(fill: Fill) -> &'static str {
    match fill {
        Fill::Outline => "outline",
        Fill::Tint => "tint",
        Fill::Solid => "solid",
    }
}

fn colour_name(colour: Colour) -> &'static str {
    match colour {
        Colour::Red => "red",
        Colour::Orange => "orange",
        Colour::Yellow => "yellow",
        Colour::Green => "green",
        Colour::Blue => "blue",
        Colour::Purple => "purple",
        Colour::White => "white",
        Colour::Black => "black",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkeys(texts: [&str; 4]) -> [Option<Hotkey>; 4] {
        texts.map(|text| Hotkey::parse(text).unwrap())
    }

    /// Where the user's Pictures folder puts the default save folder.
    const PICTURES: &str = r"C:\Users\me\Pictures\Zilum";

    fn parse(text: &str) -> (Settings, bool) {
        super::parse(text, Path::new(PICTURES))
    }

    fn defaults() -> Settings {
        Settings::defaults(PathBuf::from(PICTURES))
    }

    #[test]
    fn the_save_folder_and_start_with_windows_are_read_beside_the_rest() {
        let (settings, invalid) =
            parse("save_folder=D:\\Shots\\Zilum\nstart_with_windows=true\ncolor=blue\n");
        assert_eq!(settings.save_folder, PathBuf::from(r"D:\Shots\Zilum"));
        assert!(settings.start_with_windows);
        assert_eq!(settings.colour, Colour::Blue);
        assert!(!invalid);
    }

    fn sizes([arrow, rect, pen, counter]: [u32; 4]) -> Sizes {
        let stroke = |value| Size::new(value, Span::STROKES).unwrap();
        Sizes {
            arrow: stroke(arrow),
            rect: stroke(rect),
            pen: stroke(pen),
            counter: Size::new(counter, Span::BADGES).unwrap(),
        }
    }

    #[test]
    fn a_colour_and_each_tools_size_are_read_from_key_value_lines() {
        let (settings, invalid) =
            parse("color=blue\narrow_size=12\nrect_size=1\npen_size=7\ncounter_size=64\n");
        assert_eq!(settings.colour, Colour::Blue);
        assert_eq!(settings.sizes, sizes([12, 1, 7, 64]));
        assert!(!invalid);
    }

    #[test]
    fn a_missing_size_is_3_for_arrows_rectangles_and_strokes_and_32_for_badges() {
        assert_eq!(defaults().sizes, sizes([3, 3, 3, 32]));
        let (settings, invalid) = parse("pen_size=10\n");
        assert_eq!(settings.sizes, sizes([3, 3, 10, 32]));
        assert!(!invalid);
    }

    #[test]
    fn an_earlier_builds_size_is_medium_for_every_tool_and_not_flagged() {
        for value in ["thin", "medium", "thick", "huge", ""] {
            let (settings, invalid) = parse(&format!("size={value}\ncolor=blue\n"));
            assert_eq!(settings.sizes, Sizes::FIRST_RUN, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value:?}");
        }
        // Beside the new keys, before or after them, it leaves them as they say.
        for text in [
            "size=thin\narrow_size=9\ncounter_size=20\n",
            "arrow_size=9\ncounter_size=20\nsize=thick\n",
        ] {
            let (settings, invalid) = parse(text);
            assert_eq!(settings.sizes, sizes([9, 3, 3, 20]), "{text:?}");
            assert!(!invalid, "{text:?}");
        }
    }

    #[test]
    fn an_earlier_builds_size_is_written_back_without_it() {
        let (settings, _) = parse("color=blue\nsize=thick\nfill=true\n");
        let text = to_text(&settings);
        assert!(!text.contains("\nsize="), "{text}");
        assert!(
            text.ends_with(
                "color=blue\r\narrow_size=3\r\nrect_size=3\r\npen_size=3\r\ncounter_size=32\r\ntext_size=20\r\nhead=one\r\nfill=solid\r\nblur=8\r\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_size_out_of_its_tools_range_or_unreadable_is_flagged_and_has_its_default() {
        for (key, value) in [
            ("arrow_size", "0"),
            ("arrow_size", "13"),
            ("rect_size", "-3"),
            ("rect_size", "+3"),
            ("pen_size", "3.5"),
            ("pen_size", "thick"),
            ("pen_size", ""),
            ("counter_size", "15"),
            ("counter_size", "65"),
            ("counter_size", "3"),
            ("counter_size", "999999999999"),
        ] {
            let (settings, invalid) = parse(&format!(
                "arrow_size=9\nrect_size=9\npen_size=9\ncounter_size=50\n{key}={value}\ncolor=blue\n"
            ));
            let expected = match key {
                "arrow_size" => sizes([3, 9, 9, 50]),
                "rect_size" => sizes([9, 3, 9, 50]),
                "pen_size" => sizes([9, 9, 3, 50]),
                _ => sizes([9, 9, 9, 32]),
            };
            assert_eq!(settings.sizes, expected, "{key}={value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{key}={value:?}");
        }
    }

    #[test]
    fn start_with_windows_is_true_or_false_and_anything_else_is_flagged_and_off() {
        assert_eq!(parse("start_with_windows=false\n"), (defaults(), false));
        for value in ["banana", "1", "yes", ""] {
            let (settings, invalid) = parse(&format!("start_with_windows={value}\ncolor=blue\n"));
            assert!(!settings.start_with_windows, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn a_save_folder_that_cannot_be_a_folder_is_flagged_and_has_the_default() {
        for value in [
            "",
            r"C:\bad|name",
            r"C:\what?",
            r"C:\a<b>",
            "C:\\quote\"d",
            r"C:\star*",
            r"C:\x\y:z",
            r"Shots\Zilum",
            r"C:Shots",
            r"\Shots",
            r"C:\CON",
            r"C:\a\nul\b",
            r"C:\Com1.txt",
            r"C:\LPT9",
            r"C:\trailing dot.",
            r"C:\trailing space \x",
        ] {
            let (settings, invalid) = parse(&format!("save_folder={value}\ncolor=blue\n"));
            assert_eq!(settings.save_folder, PathBuf::from(PICTURES), "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn a_save_folder_on_a_drive_or_a_share_is_read() {
        for value in [
            r"D:\",
            r"D:\Shots",
            r"C:\Users\me\My Shots\2026",
            r"\\server\share\Shots",
            r"C:\Console\COM10\nullish\.hidden",
        ] {
            let (settings, invalid) = parse(&format!("save_folder={value}\n"));
            assert_eq!(settings.save_folder, PathBuf::from(value), "{value:?}");
            assert!(!invalid, "{value:?}");
        }
    }

    #[test]
    fn a_value_that_is_not_an_allowed_name_is_flagged_and_its_key_has_the_default() {
        let (settings, invalid) = parse("color=teal\narrow_size=9\n");
        assert_eq!(settings.colour, Colour::Red);
        assert_eq!(settings.sizes.arrow.get(), 9.0);
        assert!(invalid);
    }

    #[test]
    fn comments_unknown_keys_and_blank_lines_are_ignored() {
        let (settings, invalid) =
            parse("# arrow_size=huge\nhotkey=nonsense\n\nno equals sign\ncolor=green\n");
        assert_eq!(settings.colour, Colour::Green);
        assert_eq!(settings.sizes, Sizes::FIRST_RUN);
        assert_eq!(settings.hotkeys, Mode::DEFAULTS);
        assert!(!invalid);
    }

    #[test]
    fn a_file_saved_by_notepad_with_a_byte_order_mark_and_crlf_lines_is_read() {
        let (settings, invalid) = parse("\u{feff}color=green\r\npen_size=2\r\n");
        assert_eq!(settings.colour, Colour::Green);
        assert_eq!(settings.sizes.pen.get(), 2.0);
        assert!(!invalid);
    }

    #[test]
    fn spaces_around_a_key_or_a_value_are_ignored() {
        let (settings, invalid) =
            parse("color = green \n  pen_size=2\t\n hotkey_capture = Ctrl+Alt+R \n");
        assert_eq!(settings.colour, Colour::Green);
        assert_eq!(settings.sizes.pen.get(), 2.0);
        assert_eq!(settings.hotkeys[0], Hotkey::parse("Ctrl+Alt+R").unwrap());
        assert!(!invalid);
    }

    #[test]
    fn the_four_hotkeys_are_read_beside_the_colour_and_sizes() {
        let (settings, invalid) = parse(
            "hotkey_capture=Ctrl+Alt+R\nhotkey_screen=None\nhotkey_window=shift+f12\nhotkey_text=None\ncolor=blue\ncounter_size=40\n",
        );
        assert_eq!(
            settings.hotkeys,
            hotkeys(["Ctrl+Alt+R", "None", "Shift+F12", "None"])
        );
        assert_eq!(settings.colour, Colour::Blue);
        assert_eq!(settings.sizes.counter.get(), 40.0);
        assert!(!invalid);
    }

    #[test]
    fn an_m5_files_hotkey_region_is_the_capture_hotkey_and_is_written_as_hotkey_capture() {
        let (settings, invalid) = parse("hotkey_region=F12\ncolor=blue\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys(["F12", "Shift+PrintScreen", "None", "None"])
        );
        assert!(!invalid);
        let text = to_text(&settings);
        assert!(text.starts_with("hotkey_capture=F12\r\n"), "{text}");
        assert!(!text.contains("hotkey_region"), "{text}");
    }

    #[test]
    fn with_both_hotkey_capture_and_hotkey_region_hotkey_capture_is_read() {
        for text in [
            "hotkey_capture=Ctrl+Alt+R\nhotkey_region=F12\n",
            "hotkey_region=F12\nhotkey_capture=Ctrl+Alt+R\n",
            "hotkey_capture=Ctrl+Alt+R\nhotkey_region=Banana\n",
        ] {
            let (settings, invalid) = parse(text);
            assert_eq!(
                settings.hotkeys[0],
                Hotkey::parse("Ctrl+Alt+R").unwrap(),
                "{text:?}"
            );
            assert!(!invalid, "{text:?}");
        }
    }

    #[test]
    fn a_missing_hotkey_has_its_default() {
        let (settings, invalid) = parse("hotkey_screen=F12\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys(["PrintScreen", "F12", "None", "None"])
        );
        assert!(!invalid);
        assert_eq!(parse(""), (defaults(), false));
    }

    #[test]
    fn the_window_hotkey_is_none_unless_the_file_sets_it() {
        assert_eq!(
            defaults().hotkeys,
            hotkeys(["PrintScreen", "Shift+PrintScreen", "None", "None"])
        );
        let (settings, invalid) = parse("hotkey_window=Ctrl+Shift+PrintScreen\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys([
                "PrintScreen",
                "Shift+PrintScreen",
                "Ctrl+Shift+PrintScreen",
                "None"
            ])
        );
        assert!(!invalid);
    }

    #[test]
    fn the_copy_text_hotkey_is_none_unless_the_file_sets_it_and_is_written_after_window() {
        assert_eq!(defaults().hotkeys[Mode::CopyText as usize], None);
        let (settings, invalid) = parse("hotkey_text=ctrl+alt+t\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys(["PrintScreen", "Shift+PrintScreen", "None", "Ctrl+Alt+T"])
        );
        assert!(!invalid);
        assert!(
            to_text(&settings).starts_with(
                "hotkey_capture=PrintScreen\r\nhotkey_screen=Shift+PrintScreen\r\nhotkey_window=None\r\nhotkey_text=Ctrl+Alt+T\r\nsave_folder="
            ),
            "{}",
            to_text(&settings)
        );
    }

    #[test]
    fn an_invalid_copy_text_hotkey_is_flagged_and_none() {
        for value in ["Banana", "T", "Win+T", ""] {
            let (settings, invalid) = parse(&format!(
                "hotkey_text=Ctrl+Alt+T\nhotkey_text={value}\ncolor=blue\n"
            ));
            assert_eq!(settings.hotkeys, Mode::DEFAULTS, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn a_copy_text_hotkey_repeating_a_capture_hotkey_is_flagged_and_none() {
        let (settings, invalid) = parse("hotkey_window=F9\nhotkey_text=f9\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys(["PrintScreen", "Shift+PrintScreen", "F9", "None"])
        );
        assert!(invalid);
    }

    #[test]
    fn an_invalid_hotkey_is_flagged_and_has_its_default() {
        for value in ["Banana", "R", "5", "Win+R", ""] {
            let (settings, invalid) = parse(&format!("hotkey_window={value}\ncolor=blue\n"));
            assert_eq!(settings.hotkeys, Mode::DEFAULTS, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn a_hotkey_repeating_an_earlier_modes_is_flagged_and_leaves_that_mode_with_none() {
        let (settings, invalid) = parse("hotkey_screen=PrintScreen\n");
        assert_eq!(
            settings.hotkeys,
            hotkeys(["PrintScreen", "None", "None", "None"])
        );
        assert!(invalid);
    }

    #[test]
    fn settings_are_written_as_one_key_value_line_each_and_nothing_else() {
        let (settings, _) = parse(
            "# a comment\nhotkey_capture=alt+shift+ctrl+r\nhotkey=nonsense\ncolor=blue\nrect_size=11\nhotkey_window=None\nstart_with_windows=true\ntheme=light\n",
        );
        assert_eq!(
            to_text(&settings),
            "hotkey_capture=Ctrl+Alt+Shift+R\r\nhotkey_screen=Shift+PrintScreen\r\nhotkey_window=None\r\nhotkey_text=None\r\nsave_folder=C:\\Users\\me\\Pictures\\Zilum\r\nstart_with_windows=true\r\ntheme=light\r\nwindow_background=trim\r\ncolor=blue\r\narrow_size=3\r\nrect_size=11\r\npen_size=3\r\ncounter_size=32\r\ntext_size=20\r\nhead=one\r\nfill=outline\r\nblur=8\r\n"
        );
    }

    #[test]
    fn head_is_one_both_or_none_and_one_when_missing() {
        assert_eq!(defaults().head, Head::One);
        assert_eq!(parse("color=blue\n").0.head, Head::One);
        for (value, head) in [
            ("one", Head::One),
            ("both", Head::Both),
            ("none", Head::None),
        ] {
            let (settings, invalid) = parse(&format!("head={value}\ncolor=blue\n"));
            assert_eq!(settings.head, head, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value:?}");
        }
    }

    #[test]
    fn any_other_head_is_flagged_and_one() {
        for value in ["two", "Both", "1", "true", ""] {
            let (settings, invalid) = parse(&format!("head=none\nhead={value}\ncolor=blue\n"));
            assert_eq!(settings.head, Head::One, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn head_is_written_after_the_text_size() {
        let (settings, _) = parse("head=both\n");
        assert!(
            to_text(&settings).ends_with("text_size=20\r\nhead=both\r\nfill=outline\r\nblur=8\r\n")
        );
    }

    #[test]
    fn fill_is_outline_tint_or_solid_and_outline_when_missing() {
        assert_eq!(defaults().fill, Fill::Outline);
        assert_eq!(parse("color=blue\n").0.fill, Fill::Outline);
        for (value, fill) in [
            ("outline", Fill::Outline),
            ("tint", Fill::Tint),
            ("solid", Fill::Solid),
        ] {
            let (settings, invalid) = parse(&format!("fill={value}\ncolor=blue\n"));
            assert_eq!(settings.fill, fill, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value:?}");
        }
    }

    #[test]
    fn an_earlier_builds_fill_is_read_as_solid_or_outline_and_written_back_in_the_new_form() {
        for (value, fill, written) in [
            ("true", Fill::Solid, "\r\nfill=solid\r\n"),
            ("false", Fill::Outline, "\r\nfill=outline\r\n"),
        ] {
            let (settings, invalid) = parse(&format!("fill={value}\ncolor=blue\n"));
            assert_eq!(settings.fill, fill, "{value:?}");
            assert!(!invalid, "{value:?}");
            let text = to_text(&settings);
            assert!(text.contains(written), "{text}");
        }
    }

    #[test]
    fn any_other_fill_is_flagged_and_outline() {
        for value in ["maybe", "True", "Tint", "1", "on", "filled", ""] {
            let (settings, invalid) = parse(&format!("fill=solid\nfill={value}\ncolor=blue\n"));
            assert_eq!(settings.fill, Fill::Outline, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn fill_is_written_after_the_head() {
        let (settings, _) = parse("fill=tint\n");
        assert!(to_text(&settings).ends_with(
            "color=red\r\narrow_size=3\r\nrect_size=3\r\npen_size=3\r\ncounter_size=32\r\ntext_size=20\r\nhead=one\r\nfill=tint\r\nblur=8\r\n"
        ));
        assert!(to_text(&defaults()).ends_with(
            "counter_size=32\r\ntext_size=20\r\nhead=one\r\nfill=outline\r\nblur=8\r\n"
        ));
    }

    #[test]
    fn the_text_size_is_6_to_400_and_20_when_missing() {
        assert_eq!(defaults().text_size.get(), 20);
        assert_eq!(parse("color=blue\n").0.text_size.get(), 20);
        for value in [6, 41, 400] {
            let (settings, invalid) = parse(&format!("text_size={value}\ncolor=blue\n"));
            assert_eq!(settings.text_size.get(), value, "{value}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value}");
        }
    }

    #[test]
    fn any_other_text_size_is_flagged_and_20() {
        for value in [
            "5",
            "401",
            "0",
            "-20",
            "+20",
            "20.5",
            "medium",
            "99999999999",
            "",
        ] {
            let (settings, invalid) =
                parse(&format!("text_size=40\ntext_size={value}\ncolor=blue\n"));
            assert_eq!(settings.text_size.get(), 20, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn the_blur_level_is_4_to_40_and_8_when_missing() {
        assert_eq!(defaults().blur, Level::FIRST_RUN);
        assert_eq!(parse("color=blue\n").0.blur.get(), 8.0);
        for value in [4, 13, 40] {
            let (settings, invalid) = parse(&format!("blur={value}\ncolor=blue\n"));
            assert_eq!(settings.blur.get(), value as f32, "{value}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value}");
        }
    }

    #[test]
    fn any_other_blur_level_is_flagged_and_8() {
        for value in ["99", "3", "41", "0", "-8", "+8", "8.0", "medium", ""] {
            let (settings, invalid) = parse(&format!("blur=20\nblur={value}\ncolor=blue\n"));
            assert_eq!(settings.blur.get(), 8.0, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn the_blur_level_is_written_after_fill() {
        let (settings, _) = parse("blur=40\n");
        assert!(to_text(&settings).ends_with(
            "counter_size=32\r\ntext_size=20\r\nhead=one\r\nfill=outline\r\nblur=40\r\n"
        ));
        assert!(to_text(&defaults()).ends_with("fill=outline\r\nblur=8\r\n"));
    }

    #[test]
    fn the_window_background_is_trim_shadow_or_wallpaper_and_trim_when_missing() {
        assert_eq!(defaults().window_background, Background::TrimShadow);
        for (value, background) in [
            ("trim", Background::TrimShadow),
            ("shadow", Background::Shadow),
            ("wallpaper", Background::Wallpaper),
        ] {
            let (settings, invalid) = parse(&format!("window_background={value}\ncolor=blue\n"));
            assert_eq!(settings.window_background, background, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value:?}");
        }
    }

    #[test]
    fn any_other_window_background_is_flagged_and_trim() {
        for value in ["glow", "Shadow", "none", ""] {
            let (settings, invalid) = parse(&format!(
                "window_background=shadow\nwindow_background={value}\ncolor=blue\n"
            ));
            assert_eq!(
                settings.window_background,
                Background::TrimShadow,
                "{value:?}"
            );
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn the_window_background_is_written_after_the_theme() {
        let (settings, _) = parse("window_background=shadow\n");
        let text = to_text(&settings);
        assert!(
            text.contains("\r\ntheme=system\r\nwindow_background=shadow\r\ncolor="),
            "{text}"
        );
        assert!(to_text(&defaults()).contains("\r\nwindow_background=trim\r\n"));
    }

    #[test]
    fn the_theme_is_system_light_or_dark_and_system_when_missing() {
        assert_eq!(defaults().theme, Theme::System);
        for (value, theme) in [
            ("system", Theme::System),
            ("light", Theme::Light),
            ("dark", Theme::Dark),
        ] {
            let (settings, invalid) = parse(&format!("theme={value}\ncolor=blue\n"));
            assert_eq!(settings.theme, theme, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(!invalid, "{value:?}");
        }
    }

    #[test]
    fn any_other_theme_is_flagged_and_has_the_default() {
        for value in ["teal", "Dark", "auto", ""] {
            let (settings, invalid) = parse(&format!("theme=dark\ntheme={value}\ncolor=blue\n"));
            assert_eq!(settings.theme, Theme::System, "{value:?}");
            assert_eq!(settings.colour, Colour::Blue);
            assert!(invalid, "{value:?}");
        }
    }

    #[test]
    fn every_style_head_fill_level_hotkey_and_theme_reads_back_as_written() {
        for colour in Colour::ALL {
            // Each tool's least, most and something between, with each colour.
            for step in 0..3 {
                let stroke = |offset: u32| 1 + (11 * step + offset) % 12;
                for theme in Theme::ALL {
                    let settings = Settings {
                        hotkeys: hotkeys(["Alt+F3", "None", "Ctrl+Shift+5", "F9"]),
                        colour,
                        sizes: sizes([stroke(0), stroke(5), stroke(colour as u32), 16 + 24 * step]),
                        text_size: TextSize::new(6 + 197 * step).unwrap(),
                        // Each Fill at a different step for each colour.
                        fill: Fill::ALL[(colour as usize + 2 * step as usize) % 3],
                        // Each head at a different step for each colour.
                        head: Head::ALL[(colour as usize + step as usize) % 3],
                        // A level for each colour and step, ends included.
                        blur: Level::new(4 + 36 * (3 * colour as u32 + step) / 23).unwrap(),
                        save_folder: PathBuf::from(r"\\server\share\My Shots"),
                        start_with_windows: true,
                        theme,
                        // Each background with every theme.
                        window_background: Background::ALL[colour as usize % 3],
                    };
                    assert_eq!(
                        parse(&to_text(&settings)),
                        (settings.clone(), false),
                        "{settings:?}"
                    );
                }
            }
        }
        assert_eq!(parse(&to_text(&defaults())), (defaults(), false));
    }
}
