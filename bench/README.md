# Benchmarks

## Hotkey to overlay

`latency/` measures how long each app's capture overlay takes to appear on screen after its hotkey. Zilum and the Windows Snipping Tool were measured the same way, from outside both apps, on one machine on 2026-10-23.

The two numbers on the website are the warm medians from [`latency/findings.md`](latency/findings.md):

| App | Warm median | Runs |
|---|--:|--:|
| Zilum | 50 ms | 30 |
| Snipping Tool | 198 ms | 30 |

"Warm" means back-to-back presses, one second after the previous overlay closed. The findings also give the runs after a minute idle and from a stopped process, the machine, the method and the caveats.

### What is in `latency/`

| File | What it is |
|---|---|
| `findings.md` | The results, the machine, the method and the fairness caveats |
| `measure-latency.ps1` | The driver: presses the hotkey, reads the screen until the overlay appears, closes it, repeats |
| `summarize.ps1` | Turns `runs.csv` into the table in the findings |
| `runs.csv` | One row per run, 80 runs |
| `samples.csv` | Every screen read of every run |
| `events.csv` | Every top-level window shown after each press |

### Rerun it

You need Windows 11, PowerShell 7, the Snipping Tool, and a desktop nobody else is using while it runs. It takes about 15 minutes, most of it the one-minute idles before the cold runs.

The driver assumes a 2560 x 1440 primary screen. It places its two test windows at fixed positions there, and it counts an app's window as the overlay once that window covers half of that area. On another screen size, change the layout block and the `2560 * 1440` in the script first.

1. Build Zilum:

   ```
   cargo build --release
   ```

2. Copy Zilum's settings, so the driver can put them back. It writes its own hotkey into `settings.ini` for the run and restores this copy when it ends, however it ends:

   ```
   Copy-Item "$env:APPDATA\Zilum\settings.ini" "$env:APPDATA\Zilum\settings.before-bench.ini"
   ```

3. Run the driver into a new folder, so the published CSVs stay as they are:

   ```
   pwsh -File bench\latency\measure-latency.ps1 -ZilumExe target\release\Zilum.exe -SettingsBackup "$env:APPDATA\Zilum\settings.before-bench.ini" -OutDir bench\latency\rerun
   ```

4. Summarise:

   ```
   pwsh -File bench\latency\summarize.ps1 -Runs bench\latency\rerun\runs.csv
   ```

Zilum is pressed with `Ctrl+Shift+F9` during the run, and the Snipping Tool with `Win+Shift+S`. To add ShareX, give it an entry in the driver's `$AppDefs` and pass `-Apps Zilum, SnippingTool, ShareX`.
