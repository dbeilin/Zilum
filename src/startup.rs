//! Start with Windows: the per-user Run entry, which starts Zilum when this user signs in, the
//! PC's Run entry an all-users install makes, which starts it at everyone's sign-in, and the
//! choice an installer made, which becomes the setting once per install.

use std::{os::windows::ffi::OsStrExt, path::Path};

use windows::{
    Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
    },
    core::{PCWSTR, w},
};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const NAME: PCWSTR = w!("Zilum");
/// The argument the Run entry passes, so a start at sign-in can tell itself from one the user made.
const FLAG: &str = "--startup";

/// The key the installer records its choice in, the person's for a me-only install and the PC's
/// for an all-users one, and the person's, where Zilum remembers which install's choice it has
/// taken. The names are the installer's too (installer\Zilum.iss).
const KEY: PCWSTR = w!(r"Software\Zilum");
/// Which install recorded the choice: a stamp the installer makes, new at each install.
const INSTALL: PCWSTR = w!("Install");
/// The choice: 1 for Start with Windows ticked, 0 for unticked.
const START_WITH_WINDOWS: PCWSTR = w!("StartWithWindows");
/// The stamp of the install whose choice this person has taken.
const TAKEN: PCWSTR = w!("Taken");

/// Whether Windows started this copy at sign-in, rather than the user.
pub fn at_sign_in() -> bool {
    std::env::args().skip(1).any(|arg| arg == FLAG)
}

/// Whether this start stays, given whether it is `at_sign_in` and whether the person's Start with
/// Windows is `on`. A start the user made always stays. A start at sign-in stays only while the
/// setting is on, since the PC's start comes at everyone's sign-in, theirs being off or not.
pub fn stays(at_sign_in: bool, on: bool) -> bool {
    !at_sign_in || on
}

/// Brings this person's Run entry in line with their Start with Windows `on`, at a start. While on,
/// it is set again, since it holds the exe's path. When off, it is cleared only where this start
/// has reason to think it is this exe's: an installer's choice `taken` just now, or the PC's start
/// naming this exe. Otherwise it may be another Zilum's, a Portable one's say, and is left.
pub fn refresh(on: bool, taken: bool) {
    if on || taken || std::env::current_exe().is_ok_and(|exe| machine_starts(&exe)) {
        set(on);
    }
}

/// Makes Zilum start at sign-in from where its exe is now, or stops it starting then. The entry
/// holds the exe's path and the sign-in flag, so it is set again at each start while on. Where
/// the PC's start already names this exe, this person's own entry is cleared rather than set, so
/// nobody gets two starts.
pub fn set(on: bool) {
    let exe = std::env::current_exe()
        .ok()
        .filter(|exe| own_entry(on, machine_starts(exe)));
    match exe {
        Some(exe) => write_text(RUN, NAME, &command(&exe)),
        None => {
            // SAFETY: Takes a predefined key and two NUL-ended string constants.
            let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) };
        }
    }
}

/// The Run entry's command for `exe`, NUL-ended: quoted, or a space in the path would end it
/// there, then the sign-in flag.
fn command(exe: &Path) -> Vec<u16> {
    [u16::from(b'"')]
        .into_iter()
        .chain(exe.as_os_str().encode_wide())
        .chain(format!("\" {FLAG}").encode_utf16())
        .chain([0])
        .collect()
}

/// Whether this person needs a Run entry of their own: while their Start with Windows is `on`,
/// unless the PC's start already starts this exe at their sign-in (`machine`).
fn own_entry(on: bool, machine: bool) -> bool {
    on && !machine
}

/// Whether the PC's Run entry, which an all-users install makes, starts `exe`.
fn machine_starts(exe: &Path) -> bool {
    read_text(HKEY_LOCAL_MACHINE, RUN, NAME).is_some_and(|start| names(&start, exe))
}

/// Whether the Run entry `start` starts `exe`. Windows' paths are not case-sensitive.
fn names(start: &str, exe: &Path) -> bool {
    start
        .to_lowercase()
        .starts_with(&format!("\"{}\"", exe.display()).to_lowercase())
}

/// The Start with Windows an installer's options page left, and the install it was left at.
#[cfg_attr(test, derive(Clone, Debug, PartialEq))]
pub struct Choice {
    /// The install's stamp.
    install: String,
    /// Whether Start with Windows was ticked.
    pub on: bool,
}

/// The installer's choice this Zilum is to make the setting now, if there is one. `portable` is
/// whether this is a Portable Zilum. A me-only install records it for the person, an all-users
/// one for the PC; were both there, the person's would be the one taken.
pub fn installer_choice(portable: bool) -> Option<Choice> {
    let recorded = recorded(HKEY_CURRENT_USER).or_else(|| recorded(HKEY_LOCAL_MACHINE));
    to_take(
        recorded,
        read_text(HKEY_CURRENT_USER, KEY, TAKEN).as_deref(),
        portable,
    )
}

