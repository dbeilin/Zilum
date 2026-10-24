//! Saving a capture as a PNG in the save folder or where the user chooses, from a worker thread
//! so encoding and the disk never hold up the UI, and opening the save folder in Explorer.

use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{Sender, channel},
    },
};

use windows::{
    Win32::{
        Foundation::{
            E_FAIL, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, HWND, LPARAM, SYSTEMTIME, WPARAM,
        },
        Storage::FileSystem::{
            MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree,
        },
        UI::{
            Shell::{
                Common::COMDLG_FILTERSPEC, FOS_FORCEFILESYSTEM, FOS_NOTESTFILECREATE,
                FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileDialog, IFileOpenDialog,
                IFileSaveDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
                ShellExecuteW,
            },
            WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL},
        },
    },
    core::{Error, HSTRING, Result, w},
};

use crate::{editor, graphics, image::Image, settings, settings_window};

/// The file a capture was saved to. A capture owns at most one, which saving again replaces.
pub type SavedFile = Arc<Mutex<Option<PathBuf>>>;

struct Job {
    image: Arc<Image>,
    file: SavedFile,
    /// The file Save As chose, which becomes the capture's file. None for Save.
    to: Option<PathBuf>,
    /// The tray window, which shows the failure notification.
    tray: isize,
    /// The editor told how it went.
    editor: isize,
}

impl Job {
    /// The file to write: the one Save As chose, or else the capture's own, if it has one.
    fn target(&self) -> Option<PathBuf> {
        self.to
            .clone()
            .or_else(|| self.file.lock().unwrap().clone())
    }
}

/// The worker, started by the first save. One thread, so a second save of a capture finds the
/// file the first one made.
static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

/// Writes `image` soon to `to`, or else to `file` if it has been saved before, or else into the
/// save folder, and records where it went in `file`. Shows "Couldn't save" from `tray` if that
/// fails. Then posts `WM_SAVED` to `editor`, if it is still open.
pub fn save(image: Arc<Image>, file: SavedFile, to: Option<PathBuf>, tray: HWND, editor: HWND) {
    let worker = WORKER.get_or_init(|| {
        let (sender, jobs) = channel::<Job>();
        std::thread::spawn(move || {
            graphics::init_apartment();
            for job in jobs {
                let target = job.target();
                let mut name = first_name(&job.image, target.as_deref());
                let saved = match write(&job.image, target.as_deref(), &mut name) {
                    Ok(path) => {
                        *job.file.lock().unwrap() = Some(path);
                        true
                    }
                    Err(_) => {
                        notify_failure(HWND(job.tray as _), &name);
                        false
                    }
                };
                report(HWND(job.editor as _), saved);
            }
        });
        sender
    });
    let job = Job {
        image,
        file,
        to,
        tray: tray.0 as isize,
        editor: editor.0 as isize,
    };
    if let Err(job) = worker.send(job) {
        notify_failure(tray, &first_name(&job.0.image, job.0.target().as_deref()));
        report(editor, false);
    }
}

/// What the Notice after a save to `file` reads: the file's name alone, since the save folder is
/// one tray click away.
pub fn notice_text(file: &Path) -> String {
    let name = file.file_name().unwrap_or(file.as_os_str());
    format!("Saved as {}", name.to_string_lossy())
}

/// The name a save of `image` to `target` tries first: the target's, if there is one.
fn first_name(image: &Image, target: Option<&Path>) -> String {
    match target.and_then(Path::file_name) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => names(&image.taken).next().unwrap_or_default(),
    }
}

fn notify_failure(tray: HWND, name: &str) {
    unsafe { crate::notify(tray, &format!("Couldn't save {name}")) };
}

fn report(to: HWND, saved: bool) {
    let _ = unsafe {
        PostMessageW(
            Some(to),
            editor::WM_SAVED,
            WPARAM(saved as usize),
            LPARAM(0),
        )
    };
}

/// Writes `image` to `target`, replacing any file there, or under the first free name in the save
/// folder, and returns where it went. `name` follows the name being tried, for the failure
/// notification. Nothing is left behind on failure: the PNG goes to a new temporary file beside
/// where it is going first and is moved into place only once it is all on the disk.
fn write(image: &Image, target: Option<&Path>, name: &mut String) -> Result<PathBuf> {
    let png = image.png().map_err(|_| Error::from(E_FAIL))?;
    let folder = match target.and_then(Path::parent) {
        Some(parent) => parent.to_owned(),
        None => folder()?,
    };
    // Only this thread saves, and only one Zilum runs, so a file of this name already there, in
    // the save folder or one chosen with Save As, is the remains of a save that never finished.
    let temp = folder.join("Zilum saving.tmp");
    let mut out = File::create_new(&temp).or_else(|error| {
        if error.kind() != ErrorKind::AlreadyExists {
            return Err(error);
        }
        fs::remove_file(&temp)?;
        File::create_new(&temp)
    })?;
    let written = out.write_all(png).and_then(|()| out.sync_all());
    drop(out);
    let placed = written
        .map_err(Error::from)
        .and_then(|()| place(&temp, &folder, &image.taken, target, name));
    if placed.is_err() {
        let _ = fs::remove_file(&temp);
    }
    placed
}

