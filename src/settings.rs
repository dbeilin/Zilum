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
    let lines = text
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()));
    for (key, value) in lines {
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
            "fill" => match Fill::ALL.into_iter().find(|f| fill_name(*f) == value) {
                Some(fill) => settings.fill = fill,
                None => {
                    settings.fill = Fill::FIRST_RUN;
                    invalid = true;
                }
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
    fn a_missing_size_is_3_for_arrows_rectangles_and_strokes_and_32_for_badges() {
        assert_eq!(defaults().sizes, sizes([3, 3, 3, 32]));
        let (settings, invalid) = parse("pen_size=10\n");
        assert_eq!(settings.sizes, sizes([3, 3, 10, 32]));
        assert!(!invalid);
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
    fn an_invalid_value_is_flagged_and_its_key_has_the_default() {
        for line in [
            "color=teal",
            "start_with_windows=yes",
            "start_with_windows=1",
            "head=two",
            "head=Both",
            "fill=true",
            "fill=Tint",
            "text_size=5",
            "text_size=401",
            "text_size=+20",
            "text_size=20.5",
            "text_size=99999999999",
            "blur=3",
            "blur=41",
            "blur=8.0",
            "theme=Dark",
            "theme=auto",
            "window_background=glow",
            "hotkey_window=Banana",
            "hotkey_window=Win+R",
            "hotkey_text=T",
            "head=",
        ] {
            let (settings, invalid) = parse(&format!("{line}\npen_size=7\n"));
            let expected = Settings {
                sizes: sizes([3, 3, 7, 32]),
                ..defaults()
            };
            assert_eq!(settings, expected, "{line:?}");
            assert!(invalid, "{line:?}");
        }
        // A later invalid line undoes an earlier valid one.
        assert_eq!(parse("theme=dark\ntheme=auto\n"), (defaults(), true));
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
