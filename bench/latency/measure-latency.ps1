# Hotkey-to-overlay latency, measured from outside every app.
#
# The same method for every app:
#   1. Two plain windows of this script's own sit topmost on the primary screen: a white SAMPLE square and a grey PAD.
#      The pointer rests on the pad and the pad has the front, so no app starts a run from a different state, and an
#      app that lights the window under the pointer (Zilum does) lights the pad, never the sample.
#   2. The hotkey goes in as one SendInput batch of key-downs; a QueryPerformanceCounter timestamp is taken right
#      before that call. The key-ups follow 30 ms later.
#   3. From then on the script reads the sample square's middle 16 x 16 pixels off the screen through GDI in a tight
#      loop, each read stamped with QPC. The overlay is "on screen" at the first read whose mean differs from the
#      read before the press by more than $Threshold levels: every overlay measured dims the screen.
#   4. A WinEvent hook (EVENT_OBJECT_SHOW, out of context) stamps every top-level window shown, so the time each
#      app's own overlay window was made visible is kept beside the pixel time.
#   5. Esc closes the overlay, sent only when the app's own process has the front. The next run waits until the
#      sample reads as before.
#
# Run kinds: 'launch' is the first press after the app's process was stopped: Zilum is started again and given
# 3 s, since it must be running to own a hotkey at all; the Snipping Tool is stopped and Windows starts it from the
# hotkey. 'cold' follows $ColdIdleSec seconds with no press at all, the process resident; 'warm' follows the previous
# run by $WarmGapMs once its overlay has gone. A Snipping Tool that was not running when the script began is stopped
# again at the end.
#
# Run it from an unsandboxed PowerShell 7 on a desktop nobody is using. It writes runs.csv, samples.csv and
# events.csv to $OutDir. When Zilum is measured, it writes a copy of $SettingsBackup with only hotkey_capture
# changed, and puts $SettingsBackup back over settings.ini when it ends, however it ends.

