# Hotkey-to-overlay latency: Zilum and the Windows Snipping Tool

Measured 2026-10-23. The question: how long after the hotkey does each app's capture overlay appear on screen? The website's benchmark shows the two warm medians from this file.

**Result: a wide gap.** Zilum's median is about 4x faster than the Snipping Tool's warm, 6x faster after a minute idle, and 7x faster from a stopped process. ShareX is not installed on this machine, so it was not measured.

## Numbers

All times are in milliseconds, from the hotkey's injection to the first screen read that showed the overlay. p95 is nearest rank, so with n = 5 it is the maximum.

| App | Runs | n | Median | p95 | Min | Max | Window shown, median |
|---|---|--:|--:|--:|--:|--:|--:|
| Zilum | warm | 30 | 50 | 58 | 46 | 61 | 44 |
| Zilum | cold (60 s idle) | 5 | 70 | 318 | 53 | 318 | 65 |
| Zilum | launch | 5 | 64 | 98 | 64 | 98 | 60 |
| Snipping Tool | warm | 30 | 198 | 216 | 178 | 226 | 179 |
| Snipping Tool | cold (60 s idle) | 5 | 452 | 1744 | 199 | 1744 | 426 |
| Snipping Tool | launch | 5 | 437 | 547 | 415 | 547 | 398 |

Ratios of medians, Snipping Tool divided by Zilum: warm 4.0x, cold 6.5x, launch 6.8x. Zilum's slowest warm run (61 ms) beats the Snipping Tool's fastest warm run (178 ms) by 2.9x. All 80 runs saw an overlay, and every overlay closed with Esc.

"Window shown" is when the WinEvent hook saw the app's full-screen window become visible. Pixels follow about 6 ms later for Zilum and about 20 ms later for the Snipping Tool (about 40 ms on launch). The window event alone would understate the Snipping Tool's time.

## Machine

- CPU: AMD Ryzen 7 5800X3D, 8 cores and 16 threads. RAM: 16 GB.
- GPU: NVIDIA GeForce RTX 2070 SUPER, driver 32.0.16.1088.
- Two screens, both 2560 x 1440:
  - primary: Dell AW2725DF at 360 Hz, HDR on; every measurement was read here
  - secondary: Dell S2719DGF at 59 Hz, to the left
- Windows 11 Pro 26H2, build 26300.
- Zilum: a release build from the pre-release history, built minutes before the run. Zilum 0.1.0 came after it; the changes since are in the editor and in window capture, not in the path from the hotkey to the overlay. 0.1.0 itself was not re-measured.
- Snipping Tool: `Microsoft.ScreenSketch` 11.2608.41.0.
- ShareX: not installed (checked the uninstall keys, Program Files, Steam and AppData). Nothing was installed for this run.

## Method

The driver is [`measure-latency.ps1`](measure-latency.ps1): PowerShell 7 with inline C#, run unsandboxed on the real desktop. [`summarize.ps1`](summarize.ps1) turns `runs.csv` into the table above. Every app is measured the same way:

1. **Surfaces.** The driver shows two topmost windows of its own on the primary screen: a white 240 px *sample* square and a grey *pad*.
   - Before each press, the pointer moves onto the pad and the pad takes the front. Every run starts from the same state.
   - Zilum lights the window under the pointer, so that window is the pad. The sample stays dimmed under every overlay.
2. **Press.** The hotkey's key-downs go in as one `SendInput` batch. A `QueryPerformanceCounter` timestamp is taken right before that call. The key-ups follow 30 ms later from another thread.
3. **Pixels.** The driver reads the sample's middle 16 x 16 pixels off the screen through GDI (`BitBlt` with `CAPTUREBLT`) in a tight loop and stamps each read with QPC.
   - The reads come in step with screen frames: a median of 2.8 ms apart, one frame at 360 Hz.
   - The overlay counts as arrived at the first read whose mean differs from the pre-press read by more than 12 levels. Zilum dims white to 225 and the Snipping Tool to 102.
   - In every run the change arrived whole in one frame; neither app fades in. "Settled" equals "first" in every row of `runs.csv`.
4. **Window event.** An out-of-context WinEvent hook for `EVENT_OBJECT_SHOW` stamps every top-level window shown. The first window of the app's own process that covers at least half the primary screen is the "window shown" time.
5. **Close.** Esc goes in only while the app's own process has the front, which was true in all 80 runs. The next run waits until the sample reads white again.

**Hotkeys.**
- Zilum: `Ctrl+Shift+F9`, written into `settings.ini` for the run only.
- Snipping Tool: `Win+Shift+S`, which can't be changed.
- PrintScreen was not used, so the Snipping Tool's PrintScreen setting played no part and was not touched.