/// Moves `temp` to `target`, replacing any file there, or else to the first name in `folder` no
/// other file has, keeping `name` at the name being tried.
fn place(
    temp: &Path,
    folder: &Path,
    taken: &SYSTEMTIME,
    target: Option<&Path>,
    name: &mut String,
) -> Result<PathBuf> {
    if let Some(target) = target {
        move_file(temp, target, MOVEFILE_REPLACE_EXISTING)?;
        return Ok(target.to_owned());
    }
    // Another capture from the same second may have its name, which the move finds out without
    // a gap in which a third could take it.
    for candidate in names(taken) {
        let path = folder.join(&candidate);
        *name = candidate;
        match move_file(temp, &path, MOVE_FILE_FLAGS(0)) {
            Err(error)
                if error.code() == ERROR_ALREADY_EXISTS.to_hresult()
                    || error.code() == ERROR_FILE_EXISTS.to_hresult() => {}
            moved => return moved.map(|()| path),
        }
    }
    unreachable!("there is always another suffix")
}

/// Moves `from` to `to`, returning only once the move is on the disk.
fn move_file(from: &Path, to: &Path, flags: MOVE_FILE_FLAGS) -> Result<()> {
    unsafe {
        MoveFileExW(
            &HSTRING::from(from),
            &HSTRING::from(to),
            flags | MOVEFILE_WRITE_THROUGH,
        )
    }
}

/// The save folder as the settings say it is now, created if it is missing.
fn folder() -> Result<PathBuf> {
    let folder = settings::current().save_folder;
    // Empty only when Windows could not say where Pictures is. A relative path would save
    // somewhere no one looks.
    if folder.as_os_str().is_empty() {
        return Err(E_FAIL.into());
    }
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

/// The names a capture taken at `taken`, local time, may be saved under, in the order to try
/// them: `Zilum 2026-10-16 21.07.33.png`, then `-1`, `-2` and so on.
fn names(taken: &SYSTEMTIME) -> impl Iterator<Item = String> {
    let stem = format!(
        "Zilum {:04}-{:02}-{:02} {:02}.{:02}.{:02}",
        taken.wYear, taken.wMonth, taken.wDay, taken.wHour, taken.wMinute, taken.wSecond
    );
    std::iter::once(format!("{stem}.png")).chain((1..).map(move |n| format!("{stem}-{n}.png")))
}

/// Asks where to save a capture taken at `taken` with the standard save dialog, owned by the
/// editor `owner`, starting at `file` if the capture has been saved, or else in the save folder
/// under the capture's name. Posts `WM_FILE_CHOSEN` to `owner` once the dialog closes. A dialog
/// that fails to open is taken as cancelled.
pub fn choose_file(taken: SYSTEMTIME, file: Option<PathBuf>, owner: HWND) {
    let owner = owner.0 as isize;
    in_shell(move || {
        let owner = HWND(owner as _);
        let chosen = ask(&taken, file.as_deref(), owner).ok();
        post_path(owner, editor::WM_FILE_CHOSEN, chosen);
    });
}

/// Asks for a folder with the standard folder picker, owned by the Settings window `owner`,
/// starting in `start` if it exists. Posts `WM_FOLDER_CHOSEN` to `owner` once the picker closes.
/// A picker that fails to open is taken as cancelled.
pub fn choose_folder(start: PathBuf, owner: HWND) {
    let owner = owner.0 as isize;
    in_shell(move || {
        let owner = HWND(owner as _);
        let chosen = ask_folder(&start, owner).ok();
        post_path(owner, settings_window::WM_FOLDER_CHOSEN, chosen);
    });
}

/// Posts `msg` to `to` with `path`, boxed, in `lparam`, or 0 when there is none. The window takes
/// the box.
fn post_path(to: HWND, msg: u32, path: Option<PathBuf>) {
    let path = path.map(|path| Box::into_raw(Box::new(path)));
    let posted = unsafe {
        PostMessageW(
            Some(to),
            msg,
            WPARAM(0),
            LPARAM(path.map_or(0, |path| path as isize)),
        )
    };
    // The window has gone, so nothing takes the path.
    if let (Err(_), Some(path)) = (posted, path) {
        drop(unsafe { Box::from_raw(path) });
    }
}

/// Shows the folder picker for [`choose_folder`] and returns the folder chosen. Cancelling is an
/// error.
fn ask_folder(start: &Path, owner: HWND) -> Result<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
        // A folder that does not exist yet has no item, and the picker starts where it likes.
        if let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start), None)
        {
            dialog.SetFolder(&item)?;
        }
        show(&dialog, owner)
    }
}

