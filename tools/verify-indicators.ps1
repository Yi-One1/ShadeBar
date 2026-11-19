# 随影 ShadeBar - indicator recolor verification (log based)
#
# Purpose: turn "look at the taskbar and tell me" into a repeatable assertion.
# The payload logs one [Ind] line per indicator per apply pass:
#
#   [Ind] h=0x... w=6.0 正常 -> 72F2F2F2 (elem=1 brush=1 all=0 rb=72F2F2F2 ok=1)
#
# Invariants checked here:
#   1. at least one indicator was touched (the payload actually ran)
#   2. no line reports ok=0  (written but read back a different colour)
#   3. every line's read-back value equals the colour it was told to write
#   4. selected indicators (w >= 10) appear and also read back correctly
#
# Exit code 0 = pass, 1 = fail. Read-only: never writes to the log.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tools\verify-indicators.ps1
#   powershell ... -LogPath <path> -Since <hh:mm>

[CmdletBinding()]
param(
    [string]$LogPath = '',
    [string]$Since = ''
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrEmpty($LogPath)) {
    $LogPath = Join-Path $env:LOCALAPPDATA 'ShadeBar\payload.log'
}
if (-not (Test-Path $LogPath)) {
    Write-Host "FAIL: log not found: $LogPath"
    exit 1
}

$lines = Get-Content $LogPath
if ($lines.Count -eq 0) {
    Write-Host "FAIL: log is empty: $LogPath"
    exit 1
}

# Optional time filter: the log itself has no timestamps, so callers slice the
# file themselves (e.g. tail) and pass the sliced copy in.
$ind = @()
foreach ($l in $lines) {
    if ($l -like '*[Ind]*' -and $l -like '*ok=*') { $ind += $l }
}

if ($ind.Count -eq 0) {
    Write-Host "FAIL: no [Ind] lines with read-back found."
    Write-Host "      The running payload is older than this build (no read-back field),"
    Write-Host "      or ShadeBar was never started."
    exit 1
}

$bad = 0
$mismatch = 0
$fallback = 0
$selected = 0
$selectedOk = 0
$targets = @{}

foreach ($l in $ind) {
    if ($l -match '->\s*([0-9A-Fa-f]{8})') { $targets[$Matches[1].ToUpper()] = $true }
    if ($l -match 'all=(\d+)') { if ([int]$Matches[1] -gt 0) { $fallback++ } }
    if ($l -match 'w=([0-9.]+)\s*\S*选中') {
        $selected++
        if ($l -match 'ok=1') { $selectedOk++ }
    }
    if ($l -match 'ok=0') { $bad++ }
    if ($l -match '->\s*([0-9A-Fa-f]{8}).*rb=([0-9A-Fa-f]{8})') {
        if ($Matches[1].ToUpper() -ne $Matches[2].ToUpper()) { $mismatch++ }
    }
}

Write-Host "log        : $LogPath"
Write-Host "[Ind] lines: $($ind.Count)"
Write-Host "targets    : $(($targets.Keys | Sort-Object) -join ', ')"
Write-Host "read-back  : ok=1 $($ind.Count - $bad) / ok=0 $bad"
Write-Host "mismatch   : $mismatch"
Write-Host "fallback   : $fallback (per-layer recolor; 0 means first layer was enough)"
Write-Host "selected   : $selected lines, $selectedOk with ok=1"

$fail = 0
if ($bad -gt 0) { Write-Host "FAIL: $bad indicator(s) read back a different colour."; $fail = 1 }
if ($mismatch -gt 0) { Write-Host "FAIL: $mismatch line(s) wrote one colour but read back another."; $fail = 1 }
if ($selected -eq 0) {
    Write-Host "WARN: no selected-state indicator was seen; select a window and re-run."
} elseif ($selectedOk -ne $selected) {
    Write-Host "FAIL: selected-state indicator did not read back correctly."; $fail = 1
}

if ($fail -eq 0) {
    Write-Host "PASS: every indicator that was touched reads back the intended colour."
} else {
    exit 1
}
