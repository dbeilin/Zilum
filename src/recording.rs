//! Recording: frames of the box's screen, cropped to the box and with the pointer in them,
//! written into a looping GIF in the save folder and put on the clipboard as a file. Frames are
//! captured on one thread and encoded on another, and neither holds up the message thread: a
//! frame that comes while the encoder is still on the one before is skipped, never queued, so at
//! most a couple are ever held.

use std::{
    borrow::Cow,
    collections::HashMap,
    fs::{self, File},
    io::{self, BufWriter, ErrorKind},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use color_quant::NeuQuant;
use windows::Win32::{
    Foundation::{HWND, LPARAM, SYSTEMTIME, WPARAM},
    Graphics::Gdi::HMONITOR,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use crate::{
    capture::Stream,
    clipboard, graphics,
    image::Region,
    pixels,
    save::{self, GIF},
    selection::Selection,
};

/// Posted to the window a recording was started for once it has ended and its GIF is written,
/// or could not be. `lparam` is a boxed [`Outcome`], which [`outcome`] takes.
pub const WM_RECORDED: u32 = WM_APP;

/// Frames are taken at most this often: 20 a second, five of the GIF's hundredths each.
const TICK: Duration = Duration::from_millis(50);
/// A frame is never shown for less than this many hundredths: browsers show a shorter delay as a
/// tenth of a second.
const MIN_DELAY: u32 = 2;
/// A delay longer than a GIF frame holds is split into pieces this long, leaving a last piece
/// over 5000.
const LONGEST_PIECE: u16 = 60_000;

/// Which frames go into the GIF and how long each is shown, in the GIF's hundredths of a second.
/// A frame's place is when it arrived, rounded to the hundredth, and the first one's is the click
/// on Record, so the delays add up to the recording's length however each one rounds.
#[derive(Default)]
struct Timing {
    /// The hundredth the last frame written starts at.
    last: Option<u32>,
}

/// What becomes of a frame that arrived.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Not written: the frame before it is shown for longer instead.
    Folded,
    /// Written, once the frame before it, if any, is given `delay` hundredths.
    Written { delay: Option<u32> },
}

impl Timing {
    /// The frame arriving `at` after the click on Record, `changed` if it differs from the last
    /// one written and `busy` if the encoder was still on the one before.
    fn arrive(&mut self, at: Duration, changed: bool, busy: bool) -> Decision {
        let Some(last) = self.last else {
            self.last = Some(0);
            return Decision::Written { delay: None };
        };
        let now = hundredths(at);
        if !changed || busy || now < last + MIN_DELAY {
            return Decision::Folded;
        }
        self.last = Some(now);
        Decision::Written {
            delay: Some(now - last),
        }
    }

    /// The delay of the last frame written when the recording stops `at`, if any was.
    fn end(&self, at: Duration) -> Option<u32> {
        self.last
            .map(|last| hundredths(at).saturating_sub(last).max(MIN_DELAY))
    }
}

/// A frame shown for `delay` hundredths, as the delays of the frame and of the repeats of it that
/// follow: a GIF's delay is at most 65535, under 11 minutes.
fn pieces(mut delay: u32) -> Vec<u16> {
    let mut pieces = Vec::new();
    while delay > u16::MAX as u32 {
        pieces.push(LONGEST_PIECE);
        delay -= LONGEST_PIECE as u32;
    }
    pieces.push(delay as u16);
    pieces
}

/// `at`, rounded to the nearest hundredth of a second.
fn hundredths(at: Duration) -> u32 {
    ((at.as_millis() + 5) / 10).min(u32::MAX as u128) as u32
}

/// How a recording ended.
pub enum Outcome {
    /// No frame had come, so nothing was saved.
    Nothing,
    /// The GIF is at `file`, and on the clipboard too if `copied`.
    Saved { file: PathBuf, copied: bool },
    /// The GIF, which was to be called `name`, could not be written, for the reason `why` gives.
    Failed { name: String, why: Why },
}

/// Why a GIF could not be written, as far as the user can do something about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The save folder refused the file.
    Unwritable,
    /// The disk had no room for it.
    Full,
    /// Anything else.
    Other,
}

