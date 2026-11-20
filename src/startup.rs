//! Start with Windows: the per-user Run entry, which starts Zilum when this user signs in, and
//! the choice an installer made for it, which becomes the setting once per install.

use std::os::windows::ffi::OsStrExt;

use windows::{
    Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
        RegGetValueW, RegSetKeyValueW,
    },
    core::{PCWSTR, w},
};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const NAME: PCWSTR = w!("Zilum");
/// The argument the Run entry passes, so a start at sign-in can tell itself from one the user made.
const FLAG: &str = "--startup";

/// The key the installer records its choice in, and Zilum which install's choice it has taken.
/// The names are the installer's too (installer\Zilum.iss).
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

/// Makes Zilum start at sign-in from where its exe is now, or stops it starting then. The entry
/// holds the exe's path and the sign-in flag, so it is set again at each start while on.
pub fn set(on: bool) {
    if !on {
        // SAFETY: Takes a predefined key and two NUL-ended string constants.
        let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) };
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // Quoted, or a space in the path would end the command there.
    let command: Vec<u16> = [u16::from(b'"')]
        .into_iter()
        .chain(exe.as_os_str().encode_wide())
        .chain(format!("\" {FLAG}").encode_utf16())
        .chain([0])
        .collect();
    write_text(RUN, NAME, &command);
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
/// whether this is a Portable Zilum.
pub fn installer_choice(portable: bool) -> Option<Choice> {
    let recorded = read_text(INSTALL).and_then(|install| {
        let on = read_number(START_WITH_WINDOWS)?;
        Some(Choice {
            install,
            on: on != 0,
        })
    });
    to_take(recorded, read_text(TAKEN).as_deref(), portable)
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

/// The text the value `name` of Zilum's key holds, if it holds any.
fn read_text(name: PCWSTR) -> Option<String> {
    let mut text = [0u16; 64];
    let mut size = size_of_val(&text) as u32;
    // SAFETY: `text` is the `size` bytes long that RegGetValueW is told.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
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

/// The number the value `name` of Zilum's key holds, if it holds one.
fn read_number(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a u32, the size `size` gives.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
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
    fn with_no_install_recorded_there_is_nothing_to_take() {
        assert_eq!(to_take(None, Some("20261120153000123"), false), None);
        assert_eq!(to_take(None, None, false), None);
    }
}