param(
    [string[]]$Apps = @('Zilum', 'SnippingTool'),
    [int]$WarmRuns = 30,
    [int]$ColdRuns = 5,
    [int]$LaunchRuns = 5,
    [int]$ColdIdleSec = 60,
    [int]$WarmGapMs = 1000,
    [int]$Threshold = 12,
    [string]$ZilumExe,
    [string]$ZilumHotkey = 'Ctrl+Shift+F9',
    [string]$SettingsBackup,
    [string]$OutDir = $PSScriptRoot
)
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class Lat {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l, t, r, b; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MOUSEINPUT { public int dx, dy; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; [FieldOffset(8)] public MOUSEINPUT mi; }
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr w, l; public uint time; public int x, y; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] struct WNDCLASSEX {
        public uint cbSize, style; public WndProc proc; public int clsExtra, wndExtra; public IntPtr inst, icon, cursor, background;
        public string menu, name; public IntPtr iconSm; }
    [StructLayout(LayoutKind.Sequential)] struct BITMAPINFOHEADER { public uint size; public int w, h; public ushort planes, bits; public uint comp, sizeImage; public int xppm, yppm; public uint used, important; }
    delegate IntPtr WndProc(IntPtr h, uint m, IntPtr w, IntPtr l);
    delegate void WinEventProc(IntPtr hook, uint ev, IntPtr hwnd, int obj, int child, uint thread, uint time);

    [DllImport("user32.dll")] static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKeyW(uint code, uint type);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern ushort RegisterClassExW(ref WNDCLASSEX c);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex, string cls, string name, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr inst, IntPtr param);
    [DllImport("user32.dll")] static extern IntPtr DefWindowProcW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr h);
    [DllImport("user32.dll")] static extern int GetMessageW(out MSG m, IntPtr h, uint min, uint max);
    [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref MSG m);
    [DllImport("user32.dll")] static extern bool PostThreadMessageW(uint thread, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventProc f, uint pid, uint tid, uint flags);
    [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] static extern IntPtr CreateDIBSection(IntPtr dc, ref BITMAPINFOHEADER bi, uint usage, out IntPtr bits, IntPtr section, uint offset);
    [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
    [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
    [DllImport("gdi32.dll")] static extern IntPtr CreateSolidBrush(uint rgb);
    [DllImport("gdi32.dll")] static extern bool GdiFlush();
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] static extern IntPtr GetModuleHandleW(IntPtr name);

    public static readonly double TicksPerMs = Stopwatch.Frequency / 1000.0;
    public static long Now() { return Stopwatch.GetTimestamp(); }
    public static double Ms(long from, long to) { return (to - from) / TicksPerMs; }

    // ---- The two surfaces and the hook, on a thread of their own with a message loop --------------------------
    public static IntPtr Sample, Pad;
    static uint uiThread;
    static WndProc proc = (h, m, w, l) => DefWindowProcW(h, m, w, l);
    static WinEventProc hookProc = OnEvent;
    static IntPtr hook;
    public class Shown { public long Qpc; public uint Pid; public string Process; public string Class; public int L, T, W, H; }
    public static ConcurrentQueue<Shown> Events = new ConcurrentQueue<Shown>();
    static Dictionary<uint, string> names = new Dictionary<uint, string>();

    public static void Start(int sx, int sy, int sw, int px, int py, int pw, int ph) {
        SetProcessDpiAwarenessContext(new IntPtr(-4));   // per-monitor v2: physical pixels everywhere
        var ready = new ManualResetEvent(false);
        var t = new Thread(() => {
            uiThread = GetCurrentThreadId();
            IntPtr inst = GetModuleHandleW(IntPtr.Zero);
            foreach (var c in new[] { new { Name = "LatencySample", Rgb = 0xFFFFFFu }, new { Name = "LatencyPad", Rgb = 0x808080u } }) {
                var wc = new WNDCLASSEX { cbSize = (uint)Marshal.SizeOf(typeof(WNDCLASSEX)), proc = proc, inst = inst, background = CreateSolidBrush(c.Rgb), name = c.Name };
                RegisterClassExW(ref wc);
            }
            const uint POPUP = 0x80000000, TOPMOST = 0x8, TOOL = 0x80;
            Sample = CreateWindowExW(TOPMOST | TOOL, "LatencySample", "Latency sample", POPUP, sx, sy, sw, sw, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
            Pad = CreateWindowExW(TOPMOST | TOOL, "LatencyPad", "Latency pad", POPUP, px, py, pw, ph, IntPtr.Zero, IntPtr.Zero, inst, IntPtr.Zero);
            ShowWindow(Sample, 4); ShowWindow(Pad, 5);
            hook = SetWinEventHook(0x8002, 0x8002, IntPtr.Zero, hookProc, 0, 0, 0x0000 | 0x0002);   // EVENT_OBJECT_SHOW, out of context, not our own
            ready.Set();
            MSG m;
            while (GetMessageW(out m, IntPtr.Zero, 0, 0) > 0) { TranslateMessage(ref m); DispatchMessageW(ref m); }
            UnhookWinEvent(hook); DestroyWindow(Sample); DestroyWindow(Pad);
        });
        t.IsBackground = true; t.SetApartmentState(ApartmentState.STA); t.Start();
        ready.WaitOne();
    }
    public static void Stop() { PostThreadMessageW(uiThread, 0x0012, IntPtr.Zero, IntPtr.Zero); Thread.Sleep(200); }   // WM_QUIT

    static void OnEvent(IntPtr h, uint ev, IntPtr hwnd, int obj, int child, uint thread, uint time) {
        long q = Now();
        if (obj != 0 || child != 0 || hwnd == IntPtr.Zero || GetAncestor(hwnd, 2) != hwnd) return;   // top-level windows only
        uint pid; GetWindowThreadProcessId(hwnd, out pid);
        RECT r; GetWindowRect(hwnd, out r);
        var cls = new StringBuilder(256); GetClassNameW(hwnd, cls, 256);
        Events.Enqueue(new Shown { Qpc = q, Pid = pid, Process = Name(pid), Class = cls.ToString(), L = r.l, T = r.t, W = r.r - r.l, H = r.b - r.t });
    }
    public static string Name(uint pid) {
        string n;
        lock (names) { if (names.TryGetValue(pid, out n)) return n; }
        try { n = Process.GetProcessById((int)pid).ProcessName; } catch { n = "?"; }
        lock (names) { names[pid] = n; }
        return n;
    }
    public static string ForegroundProcess() { uint pid; GetWindowThreadProcessId(GetForegroundWindow(), out pid); return Name(pid); }

    // ---- Front and keys ------------------------------------------------------------------------------------
    public static bool PadToFront() {
        // A zero-distance mouse move makes this process the last to have input, which Windows wants before it
        // lets a background process set the front.
        var mv = new INPUT[] { new INPUT { type = 0, mi = new MOUSEINPUT { flags = 1 } } };
        SendInput(1, mv, Marshal.SizeOf(typeof(INPUT)));
        SetForegroundWindow(Pad);
        Thread.Sleep(150);
        return GetForegroundWindow() == Pad;
    }
    static INPUT Key(ushort vk, bool up) {
        uint flags = up ? 2u : 0u;
        if (vk == 0x5B || vk == 0x5C || (vk >= 0x21 && vk <= 0x2E)) flags |= 1;   // extended: the Windows keys, navigation keys
        return new INPUT { type = 1, ki = new KEYBDINPUT { vk = vk, scan = (ushort)MapVirtualKeyW(vk, 0), flags = flags } };
    }
    // All key-downs in one batch; returns the QPC taken right before the batch went in.
    // The key-ups follow upDelayMs later from a thread of their own, so the screen reads can start at once.
    public static long Press(ushort[] vks, int upDelayMs) {
        var downs = new INPUT[vks.Length];
        for (int i = 0; i < vks.Length; i++) downs[i] = Key(vks[i], false);
        long t0 = Now();
        SendInput((uint)downs.Length, downs, Marshal.SizeOf(typeof(INPUT)));
        lastUp = new Thread(() => { Thread.Sleep(upDelayMs); Release(vks); });
        lastUp.Start();
        return t0;
    }
    static Thread lastUp;
    public static void WaitUp() { if (lastUp != null) lastUp.Join(); }
    public static void Release(ushort[] vks) {
        var ups = new INPUT[vks.Length];
        for (int i = 0; i < vks.Length; i++) ups[i] = Key(vks[vks.Length - 1 - i], true);
        SendInput((uint)ups.Length, ups, Marshal.SizeOf(typeof(INPUT)));
    }

    // ---- Screen reads --------------------------------------------------------------------------------------
    static IntPtr screen, mem, bits;
    const int N = 16;
    public static void PrepareReads() {
        screen = GetDC(IntPtr.Zero); mem = CreateCompatibleDC(screen);
        var bi = new BITMAPINFOHEADER { size = 40, w = N, h = -N, planes = 1, bits = 32 };
        IntPtr dib = CreateDIBSection(screen, ref bi, 0, out bits, IntPtr.Zero, 0);
        SelectObject(mem, dib);
    }
    // Mean of R, G and B over the 16 x 16 square at (x, y).
    static byte[] px = new byte[N * N * 4];
    public static double Read(int x, int y) {
        BitBlt(mem, 0, 0, N, N, screen, x, y, 0x00CC0020 | 0x40000000);   // SRCCOPY | CAPTUREBLT
        GdiFlush();
        Marshal.Copy(bits, px, 0, px.Length); long sum = 0;
        for (int i = 0; i < N * N; i++) sum += px[i * 4] + px[i * 4 + 1] + px[i * 4 + 2];
        return sum / (3.0 * N * N);
    }
    // Reads until a change of more than threshold from baseline has held for holdMs, or timeoutMs passes.
    // Returns pairs of (ms since t0, mean).
    public static double[] Watch(int x, int y, long t0, double baseline, double threshold, int holdMs, int timeoutMs) {
        var list = new List<double>(8192); long seen = 0;
        while (true) {
            double v = Read(x, y); long now = Now(); double ms = Ms(t0, now);
            list.Add(ms); list.Add(v);
            if (seen == 0 && Math.Abs(v - baseline) > threshold) seen = now;
            if (seen != 0 && Ms(seen, now) >= holdMs) break;
            if (ms >= timeoutMs) break;
        }
        return list.ToArray();
    }
    // Ms until the square reads within threshold of baseline again, or -1.
    public static double WaitBack(int x, int y, double baseline, double threshold, int timeoutMs) {
        long t = Now();
        while (Ms(t, Now()) < timeoutMs) { if (Math.Abs(Read(x, y) - baseline) <= threshold) return Ms(t, Now()); Thread.Sleep(5); }
        return -1;
    }
}
'@

# ---- Apps ----------------------------------------------------------------------------------------------
$VkNames = @{ Ctrl = 0x11; Shift = 0x10; Alt = 0x12; Win = 0x5B; PrintScreen = 0x2C }
function ConvertTo-Vks([string]$Hotkey) {
    @($Hotkey -split '\+' | ForEach-Object {
        if ($VkNames.ContainsKey($_)) { $VkNames[$_] }
        elseif ($_ -match '^F(\d+)$') { 0x6F + [int]$Matches[1] }
        elseif ($_.Length -eq 1) { [int][char]$_.ToUpper() }
        else { throw "Unknown key '$_' in $Hotkey" }
    }) -as [uint16[]]
}
$AppDefs = @{
    Zilum        = @{ Hotkey = $ZilumHotkey; Processes = @('Zilum') }
    SnippingTool = @{ Hotkey = 'Win+Shift+S'; Processes = @('SnippingTool', 'ScreenClippingHost') }
}

# ---- Layout on the primary screen ------------------------------------------------------------------------
$SampleX = 200; $SampleY = 300; $SampleSide = 240
$ReadX = $SampleX + 112; $ReadY = $SampleY + 112          # the 16 x 16 read square, in the sample's middle
$PadX = 700; $PadY = 300; $PadW = 500; $PadH = 360
$PointerX = $PadX + 250; $PointerY = $PadY + 180

# ---- Output ----------------------------------------------------------------------------------------------
New-Item -ItemType Directory -Force $OutDir | Out-Null
$RunsCsv = Join-Path $OutDir 'runs.csv'; $SamplesCsv = Join-Path $OutDir 'samples.csv'; $EventsCsv = Join-Path $OutDir 'events.csv'
'run,app,kind,process_before,hotkey,started,baseline,overlay_level,first_pixel_ms,settled_ms,show_event_ms,show_event_class,reads,median_read_ms,front_after,closed_ms,note' | Set-Content $RunsCsv
'run,ms,mean' | Set-Content $SamplesCsv
'run,ms,process,class,left,top,width,height' | Set-Content $EventsCsv
$script:Run = 0

function Measure-Once([string]$App, [string]$Kind) {
    $def = $AppDefs[$App]; $vks = ConvertTo-Vks $def.Hotkey
    $script:Run++; $run = $script:Run
    [Lat]::SetCursorPos($PointerX, $PointerY) | Out-Null
    if (-not [Lat]::PadToFront() -and -not [Lat]::PadToFront()) { throw "Run $run ($App): the pad did not come to the front (front: $([Lat]::ForegroundProcess())); stopping before any key goes to another app." }
    $before = [bool](Get-Process -Name $def.Processes -ErrorAction SilentlyContinue)
    if ($App -eq 'Zilum') {
        # Another Zilum, such as one a different checkout started, would answer a different hotkey or none.
        $paths = @(Get-Process 'Zilum*' -ErrorAction SilentlyContinue | ForEach-Object Path)
        if ($paths.Count -ne 1 -or $paths[0] -ne (Resolve-Path $ZilumExe).Path) { throw "Run $run`: the Zilum running is not the one measured ($($paths -join ', ')); stopping." }
    }
    $null = [Lat]::Events.Clear()
    $baseline = [Lat]::Read($ReadX, $ReadY)
    $started = Get-Date -Format 'o'
    $t0 = [Lat]::Press($vks, 30)
    $reads = [Lat]::Watch($ReadX, $ReadY, $t0, $baseline, $Threshold, 700, 6000)
    [Lat]::WaitUp()

    $ms = for ($i = 0; $i -lt $reads.Length; $i += 2) { $reads[$i] }
    $val = for ($i = 1; $i -lt $reads.Length; $i += 2) { $reads[$i] }
    $first = -1; for ($i = 0; $i -lt $val.Count; $i++) { if ([math]::Abs($val[$i] - $baseline) -gt $Threshold) { $first = $i; break } }
    $level = $val[-1]; $settled = -1
    if ($first -ge 0) {
        $band = [math]::Max(3, 0.1 * [math]::Abs($level - $baseline))
        for ($i = $val.Count - 1; $i -ge $first; $i--) { if ([math]::Abs($val[$i] - $level) -gt $band) { break } }
        $settled = $ms[[math]::Min($i + 1, $val.Count - 1)]
    }
    $gaps = for ($i = 1; $i -lt $ms.Count; $i++) { $ms[$i] - $ms[$i - 1] }
    $medianRead = ($gaps | Sort-Object)[[int]($gaps.Count / 2)]

    # The app's own overlay window: the first top-level window of its processes shown after the press that
    # covers at least half of the primary screen.
    $events = @(); $e = $null
    while ([Lat]::Events.TryDequeue([ref]$e)) { $events += $e }
    $show = $events | Where-Object { $_.Qpc -ge $t0 -and $def.Processes -contains $_.Process -and ($_.W * $_.H) -ge (2560 * 1440 / 2) } | Select-Object -First 1
    $showMs = if ($show) { [math]::Round([Lat]::Ms($t0, $show.Qpc), 2) } else { '' }

    # Close: Esc only while the app's own process has the front.
    $front = [Lat]::ForegroundProcess(); $note = ''
    if ($first -lt 0) { $note = 'no overlay seen' }
    if ($def.Processes -contains $front) { [void][Lat]::Press([uint16[]]@(0x1B), 20); [Lat]::WaitUp() }
    elseif ($first -ge 0) { $note = "front was $front, no Esc sent" }
    $closed = [Lat]::WaitBack($ReadX, $ReadY, $baseline, $Threshold, 5000)

    $line = '{0},{1},{2},{3},{4},{5},{6:F1},{7:F1},{8},{9},{10},{11},{12},{13:F2},{14},{15},{16}' -f $run, $App, $Kind, $before, $def.Hotkey, $started, $baseline, $level,
        $(if ($first -ge 0) { [math]::Round($ms[$first], 2) } else { '' }), $(if ($settled -ge 0) { [math]::Round($settled, 2) } else { '' }),
        $showMs, $(if ($show) { $show.Class } else { '' }), $ms.Count, $medianRead, $front, [math]::Round($closed), $note
    Add-Content $RunsCsv $line
    $sb = [Text.StringBuilder]::new()
    for ($i = 0; $i -lt $ms.Count; $i++) { [void]$sb.AppendLine(('{0},{1:F3},{2:F1}' -f $run, $ms[$i], $val[$i])) }
    Add-Content $SamplesCsv $sb.ToString().TrimEnd() -NoNewline:$false
    foreach ($ev in $events | Where-Object { $_.Qpc -ge $t0 }) {
        Add-Content $EventsCsv ('{0},{1:F2},{2},{3},{4},{5},{6},{7}' -f $run, [Lat]::Ms($t0, $ev.Qpc), $ev.Process, $ev.Class, $ev.L, $ev.T, $ev.W, $ev.H)
    }
    Write-Output ('run {0,3} {1,-12} {2,-5} first {3,8} ms  settled {4,8} ms  show {5,8} ms  front {6} {7}' -f $run, $App, $Kind,
        $(if ($first -ge 0) { '{0:F1}' -f $ms[$first] } else { '-' }), $(if ($settled -ge 0) { '{0:F1}' -f $settled } else { '-' }), $showMs, $front, $note)
    if ($closed -lt 0) { throw "Run $run ($App): the overlay did not close within 5 s (front: $([Lat]::ForegroundProcess())); stopping." }
}

# ---- The run -----------------------------------------------------------------------------------------------
$settingsIni = Join-Path $env:APPDATA 'Zilum\settings.ini'
if ($Apps -contains 'Zilum') {
    if (-not $ZilumExe -or -not (Test-Path $ZilumExe)) { throw 'Pass -ZilumExe, the release exe to measure.' }
    if (-not $SettingsBackup -or -not (Test-Path $SettingsBackup)) { throw 'Pass -SettingsBackup, a copy of settings.ini made before this run.' }
}
$snippingBefore = [bool](Get-Process -Name $AppDefs.SnippingTool.Processes -ErrorAction SilentlyContinue)
try {
    [Lat]::Start($SampleX, $SampleY, $SampleSide, $PadX, $PadY, $PadW, $PadH)
    [Lat]::PrepareReads()
    Start-Sleep -Milliseconds 600   # the surfaces fade in
    if ($Apps -contains 'Zilum') {
        $text = (Get-Content $SettingsBackup) -replace '^hotkey_capture=.*$', "hotkey_capture=$ZilumHotkey"
        if (-not ($text -match '^hotkey_capture=')) { $text = @("hotkey_capture=$ZilumHotkey") + $text }
        Set-Content $settingsIni $text
    }
    for ($r = 0; $r -lt [math]::Max($LaunchRuns, 1); $r++) {
        foreach ($app in $Apps) {
            Get-Process -Name $AppDefs[$app].Processes -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Seconds 2
            if ($app -eq 'Zilum') { $null = Start-Process $ZilumExe; Start-Sleep -Seconds 3 }   # startup and the hotkey's registration
            Measure-Once $app 'launch'; Start-Sleep -Milliseconds $WarmGapMs
        }
    }
    for ($r = 0; $r -lt $ColdRuns; $r++) {
        foreach ($app in $Apps) { Start-Sleep -Seconds $ColdIdleSec; Measure-Once $app 'cold' }
    }
    foreach ($app in $Apps) {
        for ($r = 0; $r -lt $WarmRuns; $r++) { Measure-Once $app 'warm'; Start-Sleep -Milliseconds $WarmGapMs }
    }
}
finally {
    [Lat]::Stop()
    if ($Apps -contains 'Zilum') {
        Get-Process 'Zilum*' -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Milliseconds 500
        Copy-Item $SettingsBackup $settingsIni -Force
        Write-Output "settings.ini restored from $SettingsBackup"
    }
    if (-not $snippingBefore) { Get-Process -Name $AppDefs.SnippingTool.Processes -ErrorAction SilentlyContinue | Stop-Process -Force }
}