/// Why writing failed with `error`.
fn why(error: &io::Error) -> Why {
    match error.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => Why::Unwritable,
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => Why::Full,
        _ => Why::Other,
    }
}

/// `error` as an I/O error, with its kind when it carries a Win32 error code.
fn os_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::other(error)
    }
}

/// `error` as an I/O error, the one it wraps if it wraps one.
fn gif_error(error: gif::EncodingError) -> io::Error {
    match error {
        gif::EncodingError::Io(error) => error,
        error => io::Error::other(error),
    }
}

/// A recording, from its start until its GIF is written. Dropping it stops the recording, as
/// [`Recorder::stop`] does.
pub struct Recorder {
    stop: Sender<Instant>,
    /// Records, then writes and copies the GIF, then posts how it went.
    thread: JoinHandle<()>,
}

impl Recorder {
    /// Starts recording the box, `area` in physical pixels of `screen`, as from `started`, the
    /// click on Record at local time `taken`, which names the GIF. Posts `WM_RECORDED` to `to` once the
    /// recording has stopped and its GIF is written.
    pub fn start(
        screen: HMONITOR,
        area: Selection,
        started: Instant,
        taken: SYSTEMTIME,
        to: HWND,
    ) -> io::Result<Recorder> {
        let (stop, stopped) = mpsc::channel();
        let (screen, to) = (screen.0 as isize, to.0 as isize);
        let thread = std::thread::Builder::new().spawn(move || {
            graphics::init_apartment();
            let outcome = record(HMONITOR(screen as _), area, started, taken, &stopped);
            post(HWND(to as _), outcome);
        })?;
        Ok(Recorder { stop, thread })
    }

    /// Stops the recording now, if it has not stopped. The GIF is written soon after.
    pub fn stop(&self) {
        let _ = self.stop.send(Instant::now());
    }

    /// Stops the recording now, if it has not stopped, and waits until its GIF is written and
    /// copied, or could not be, for Zilum to leave. The window it was started for is gone by
    /// then, so nothing is told how it went.
    pub fn finish(self) {
        self.stop();
        let _ = self.thread.join();
    }
}

/// What the notification after a recording saved as `file` reads: "Copied and saved as" when the
/// file went on the clipboard too, else "Saved as", as after a Save, and then the folder it is in.
pub fn saved_text(file: &Path, copied: bool) -> String {
    let saved = if copied {
        let name = file.file_name().unwrap_or(file.as_os_str());
        format!("Copied and saved as {}", name.to_string_lossy())
    } else {
        save::notice_text(file)
    };
    match file.parent() {
        Some(folder) => format!("{saved}\nin {}", folder.display()),
        None => saved,
    }
}

/// What the notification after a recording whose GIF, to be called `name`, could not be written
/// reads, saying why when the user can do something about it.
pub fn failed_text(name: &str, why: Why) -> String {
    match why {
        Why::Unwritable => format!("Couldn't save {name}: the save folder can't be written to"),
        Why::Full => format!("Couldn't save {name}: the disk is full"),
        Why::Other => format!("Couldn't save {name}"),
    }
}

/// The outcome posted with `WM_RECORDED`, from its `lparam`.
///
/// # Safety
///
/// `lparam` is a `WM_RECORDED` message's, and is taken only once.
pub unsafe fn outcome(lparam: LPARAM) -> Outcome {
    // SAFETY: `post` made it with Box::into_raw, and the caller takes it once.
    *unsafe { Box::from_raw(lparam.0 as *mut Outcome) }
}

