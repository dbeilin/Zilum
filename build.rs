//! Embeds the icon Explorer, the taskbar and the tray show, and the version details Explorer's
//! Properties and Task Manager read.
//!
//! The resource file is written here and handed straight to the linker, which converts it, so
//! the build needs no resource compiler: `rc.exe` comes with the Windows SDK, not the Rust
//! toolchain, and once vanished from the development machine on its own.

use std::{env, fs, path::PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// English (United States), the language `rc.exe` gives resources by default.
const LANGUAGE: u16 = 0x0409;
/// The ID of the icon group, which Explorer shows for the exe and `load_icon` loads.
const ICON_ID: u16 = 1;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        println!("cargo:warning=no icon or version details: only the MSVC linker takes a .res");
        return;
    }
    let ico = fs::read("assets/icon.ico").expect("assets/icon.ico is readable");

    let mut res = Vec::new();
    // A resource file opens with an empty entry, which marks it as 32-bit.
    entry(&mut res, 0, 0, 0, 0, &[]);
    icons(&mut res, &ico);
    entry(&mut res, RT_VERSION, 1, 0x0030, LANGUAGE, &version());

    let path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("zilum.res");
    fs::write(&path, res).expect("the resource file is writable");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

/// Appends one resource with a numeric type and ID, padded to a 4-byte boundary.
fn entry(res: &mut Vec<u8>, kind: u16, id: u16, flags: u16, language: u16, data: &[u8]) {
    res.extend((data.len() as u32).to_le_bytes());
    res.extend(32u32.to_le_bytes()); // this header's size
    res.extend(
        [0xFFFF, kind, 0xFFFF, id]
            .iter()
            .flat_map(|w: &u16| w.to_le_bytes()),
    );
    res.extend(0u32.to_le_bytes()); // data version
    res.extend(flags.to_le_bytes());
    res.extend(language.to_le_bytes());
    res.extend(0u32.to_le_bytes()); // version
    res.extend(0u32.to_le_bytes()); // characteristics
    res.extend(data);
    pad(res);
}

/// Each image of the .ico as its own icon resource, numbered from 1, and the group listing them.
fn icons(res: &mut Vec<u8>, ico: &[u8]) {
    let word = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let dword = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(
        (word(0), word(2)),
        (0, 1),
        "assets/icon.ico is an icon file"
    );
    let count = word(4);
    let mut group = Vec::new();
    group.extend([0u16, 1, count].iter().flat_map(|w| w.to_le_bytes()));
    for n in 0..count {
        let at = 6 + 16 * n as usize;
        let (size, offset) = (dword(at + 8), dword(at + 12));
        entry(
            res,
            RT_ICON,
            n + 1,
            0x1010,
            LANGUAGE,
            &ico[offset..offset + size],
        );
        // The directory entry as the .ico has it, with the image's offset swapped for its ID.
        group.extend(&ico[at..at + 12]);
        group.extend((n + 1).to_le_bytes());
    }
    entry(res, RT_GROUP_ICON, ICON_ID, 0x1030, LANGUAGE, &group);
}

/// The version resource. Task Manager names a process by its file description, so that is the
/// app's name, not the package's one-line description.
fn version() -> Vec<u8> {
    let part = |name: &str| env::var(name).unwrap().parse::<u32>().unwrap();
    let (major, minor, patch) = (
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH"),
    );
    let text = env::var("CARGO_PKG_VERSION").unwrap();
    let (ms, ls) = (major << 16 | minor, patch << 16);
    // VS_FIXEDFILEINFO: signature, structure version, file and product versions, flags mask,
    // flags, Windows NT, an application, no subtype and no date.
    let fixed: Vec<u8> = [
        0xFEEF_04BD,
        0x0001_0000,
        ms,
        ls,
        ms,
        ls,
        0x3F,
        0,
        0x0004_0004,
        1,
        0,
        0,
        0,
    ]
    .iter()
    .flat_map(|d: &u32| d.to_le_bytes())
    .collect();
    let strings = [
        ("FileDescription", "Zilum"),
        ("FileVersion", text.as_str()),
        ("OriginalFilename", "Zilum.exe"),
        ("ProductName", "Zilum"),
        ("ProductVersion", text.as_str()),
    ]
    .iter()
    .map(|(key, value)| {
        let value = wide(value);
        block(key, &value, (value.len() / 2) as u16, true, &[])
    })
    .collect::<Vec<_>>();
    // 040904B0: English (United States) in Unicode.
    let table = block("040904B0", &[], 0, true, &strings);
    let string_info = block("StringFileInfo", &[], 0, true, &[table]);
    let translation: Vec<u8> = [LANGUAGE, 0x04B0]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    let var = block("Translation", &translation, 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &[var]);
    block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len() as u16,
        false,
        &[string_info, var_info],
    )
}

/// One block of a version resource: its length, its value's length (in characters for text),
/// whether the value is text, its key, its value and its children, each on a 4-byte boundary.
fn block(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0];
    b.extend(value_length.to_le_bytes());
    b.extend(u16::from(text).to_le_bytes());
    b.extend(wide(key));
    pad(&mut b);
    b.extend(value);
    for child in children {
        pad(&mut b);
        b.extend(child);
    }
    let length = (b.len() as u16).to_le_bytes();
    b[..2].copy_from_slice(&length);
    b
}

/// `text` as UTF-16 with its terminating null.
fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(|w| w.to_le_bytes())
        .collect()
}

fn pad(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}