/// The choice an install recorded in `root`'s Zilum key, if one did.
fn recorded(root: HKEY) -> Option<Choice> {
    let install = read_text(root, KEY, INSTALL)?;
    let on = read_number(root, START_WITH_WINDOWS)?;
    Some(Choice {
        install,
        on: on != 0,
    })
}

/// Remembers that this person has taken `choice`, so later starts leave the setting to them.
pub fn remember_taken(choice: &Choice) {
    let text: Vec<u16> = choice.install.encode_utf16().chain([0]).collect();
    write_text(KEY, TAKEN, &text);
}

/// Sets the value `name` of this person's `key` to `text`, which ends in a NUL.
fn write_text(key: PCWSTR, name: PCWSTR, text: &[u16]) {
    // SAFETY: `text` is the NUL-ended string whose byte length is passed.
    let _ = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            REG_SZ.0,
            Some(text.as_ptr().cast()),
            size_of_val(text) as u32,
        )
    };
}

/// The choice an install `recorded` that is still to be taken, given the stamp of the install
/// whose choice this person has `taken`: each install's choice is taken once, so a change made
/// in Settings afterwards holds. A Portable Zilum takes none, its settings being its own.
fn to_take(recorded: Option<Choice>, taken: Option<&str>, portable: bool) -> Option<Choice> {
    recorded.filter(|choice| !portable && taken != Some(choice.install.as_str()))
}

/// The text the value `name` of `root`'s `key` holds, if it holds any that fits a long path.
fn read_text(root: HKEY, key: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut text = [0u16; 1024];
    let mut size = size_of_val(&text) as u32;
    // SAFETY: `text` is the `size` bytes long that RegGetValueW is told.
    let status = unsafe {
        RegGetValueW(
            root,
            key,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    // The size counts the ending NUL.
    let length = (size as usize / size_of::<u16>()).saturating_sub(1);
    status
        .is_ok()
        .then(|| String::from_utf16_lossy(&text[..length]))
}

/// The number the value `name` of `root`'s Zilum key holds, if it holds one.
fn read_number(root: HKEY, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a u32, the size `size` gives.
    let status = unsafe {
        RegGetValueW(
            root,
            KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(install: &str, on: bool) -> Choice {
        Choice {
            install: install.to_owned(),
            on,
        }
    }

    #[test]
    fn an_installs_choice_not_yet_taken_is_taken_ticked_or_unticked() {
        for on in [true, false] {
            let recorded = choice("20261120153000123", on);
            assert_eq!(to_take(Some(recorded.clone()), None, false), Some(recorded));
        }
    }

    #[test]
    fn a_choice_already_taken_is_not_taken_again_so_a_later_change_in_settings_holds() {
        let recorded = choice("20261120153000123", true);
        assert_eq!(
            to_take(Some(recorded), Some("20261120153000123"), false),
            None
        );
    }

    #[test]
    fn a_later_installs_choice_is_taken_though_an_earlier_ones_was() {
        let recorded = choice("20261121090000456", false);
        assert_eq!(
            to_take(Some(recorded.clone()), Some("20261120153000123"), false),
            Some(recorded)
        );
    }

    #[test]
    fn a_portable_zilum_takes_no_installers_choice() {
        let recorded = choice("20261120153000123", true);
        assert_eq!(to_take(Some(recorded), None, true), None);
    }

    #[test]
    fn a_start_at_sign_in_stays_only_while_start_with_windows_is_on() {
        assert!(stays(true, true));
        assert!(!stays(true, false));
    }

    #[test]
    fn a_start_the_user_made_stays_whatever_start_with_windows_says() {
        assert!(stays(false, true));
        assert!(stays(false, false));
    }

    #[test]
    fn a_person_whose_start_with_windows_is_on_has_their_own_start_unless_the_pcs_starts_this_exe()
    {
        assert!(own_entry(true, false));
        assert!(!own_entry(true, true));
        assert!(!own_entry(false, false));
        assert!(!own_entry(false, true));
    }

    #[test]
    fn the_pcs_start_names_an_exe_by_its_quoted_path_whatever_the_case() {
        let exe = Path::new(r"C:\Program Files\Zilum\Zilum.exe");
        assert!(names(
            r#""C:\Program Files\Zilum\Zilum.exe" --startup"#,
            exe
        ));
        assert!(names(
            r#""c:\program files\zilum\ZILUM.EXE" --startup"#,
            exe
        ));
        assert!(!names(
            r#""C:\Program Files\Zilum\Zilum.exe.old" --startup"#,
            exe
        ));
        assert!(!names(r#""D:\Zilum\Zilum.exe" --startup"#, exe));
        let command = command(exe);
        assert!(names(
            &String::from_utf16_lossy(&command[..command.len() - 1]),
            exe
        ));
    }

    #[test]
    fn with_no_install_recorded_there_is_nothing_to_take() {
        assert_eq!(to_take(None, Some("20261120153000123"), false), None);
        assert_eq!(to_take(None, None, false), None);
    }
}