/// Posts `outcome` to `to` with `WM_RECORDED`, boxed. If the window has gone, nothing takes it.
fn post(to: HWND, outcome: Outcome) {
    let outcome = Box::into_raw(Box::new(outcome));
    // SAFETY: Handles and integers only; if the post fails, the box in lparam is reclaimed below.
    let posted =
        unsafe { PostMessageW(Some(to), WM_RECORDED, WPARAM(0), LPARAM(outcome as isize)) };
    if posted.is_err() {
        // SAFETY: The post failed, so nothing took `outcome`, which Box::into_raw made above.
        drop(unsafe { Box::from_raw(outcome) });
    }
}

/// What the capture hands the encoder.
enum Job {
    /// A frame to write once the frame before it, if any, is given `delay` hundredths: `part` of
    /// the box, its `pixels` blue, green, red and alpha in top-down rows.
    Frame {
        part: Region,
        pixels: Vec<u8>,
        delay: Option<u32>,
    },
    /// The recording has stopped, and the last frame, if any, is given `delay`.
    End { delay: Option<u32> },
}

/// Records until a stop comes from `stopped`, or until the screen cannot be captured, then
/// writes what was recorded.
fn record(
    screen: HMONITOR,
    area: Selection,
    started: Instant,
    taken: SYSTEMTIME,
    stopped: &Receiver<Instant>,
) -> Outcome {
    let (Ok(width), Ok(height)) = (u16::try_from(area.width), u16::try_from(area.height)) else {
        return Outcome::Nothing;
    };
    let busy = Arc::new(AtomicBool::new(false));
    let (jobs, queue) = mpsc::channel();
    let encoding = {
        let busy = busy.clone();
        std::thread::Builder::new().spawn(move || {
            graphics::init_apartment();
            encode(queue, &busy, width, height, taken)
        })
    };
    let Ok(encoding) = encoding else {
        return Outcome::Nothing;
    };
    let delay = capture(screen, area, started, stopped, &jobs, &busy);
    let _ = jobs.send(Job::End { delay });
    drop(jobs);
    encoding.join().unwrap_or(Outcome::Nothing)
}

/// Takes a frame of `area` of `screen` each tick from `started` until a stop comes from
/// `stopped`, or until the screen cannot be captured, and hands the encoder each one the timing
/// writes. Returns the last frame's delay, if any frame was written.
fn capture(
    screen: HMONITOR,
    area: Selection,
    started: Instant,
    stopped: &Receiver<Instant>,
    jobs: &Sender<Job>,
    busy: &AtomicBool,
) -> Option<u32> {
    let Ok(mut stream) = Stream::start(screen, TICK) else {
        return None;
    };
    let mut timing = Timing::default();
    // The box's pixels as last written, which the next frame is compared with.
    let mut last: Option<Vec<u8>> = None;
    // Whether the screen may have changed since the box's pixels were last compared.
    let mut fresh = false;
    // The tick after this one, skipping any already past by the time this one is done.
    let next = || (started.elapsed().as_nanos() / TICK.as_nanos()) as u32 + 1;
    let mut tick: u32 = 0;
    let stop = loop {
        let due = started + TICK * tick;
        match stopped.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(at) => break at,
            Err(RecvTimeoutError::Disconnected) => break Instant::now(),
            Err(RecvTimeoutError::Timeout) => {}
        }
        match stream.refresh() {
            Ok(came) => fresh |= came,
            Err(_) => break Instant::now(),
        }
        let frame = match stream.frame() {
            Ok(Some(frame)) => frame,
            // Nothing to record before the first frame comes.
            Ok(None) => {
                tick = next();
                continue;
            }
            Err(_) => break Instant::now(),
        };
        let skipped = busy.load(Ordering::Acquire);
        // The box's pixels, if they changed since the last frame written, and the part that did.
        let mut changed = None;
        if fresh && !skipped {
            let read = pixels::read(&frame, screen, SYSTEMTIME::default(), area, false);
            // A screen whose size changed no longer holds the box.
            let Some(image) = read.ok().filter(|image| {
                image.width == area.width as u32 && image.height == area.height as u32
            }) else {
                break Instant::now();
            };
            fresh = false;
            let part = match &last {
                Some(last) => difference(last, &image.pixels, image.width),
                None => Some(Region {
                    left: 0,
                    top: 0,
                    right: image.width,
                    bottom: image.height,
                }),
            };
            changed = part.map(|part| (part, image.pixels));
        }
        match (
            timing.arrive(TICK * tick, changed.is_some(), skipped),
            changed,
        ) {
            (Decision::Written { delay }, Some((part, pixels))) => {
                busy.store(true, Ordering::Release);
                let job = Job::Frame {
                    part,
                    pixels: crop(&pixels, area.width as u32, part),
                    delay,
                };
                if jobs.send(job).is_err() {
                    break Instant::now();
                }
                last = Some(pixels);
            }
            // Too soon after the last frame written: compared again at the next tick.
            (Decision::Folded, Some(_)) => fresh = true,
            _ => {}
        }
        tick = next();
    };
    timing.end(stop.saturating_duration_since(started))
}

