<p align="center">
  <img src="assets/logo.svg" width="96" height="96" alt="">
</p>

<h1 align="center">Zilum</h1>

<p align="center">
  A fast, tiny screenshot tool for Windows 11.
</p>

<p align="center">
  <a href="https://github.com/dbeilin/Zilum/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/dbeilin/Zilum/ci.yml?branch=main&style=flat-square&label=build" alt="Build"></a>
  <a href="https://github.com/dbeilin/Zilum/releases/latest"><img src="https://img.shields.io/github/v/release/dbeilin/Zilum?style=flat-square&color=2060D0" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2011-2060D0?style=flat-square" alt="Windows 11">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/dbeilin/Zilum?style=flat-square&color=2060D0" alt="MIT licence"></a>
</p>

<p align="center">
  <a href="https://github.com/dbeilin/Zilum/releases/latest/download/Zilum.exe"><b>Download</b></a>
  &nbsp;·&nbsp;
  <a href="https://dbeilin.github.io/Zilum/">Website</a>
  &nbsp;·&nbsp;
  <a href="https://github.com/dbeilin/Zilum/releases">Changelog</a>
</p>

<p align="center">
  <img src="website/assets/hero-editor-arrow.png" width="880" alt="The Zilum editor over a capture of the Windows 11 Bloom wallpaper, its bar along the top and one red arrow placed.">
</p>

## What it does

Hit PrintScreen, grab a region or a window, and mark it up.

- Arrows, boxes, pen, text and numbered steps
- Blur in one drag
- Crop and resize
- Colour picker that copies the hex code
- Copy or save in one key
- Pin a capture above your other windows
- OCR: copy the text out of anything on screen

<table>
  <tr>
    <td width="50%"><img src="assets/capture.png" alt="The overlay mid-drag: the selection lit, the screen around it dimmed, and 841 × 441 beside the pointer."></td>
    <td width="50%"><img src="website/assets/editor-arrow-blur-flyout.png" alt="The editor with an arrow and a blur placed, the blur's Level slider open under the bar."></td>
  </tr>
</table>

## Small on purpose

| 1.3 MB | ~20 MB | 50 ms |
|:-:|:-:|:-:|
| single exe | memory in use | key to overlay |

The Snipping Tool takes 198 ms to open its overlay on the same machine ([how it was measured](bench/)).

## Privacy

Zilum works offline and collects nothing. The exe contains no networking code, so there's no telemetry and no update check (watch this repo's releases for new versions). Captures go to your clipboard or save folder, and OCR runs on your PC with the engine built into Windows.

## Install

1. Download [`Zilum.exe`](https://github.com/dbeilin/Zilum/releases/latest/download/Zilum.exe) from the latest release and run it. Zilum lives in the tray.
2. The exe is not code-signed yet, so SmartScreen may stop it the first time: choose **More info**, then **Run anyway**. Each release lists the exe's SHA-256 next to it.
3. To have Zilum running after a restart, turn on **Start with Windows** in its Settings.
4. To update, download the new `Zilum.exe` and run it. It closes the older copy that is running and takes its place.

> [!NOTE]
> If PrintScreen still opens the Snipping Tool, turn off **Use the Print screen key to open screen capture** in Windows Settings › Accessibility › Keyboard.

To remove Zilum, turn off Start with Windows, quit it from the tray and delete the exe. Its settings are in `%APPDATA%\Zilum`.

## Usage

| Key | Does |
|---|---|
| <kbd>PrintScreen</kbd> | Capture: drag a region, or click a window |
| <kbd>Shift</kbd>+<kbd>PrintScreen</kbd> | Capture the whole screen |

Capture window and Copy text are in the tray menu, and either can have its own key in Settings. Every key can be changed there.

<details>
<summary>Keys in the editor</summary>

| Key | Does |
|---|---|
| <kbd>Enter</kbd> or <kbd>Ctrl</kbd>+<kbd>C</kbd> | Copy |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save to the save folder |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Save as |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Ctrl</kbd>+<kbd>Y</kbd> | Undo / Redo |
| <kbd>Ctrl</kbd>+<kbd>P</kbd> | Pin |
| <kbd>Esc</kbd> | Close |
| <kbd>S</kbd> <kbd>A</kbd> <kbd>R</kbd> <kbd>P</kbd> <kbd>T</kbd> <kbd>N</kbd> <kbd>B</kbd> <kbd>I</kbd> <kbd>C</kbd> | Select, Arrow, Rectangle, Pen, Text, Badge, Blur, Colour picker, Crop |

</details>

## Build

Zilum is one exe written in Rust. On Windows 11 with Rust 1.85 or later (MSVC toolchain):

```
cargo build --release
```

The exe is `target\release\Zilum.exe`.

## Licence

MIT, see [LICENSE](LICENSE).