/// Shows the save dialog for [`choose_file`] and returns the file chosen. Cancelling is an error.
fn ask(taken: &SYSTEMTIME, file: Option<&Path>, owner: HWND) -> Result<PathBuf> {
    let (folder, name) = match file {
        Some(file) => (
            file.parent().map(Path::to_owned),
            file.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        None => {
            // Made first, so the dialog can start in it.
            let folder = folder().ok();
            let name = names(taken)
                .find(|name| {
                    folder
                        .as_ref()
                        .is_none_or(|folder| !folder.join(name).exists())
                })
                .unwrap_or_default();
            (folder, name)
        }
    };
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("PNG image"),
            pszSpec: w!("*.png"),
        }])?;
        dialog.SetDefaultExtension(w!("png"))?;
        // With no test file written first, a folder that cannot be written to fails the save,
        // which says so, rather than the dialog.
        dialog.SetOptions(dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_NOTESTFILECREATE)?;
        if let Some(folder) = folder {
            let start: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(folder.as_path()), None)?;
            dialog.SetFolder(&start)?;
        }
        dialog.SetFileName(&HSTRING::from(name))?;
        show(&dialog, owner)
    }
}

/// Shows `dialog`, owned by `owner`, and returns the file or folder chosen. Cancelling is an
/// error.
fn show(dialog: &IFileDialog, owner: HWND) -> Result<PathBuf> {
    unsafe {
        dialog.Show(Some(owner))?;
        let path = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let chosen = path.to_hstring().to_os_string();
        CoTaskMemFree(Some(path.0 as _));
        Ok(PathBuf::from(chosen))
    }
}

/// Opens the save folder in Explorer, creating it if it is missing.
pub fn open_folder() {
    in_shell(|| {
        if let Ok(folder) = folder() {
            let _ = unsafe {
                ShellExecuteW(
                    None,
                    w!("open"),
                    &HSTRING::from(folder.as_path()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                )
            };
        }
    });
}

/// Runs `f` on a thread of its own, since Explorer and the save dialog can take a while, in the
/// single-threaded apartment the shell's extensions expect.
fn in_shell(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        f();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: year,
            wMonth: month,
            wDay: day,
            wHour: hour,
            wMinute: minute,
            wSecond: second,
            // Milliseconds never reach the name.
            wMilliseconds: 999,
            ..Default::default()
        }
    }

    #[test]
    fn the_notice_names_the_saved_file_alone() {
        assert_eq!(
            notice_text(Path::new(
                r"C:\Users\me\Pictures\Zilum\Zilum 2026-10-23 14.05.09.png"
            )),
            "Saved as Zilum 2026-10-23 14.05.09.png"
        );
        assert_eq!(
            notice_text(Path::new(r"\\server\share\Shots\bug report.png")),
            "Saved as bug report.png"
        );
    }

    #[test]
    fn a_capture_is_named_after_its_local_time() {
        assert_eq!(
            names(&at(2026, 10, 16, 21, 7, 33)).next().unwrap(),
            "Zilum 2026-10-16 21.07.33.png"
        );
        assert_eq!(
            names(&at(2027, 1, 2, 3, 4, 5)).next().unwrap(),
            "Zilum 2027-01-02 03.04.05.png"
        );
    }

    #[test]
    fn a_taken_name_gets_the_next_free_suffix() {
        let taken = at(2026, 10, 16, 21, 7, 33);
        let free = |owned: &[&str]| names(&taken).find(|name| !owned.contains(&name.as_str()));

        assert_eq!(free(&[]).unwrap(), "Zilum 2026-10-16 21.07.33.png");
        assert_eq!(
            free(&["Zilum 2026-10-16 21.07.33.png"]).unwrap(),
            "Zilum 2026-10-16 21.07.33-1.png"
        );
        assert_eq!(
            free(&[
                "Zilum 2026-10-16 21.07.33.png",
                "Zilum 2026-10-16 21.07.33-1.png",
            ])
            .unwrap(),
            "Zilum 2026-10-16 21.07.33-2.png"
        );
        // A gap is filled before the sequence goes on.
        assert_eq!(
            free(&[
                "Zilum 2026-10-16 21.07.33.png",
                "Zilum 2026-10-16 21.07.33-2.png",
            ])
            .unwrap(),
            "Zilum 2026-10-16 21.07.33-1.png"
        );
    }
}