/// The smallest part of two frames `width` pixels wide, `last` and `now`, that holds every pixel
/// that differs between them, if any does.
fn difference(last: &[u8], now: &[u8], width: u32) -> Option<Region> {
    let row = width as usize * 4;
    let mut part: Option<Region> = None;
    for (y, (was, is)) in last
        .chunks_exact(row)
        .zip(now.chunks_exact(row))
        .enumerate()
    {
        if was == is {
            continue;
        }
        let differs = |x: &usize| was[x * 4..x * 4 + 4] != is[x * 4..x * 4 + 4];
        let (Some(left), Some(right)) = (
            (0..width as usize).find(differs),
            (0..width as usize).rev().find(differs),
        ) else {
            continue;
        };
        let (y, left, right) = (y as u32, left as u32, right as u32 + 1);
        part = Some(match part {
            None => Region {
                left,
                top: y,
                right,
                bottom: y + 1,
            },
            Some(part) => Region {
                left: part.left.min(left),
                right: part.right.max(right),
                bottom: y + 1,
                ..part
            },
        });
    }
    part
}

/// `part` of `pixels`, a frame `width` pixels wide.
fn crop(pixels: &[u8], width: u32, part: Region) -> Vec<u8> {
    let row = width as usize * 4;
    let (left, across) = (part.left as usize * 4, part.width() as usize * 4);
    let mut cropped = Vec::with_capacity(across * part.height() as usize);
    for y in part.top as usize..part.bottom as usize {
        let start = y * row + left;
        cropped.extend_from_slice(&pixels[start..start + across]);
    }
    cropped
}

/// Writes each frame from `jobs` into a GIF `width` by `height` until the end comes, clearing
/// `busy` after each, then puts the GIF in the save folder under a name from `taken`, and the
/// file on the clipboard.
fn encode(
    jobs: Receiver<Job>,
    busy: &AtomicBool,
    width: u16,
    height: u16,
    taken: SYSTEMTIME,
) -> Outcome {
    let mut name = save::names(&taken, GIF).next().unwrap_or_default();
    // Begun with the first frame, so a recording with none leaves nothing behind. Once writing
    // has failed, the frames that follow are let go.
    let mut gif: Option<io::Result<Gif>> = None;
    let mut last = None;
    for job in jobs {
        match job {
            Job::Frame {
                part,
                pixels,
                delay,
            } => {
                let writing = gif.get_or_insert_with(|| Gif::create(width, height));
                if let Ok(writing) = writing
                    && let Err(error) = writing.add(part, &pixels, delay)
                {
                    gif = Some(Err(error));
                }
                busy.store(false, Ordering::Release);
            }
            Job::End { delay } => {
                last = delay;
                break;
            }
        }
    }
    match gif {
        None => Outcome::Nothing,
        Some(Err(error)) => Outcome::Failed {
            name,
            why: why(&error),
        },
        Some(Ok(writing)) => match writing.finish(last, &taken, &mut name) {
            Ok(file) => {
                // Only once the GIF is in its place, so nothing is copied for a failed save.
                let copied = clipboard::put_file(&file).is_ok();
                Outcome::Saved { file, copied }
            }
            Err(error) => Outcome::Failed {
                name,
                why: why(&error),
            },
        },
    }
}

