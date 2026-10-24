//! Start with Windows: the per-user Run entry, which starts Zilum when this user signs in.

use std::os::windows::ffi::OsStrExt;

use windows::{
    Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW},
    core::{PCWSTR, w},
};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const NAME: PCWSTR = w!("Zilum");

/// Makes Zilum start at sign-in from where its exe is now, or stops it starting then. The entry
/// holds the exe's path, so it is set again at each start while on.
pub fn set(on: bool) {
    // SAFETY: Win32 calls taking handles, integers and pointers to live locals.
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // Quoted, or a space in the path would end the command there.
        let command: Vec<u16> = [u16::from(b'"')]
            .into_iter()
            .chain(exe.as_os_str().encode_wide())
            .chain([u16::from(b'"'), 0])
            .collect();
        let _ = RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * size_of::<u16>()) as u32,
        );
    }
}