**Run kinds**, per app:
- **launch, 5 runs.** The process was stopped first.
  - The Snipping Tool was started by Windows from the hotkey itself.
  - Zilum was started and given 3 s, because it has to be running to own a hotkey at all.
- **cold, 5 runs.** Each came after 60 s with no press to any app, the process resident. The apps took turns, so each one's cold run had a full minute of quiet.
- **warm, 30 runs.** Back to back, 1 s after the previous overlay closed.

**Order.** The launch runs alternated between the apps, then the cold runs, then all 30 warm runs for Zilum, then all 30 for the Snipping Tool.

## Raw data

- `runs.csv` has one row per run: app, kind, whether the process existed before the press, timestamp, baseline and overlay levels, first-pixel, settled and window-shown ms, read count, median read gap, the process in front afterwards, and close time.
- `samples.csv` has every screen read: run, ms since the press, mean level.
- `events.csv` has every top-level window shown after each press: run, ms, process, class, rect.
- An earlier attempt was discarded and is not included here. Partway through it, a separate test on the same desktop stopped the measured Zilum and briefly ran a different Zilum build. Its 13 runs before the break agree with the clean run, but none of them are in the table.

## Fairness caveats

- **The Snipping Tool is resident after first use.** It was not running when the measurement began, so Windows did not pre-warm it on this machine. After the first `Win+Shift+S` its two processes stayed resident until the driver stopped them. On other machines it may already be running at sign-in.
  - The cold rows had it resident, which is the common case. The launch rows stopped it first, which a user rarely meets.
  - The headline is the warm and cold rows. The launch row only shows what a stopped Snipping Tool costs.
- **Zilum's launch row is not like the Snipping Tool's.** Zilum was given 3 s after starting before the press. A stopped Zilum does not answer its hotkey at all; that is the cost of a tray app. The row is not "Zilum launches 7x faster".
- **Different hotkey routes.** Zilum's hotkey is a `RegisterHotKey` in its own process. `Win+Shift+S` is handled by the shell and handed to the Snipping Tool, which may add a hop. That hop is part of how users reach the Snipping Tool, so it counts.
- **The two overlays do different amounts of work.**
  - Zilum freezes every screen with Windows Graphics Capture and dims them.
  - The Snipping Tool freezes and dims too, and adds its mode toolbar, in rectangle mode, its default.
  - Neither was configured beyond the hotkey.
- **One outlier each in the cold runs.**
  - Zilum run 17: 318 ms, against 53 to 72 in the other four.
  - Snipping Tool run 12: 1,744 ms, its first cold run.
  - Nothing else was known to touch the desktop during the clean run, and nothing in `events.csv` explains either one. Treat cold p95 with n = 5 as anecdotal.
- **Defender and a fresh exe.** Zilum was built minutes before the run and had launched once in a pilot run, so Defender had already scanned it. The first launch row (98 ms) is the slowest launch. A first launch right after download, scanned in full, was not measured. The Snipping Tool is a signed Store app.
- **Hotkey conflicts.** Zilum's own default is PrintScreen, which the Snipping Tool can claim (`PrintScreenKeyForSnippingEnabled` is absent here, so the Windows default applies). Giving Zilum `Ctrl+Shift+F9` kept the two apart; neither saw the other's key.
- **Injected input.** `SendInput` keys carry the injected flag. Neither app is known to treat injected keys differently, and the same path was used for both.
- **What the numbers include.**
  - They include the time from the screen's composition to GDI's read. They exclude scan-out, panel response and any USB keyboard delay. Those add the same for every app.
  - On a 60 Hz screen each number grows by up to about one frame (17 ms) for every app alike. The 360 Hz screen lets small differences show.
- **HDR primary.** GDI reads an HDR screen's levels wrong in absolute terms. It still sees the change, and only the change is used.
- **One machine, one pass, a busy desktop.** Other desktop apps were open and idle. The numbers are this machine's, not a promise for every PC. A published version should be re-run on a second, more ordinary machine, with ShareX installed there.

## Not measured

- **ShareX.** Not installed on this machine.
  - The driver can measure it: add an entry to `$AppDefs` with its region hotkey and process name `ShareX`, then run with `-Apps Zilum, SnippingTool, ShareX`.
  - ShareX's region capture draws a dimmed, frozen screen, so the same pixel test should work.
- **Copy and save timings.** Out of scope: only the overlay's arrival was measured.

## Restored

- Zilum's `settings.ini` was backed up before any run. It was put back byte for byte after the pilot, the discarded run and the clean run, checked against the backup's SHA-256.
- No Zilum and no Snipping Tool was left running afterwards, as at the start.
- No Windows setting or registry value was changed.