/// A GIF being written into a temporary file in the save folder.
struct Gif {
    /// Taken when the GIF is finished.
    encoder: Option<gif::Encoder<BufWriter<File>>>,
    temp: PathBuf,
    folder: PathBuf,
    /// The last frame, compressed, waiting to be given its delay.
    pending: Option<Pending>,
    /// Whether the GIF reached its place in the save folder, so `temp` is no longer there.
    placed: bool,
}

/// A frame waiting for its delay, which comes with the next frame or the end.
struct Pending {
    frame: gif::Frame<'static>,
    /// The colour of its top-left pixel, which a repeat of it shows.
    corner: [u8; 3],
}

impl Gif {
    /// A GIF `width` by `height` that loops forever, begun in the save folder, which is made if
    /// it is missing.
    fn create(width: u16, height: u16) -> io::Result<Gif> {
        let folder = save::folder().map_err(os_error)?;
        // Only one recording runs at a time, so a file of this name already there is the remains
        // of one that never finished.
        let temp = folder.join("Zilum recording.tmp");
        let file = File::create_new(&temp).or_else(|error| {
            if error.kind() != ErrorKind::AlreadyExists {
                return Err(error);
            }
            fs::remove_file(&temp)?;
            File::create_new(&temp)
        })?;
        let begun =
            gif::Encoder::new(BufWriter::new(file), width, height, &[]).and_then(|mut encoder| {
                encoder.set_repeat(gif::Repeat::Infinite)?;
                Ok(encoder)
            });
        match begun {
            Ok(encoder) => Ok(Gif {
                encoder: Some(encoder),
                temp,
                folder,
                pending: None,
                placed: false,
            }),
            Err(error) => {
                let _ = fs::remove_file(&temp);
                Err(gif_error(error))
            }
        }
    }

    /// Writes the frame waiting, if `delay` is given, and makes `pixels`, `part` of the box, the
    /// frame waiting.
    fn add(&mut self, part: Region, pixels: &[u8], delay: Option<u32>) -> io::Result<()> {
        if let Some(delay) = delay {
            self.write_pending(delay)?;
        }
        self.pending = Some(quantised(part, pixels));
        Ok(())
    }

    /// Writes the frame waiting, if there is one, shown for `delay` hundredths.
    fn write_pending(&mut self, delay: u32) -> io::Result<()> {
        let (Some(mut pending), Some(encoder)) = (self.pending.take(), self.encoder.as_mut())
        else {
            return Ok(());
        };
        let mut pieces = pieces(delay).into_iter();
        pending.frame.delay = pieces.next().unwrap_or(MIN_DELAY as u16);
        encoder
            .write_lzw_pre_encoded_frame(&pending.frame)
            .map_err(gif_error)?;
        for delay in pieces {
            // One pixel of the frame, as it is, carrying the rest of its delay.
            let repeat = gif::Frame {
                delay,
                left: pending.frame.left,
                top: pending.frame.top,
                width: 1,
                height: 1,
                palette: Some(pending.corner.to_vec()),
                buffer: Cow::Borrowed(&[0]),
                ..Default::default()
            };
            encoder.write_frame(&repeat).map_err(gif_error)?;
        }
        Ok(())
    }

