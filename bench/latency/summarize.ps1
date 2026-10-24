# Median, p95 (nearest rank), min and max of runs.csv per app and run kind, in milliseconds, as a Markdown table.
# The pixel column is the headline: the first screen read the overlay changed. The show column is the app's own
# full-screen window becoming visible, from the WinEvent hook.
param([string]$Runs = (Join-Path $PSScriptRoot 'runs.csv'))

function Get-Stats([double[]]$Values) {
    $s = @($Values | Sort-Object)
    if (-not $s.Count) { return $null }
    [pscustomobject]@{ N = $s.Count; Median = $(if ($s.Count % 2) { $s[($s.Count - 1) / 2] } else { ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2 })
        P95 = $s[[math]::Ceiling(0.95 * $s.Count) - 1]; Min = $s[0]; Max = $s[-1] }
}
$rows = Import-Csv $Runs
'| App | Runs | n | Median | p95 | Min | Max | Window shown, median |'
'|---|---|--:|--:|--:|--:|--:|--:|'
foreach ($app in $rows.app | Select-Object -Unique) {
    foreach ($kind in 'warm', 'cold', 'launch') {
        $set = @($rows | Where-Object { $_.app -eq $app -and $_.kind -eq $kind -and $_.first_pixel_ms })
        if (-not $set.Count) { continue }
        $p = Get-Stats ($set.first_pixel_ms | ForEach-Object { [double]$_ })
        $w = Get-Stats ($set | Where-Object show_event_ms | ForEach-Object { [double]$_.show_event_ms })
        '| {0} | {1} | {2} | {3:F0} | {4:F0} | {5:F0} | {6:F0} | {7:F0} |' -f $app, $kind, $p.N, $p.Median, $p.P95, $p.Min, $p.Max, $w.Median
    }
}
$missed = @($rows | Where-Object { -not $_.first_pixel_ms -or $_.note })
''
"Runs with no overlay seen or a note: $($missed.Count)"
$missed | ForEach-Object { "- run $($_.run) $($_.app) $($_.kind): $($_.note)" }