    /// Writes the last frame, shown for `delay` hundredths, and the end of the GIF, and moves it
    /// into the save folder under the first free name from `taken`, which `name` follows. Nothing
    /// is left behind on failure.
    fn finish(
        mut self,
        delay: Option<u32>,
        taken: &SYSTEMTIME,
        name: &mut String,
    ) -> io::Result<PathBuf> {
        if let Some(delay) = delay {
            self.write_pending(delay)?;
        }
        let encoder = self.encoder.take().ok_or(ErrorKind::Other)?;
        let file = encoder
            .into_inner()
            .map_err(gif_error)?
            .into_inner()
            .map_err(|error| error.into_error())?;
        let synced = file.sync_all();
        drop(file);
        synced?;
        let path = save::place_new(&self.temp, &self.folder, taken, GIF, name).map_err(os_error)?;
        self.placed = true;
        Ok(path)
    }
}

impl Drop for Gif {
    fn drop(&mut self) {
        // A GIF given up on, or that failed on the way to the save folder: its file closed, then
        // not left behind.
        drop(self.encoder.take());
        if !self.placed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

/// `pixels`, `part` of the box, as a frame there with a palette of its own, compressed.
fn quantised(part: Region, pixels: &[u8]) -> Pending {
    let (palette, indices) = exact(pixels).unwrap_or_else(|| approximated(pixels));
    let first = indices.first().map_or(0, |&index| index as usize * 3);
    let mut corner = [0; 3];
    corner.copy_from_slice(&palette[first..first + 3]);
    let mut frame = gif::Frame {
        left: part.left as u16,
        top: part.top as u16,
        width: part.width() as u16,
        height: part.height() as u16,
        palette: Some(palette),
        buffer: Cow::Owned(indices),
        ..Default::default()
    };
    frame.make_lzw_pre_encoded();
    Pending { frame, corner }
}

/// The palette, red, green and blue, and each pixel's index into it, for `pixels` of 256 colours
/// or fewer, which then keep their colours exactly. None for more colours.
fn exact(pixels: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut indices = Vec::with_capacity(pixels.len() / 4);
    let mut palette = Vec::new();
    let mut seen: HashMap<[u8; 3], u8> = HashMap::new();
    // Runs of one colour are the rule on a screen, so the last pixel's index is kept at hand.
    let mut previous: Option<([u8; 3], u8)> = None;
    for pixel in pixels.as_chunks::<4>().0 {
        let colour = [pixel[2], pixel[1], pixel[0]];
        let index = match previous {
            Some((last, index)) if last == colour => index,
            _ => match seen.get(&colour) {
                Some(&index) => index,
                None => {
                    let index = u8::try_from(seen.len()).ok()?;
                    seen.insert(colour, index);
                    palette.extend_from_slice(&colour);
                    index
                }
            },
        };
        previous = Some((colour, index));
        indices.push(index);
    }
    Some((palette, indices))
}

/// A palette of 256 colours chosen from `pixels`, red, green and blue, and each pixel's index
/// into it.
fn approximated(pixels: &[u8]) -> (Vec<u8>, Vec<u8>) {
    // Learning from every tenth pixel, or fewer on a large frame, so a frame of a 4K screen is
    // learnt from in about the time one of a megapixel takes.
    let sample = (pixels.len() / 4 / 100_000).clamp(10, 30) as i32;
    let quantiser = NeuQuant::new(sample, 256, pixels);
    // The quantiser keeps the pixels' order, blue, green and red.
    let palette: Vec<u8> = quantiser
        .color_map_rgb()
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|&[blue, green, red]| [red, green, blue])
        .collect();
    // The quantiser's own lookup can pick a colour far from the nearest, which wipes out thin
    // lines, so each pixel takes the nearest of the whole palette. It is found once for each cell
    // of 64 levels a channel, from the cell's middle, so a frame of any number of colours costs
    // at most that many searches.
    let mut nearest = vec![u16::MAX; 1 << 18];
    let indices = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[blue, green, red, _]| {
            let cell = (red as usize >> 2) << 12 | (green as usize >> 2) << 6 | blue as usize >> 2;
            if nearest[cell] == u16::MAX {
                let middle = [red, green, blue].map(|level| level & !3 | 2);
                nearest[cell] = closest(&palette, middle) as u16;
            }
            nearest[cell] as u8
        })
        .collect();
    (palette, indices)
}

/// The index of the colour in `palette`, red, green and blue, nearest `colour`.
fn closest(palette: &[u8], colour: [u8; 3]) -> usize {
    let distance = |entry: &[u8; 3]| -> u32 {
        (0..3)
            .map(|channel| (entry[channel] as i32 - colour[channel] as i32).pow(2) as u32)
            .sum()
    };
    palette
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .min_by_key(|(_, entry)| distance(entry))
        .map_or(0, |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::E_FAIL;
    use windows::core::HRESULT;

    use super::*;

    /// A frame arriving `ms` milliseconds in, changed or not and while the encoder was busy or not.
    struct Arrival(u64, bool, bool);

    /// The frames written, by their place in `arrivals`, and each one's delay, for a recording
    /// stopped `stop` milliseconds in.
    fn written(arrivals: &[Arrival], stop: u64) -> Vec<(usize, u32)> {
        let mut timing = Timing::default();
        let mut written: Vec<(usize, u32)> = Vec::new();
        for (index, &Arrival(ms, changed, busy)) in arrivals.iter().enumerate() {
            if let Decision::Written { delay } =
                timing.arrive(Duration::from_millis(ms), changed, busy)
            {
                if let (Some(delay), Some(last)) = (delay, written.last_mut()) {
                    last.1 = delay;
                }
                written.push((index, 0));
            }
        }
        if let (Some(delay), Some(last)) =
            (timing.end(Duration::from_millis(stop)), written.last_mut())
        {
            last.1 = delay;
        }
        written
    }

    #[test]
    fn the_first_frame_is_always_written_and_shown_from_the_click() {
        assert_eq!(written(&[Arrival(30, false, false)], 500), [(0, 50)]);
        assert_eq!(written(&[Arrival(30, true, true)], 500), [(0, 50)]);
    }

    #[test]
    fn changed_frames_are_written_each_shown_until_the_next() {
        let arrivals = [
            Arrival(50, true, false),
            Arrival(100, true, false),
            Arrival(150, true, false),
        ];
        assert_eq!(written(&arrivals, 300), [(0, 10), (1, 5), (2, 15)]);
    }

    #[test]
    fn unchanged_and_skipped_frames_fold_into_the_one_before() {
        let arrivals = [
            Arrival(50, true, false),
            Arrival(100, false, false),
            Arrival(150, true, true),
            Arrival(200, true, false),
            Arrival(250, false, true),
        ];
        assert_eq!(written(&arrivals, 1000), [(0, 20), (3, 80)]);
    }

    #[test]
    fn the_delays_add_up_to_the_length_with_the_rounding_carried() {
        // 30 a second, which no whole number of hundredths is.
        let arrivals: Vec<_> = (0..300)
            .map(|n| Arrival(n * 1000 / 30, true, false))
            .collect();
        let frames = written(&arrivals, 10_000);
        assert_eq!(frames.len(), 300);
        assert_eq!(frames.iter().map(|(_, delay)| delay).sum::<u32>(), 1000);
        assert!(frames.iter().all(|(_, delay)| (3..=4).contains(delay)));
        // Rounding to the nearest hundredth, not down.
        assert_eq!(written(&[Arrival(0, true, false)], 4_996), [(0, 500)]);
    }

    #[test]
    fn a_still_recording_is_one_frame_shown_for_all_of_it() {
        let mut arrivals = vec![Arrival(50, true, false)];
        arrivals.extend((2..200).map(|n| Arrival(n * 50, false, false)));
        assert_eq!(written(&arrivals, 10_000), [(0, 1000)]);
    }

    #[test]
    fn no_frame_is_shown_for_less_than_two_hundredths() {
        // A change straight after one written waits for the next.
        let arrivals = [
            Arrival(50, true, false),
            Arrival(100, true, false),
            Arrival(110, true, false),
            Arrival(150, true, false),
        ];
        assert_eq!(written(&arrivals, 300), [(0, 10), (1, 5), (3, 15)]);
        // Stopping just after a frame still shows it for two hundredths.
        assert_eq!(
            written(&[Arrival(50, true, false), Arrival(100, true, false)], 105),
            [(0, 10), (1, 2)]
        );
    }

    #[test]
    fn a_delay_too_long_for_one_frame_is_split_across_repeats() {
        assert_eq!(pieces(500), [500]);
        assert_eq!(pieces(65_535), [65_535]);
        // Twenty minutes of a still screen.
        assert_eq!(pieces(120_000), [60_000, 60_000]);
        // No piece left too short to be shown as it is.
        assert_eq!(pieces(65_536), [60_000, 5_536]);
        assert_eq!(pieces(125_536), [60_000, 60_000, 5_536]);
    }

    #[test]
    fn a_recording_with_no_frame_writes_nothing() {
        assert_eq!(written(&[], 5000), []);
    }

    #[test]
    fn the_saved_notification_says_whether_the_gif_was_copied_too_and_where_it_is() {
        let file = Path::new(r"C:\Users\me\Pictures\Zilum\Zilum 2026-10-31 14.05.09.gif");
        assert_eq!(
            saved_text(file, true),
            "Copied and saved as Zilum 2026-10-31 14.05.09.gif\nin C:\\Users\\me\\Pictures\\Zilum"
        );
        assert_eq!(
            saved_text(file, false),
            "Saved as Zilum 2026-10-31 14.05.09.gif\nin C:\\Users\\me\\Pictures\\Zilum"
        );
        assert_eq!(
            saved_text(
                Path::new(r"\\server\share\Clips\Zilum 2026-10-31 14.05.09-1.gif"),
                true
            ),
            "Copied and saved as Zilum 2026-10-31 14.05.09-1.gif\nin \\\\server\\share\\Clips"
        );
    }

    #[test]
    fn a_failed_notification_says_why_when_the_user_can_act_on_it() {
        let name = "Zilum 2026-10-31 14.05.09.gif";
        assert_eq!(
            failed_text(name, Why::Unwritable),
            "Couldn't save Zilum 2026-10-31 14.05.09.gif: the save folder can't be written to"
        );
        assert_eq!(
            failed_text(name, Why::Full),
            "Couldn't save Zilum 2026-10-31 14.05.09.gif: the disk is full"
        );
        assert_eq!(
            failed_text(name, Why::Other),
            "Couldn't save Zilum 2026-10-31 14.05.09.gif"
        );
    }

    #[test]
    fn a_refused_folder_reads_as_unwritable_and_no_room_as_full() {
        // ERROR_ACCESS_DENIED, ERROR_WRITE_PROTECT, ERROR_DISK_FULL, ERROR_HANDLE_DISK_FULL and
        // ERROR_PATH_NOT_FOUND, as the file calls give them.
        let raw = |code| why(&io::Error::from_raw_os_error(code));
        assert_eq!(raw(5), Why::Unwritable);
        assert_eq!(raw(112), Why::Full);
        assert_eq!(raw(39), Why::Full);
        assert_eq!(raw(3), Why::Other);
        // As the save folder and the move into it give them: HRESULTs of Win32 codes.
        let hresult = |code: u32| why(&os_error(HRESULT((0x8007_0000 | code) as i32).into()));
        assert_eq!(hresult(5), Why::Unwritable);
        assert_eq!(hresult(112), Why::Full);
        assert_eq!(why(&os_error(E_FAIL.into())), Why::Other);
        // As the GIF encoder gives them.
        let full = gif::EncodingError::Io(io::Error::from_raw_os_error(112));
        assert_eq!(why(&gif_error(full)), Why::Full);
    }
}
