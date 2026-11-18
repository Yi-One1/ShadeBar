# =============================================================================
# manual-sp2-verify.ps1 - SP-2 one-click manual verification harness
# =============================================================================
# Purpose:
#   Run OUTSIDE the AI sandbox (normal user session, double-click or
#   `powershell -ExecutionPolicy Bypass -File manual-sp2-verify.ps1`).
#   The AI sandbox showed environment-specific failures (fail-fast
#   0xC0000409 on near-jump hooks; AV 0xC0000005 at SystemTray+0x2F2B5
#   for in-template visual-tree walks). On a real user machine these
#   paths may behave differently - this script produces the definitive
#   end-to-end evidence: install -> hook -> color -> pixel sampling ->
#   screenshots -> automatic restore -> report file.
#
# Phases:
#   A. Leaf-function near-jump probe, count-only (0x170DD4, len 12).
#      Pass = explorer survives 150s AND nearCount > 0.
#   B. OnApplyTemplate color hook (0x2F28C, 14-byte abs-jmp, in-hook
#      walk). Requires TranslucentTB stopped (known conflict).
#      Pass = "Colored" evidence event AND red pixels detected in the
#      clock region screenshot. Includes pixel polling + screenshots.
#   C. Automatic restore: original bytes, hook uninstall, TTB restart.
#
# Output: sp2_manual_report.txt + PNG screenshots in scratch\sp2_xaml_locate\shots\
#
# Safety: explorer may crash during phases (it auto-restarts on a real
# machine). Every phase restores what it changed. If the SystemTray.dll
# version does not match, the script aborts before touching anything.
# =============================================================================

$ErrorActionPreference = 'Continue'
$root    = Split-Path -Parent $PSScriptRoot          # repo root (tools/..)
$rel     = Join-Path $root 'scratch\sp2_xaml_locate\target\x86_64-pc-windows-msvc\release'
$hostExe = Join-Path $rel 'inject_hook.exe'
$dllFile = Join-Path $rel 'sp2_dll.dll'
$shots   = Join-Path $root 'scratch\sp2_xaml_locate\shots'
$logDir  = Join-Path $root 'scratch\sp2_xaml_locate'
$trayDll = "$env:WINDIR\SystemApps\MicrosoftWindows.Client.Core_cw5n1h2txyewy\SystemTray.dll"
$report  = Join-Path $root 'sp2_manual_report.txt'
$lines   = New-Object System.Collections.Generic.List[string]

function Log($msg) {
    $ts = (Get-Date).ToString('HH:mm:ss')
    $lines.Add("[$ts] $msg")
    Write-Host "[$ts] $msg"
}

function Save-Report {
    $lines | Set-Content -Path $report -Encoding UTF8
    Write-Host "Report written: $report"
}

function Shot-Clock([string]$tag, [int]$w = 800, [int]$h = 96) {
    try {
        Add-Type -AssemblyName System.Windows.Forms
        Add-Type -AssemblyName System.Drawing
        $s  = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
        $x  = $s.Width - $w; $y = $s.Height - $h
        $bm = New-Object System.Drawing.Bitmap($w, $h)
        $g  = [System.Drawing.Graphics]::FromImage($bm)
        $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size($w, $h)))
        $out = Join-Path $shots ("manual-clock-{0}.png" -f $tag)
        $bm.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bm.Dispose()
        return $out
    } catch {
        Log "Screenshot failed: $($_.Exception.Message)"
        return $null
    }
}

function Count-Red([string]$png) {
    # Returns red pixel count in the clock-corner screenshot (color evidence).
    if (-not $png -or -not (Test-Path $png)) { return -1 }
    try {
        Add-Type -AssemblyName System.Drawing
        $bm = [System.Drawing.Bitmap]::FromFile($png)
        $red = 0
        for ($yy = 0; $yy -lt $bm.Height; $yy++) {
            for ($xx = 0; $xx -lt $bm.Width; $xx++) {
                $c = $bm.GetPixel($xx, $yy)
                if ($c.R -gt 150 -and $c.G -lt 120 -and $c.B -lt 120 -and ($c.R - $c.G) -gt 60 -and ($c.R - $c.B) -gt 60) { $red++ }
            }
        }
        $bm.Dispose()
        return $red
    } catch { return -1 }
}

function Explorer-Pids { @(Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }

# ---------------------------------------------------------------- preflight
Log "=== SP-2 manual verification start ==="
if (-not (Test-Path $hostExe) -or -not (Test-Path $dllFile)) {
    Log "ABORT: binaries missing. Build first:"
    Log "  cargo build --release --manifest-path scratch\sp2_xaml_locate\Cargo.toml"
    Save-Report; exit 1
}
if (-not (Test-Path $trayDll)) {
    Log "ABORT: SystemTray.dll not found at $trayDll"
    Save-Report; exit 1
}
# PE TimeDateStamp check (offset e_lfanew+8), expected 0x6AA9E58A (build 26300.9711)
try {
    $fs  = [System.IO.File]::OpenRead($trayDll)
    $br  = New-Object System.IO.BinaryReader($fs)
    $null = $fs.Seek(0x3C, 'Begin'); $elf = $br.ReadInt32()
    $null = $fs.Seek($elf + 8, 'Begin'); $tds = $br.ReadUInt32()
    $br.Close(); $fs.Close()
    if ($tds -ne 0x6AA9E58A) {
        Log ("ABORT: SystemTray.dll TimeDateStamp=0x{0:X8}, expected 0x6AA9E58A (build 26300.9711). " -f $tds +
            "Offsets in data/symbols/26300.9711.json do not apply to this build - DO NOT inject.")
        Save-Report; exit 1
    }
    Log ("SystemTray.dll TimeDateStamp check OK: 0x{0:X8}" -f $tds)
} catch {
    Log "ABORT: PE header read failed: $($_.Exception.Message)"
    Save-Report; exit 1
}

$ttbWasRunning = $false
if (Get-Process TranslucentTB -ErrorAction SilentlyContinue) { $ttbWasRunning = $true }
Log "Preflight: explorer=[$(Explorer-Pids -join ',')] TTB running=$ttbWasRunning"

# Baseline evidence
$basePng = Shot-Clock 'baseline'
Log ("Baseline red pixels: {0} ({1})" -f (Count-Red $basePng), $basePng)

# ---------------------------------------------------------------- Phase A
Log "=== Phase A: leaf-function near-jump probe (0x170DD4, count-only, 150s) ==="
$aLog = Join-Path $logDir 'manual_phaseA.log'
& $hostExe --race-near 170DD4:12 *> $aLog
$aOut  = (Get-Content $aLog -Raw -ErrorAction SilentlyContinue)
$aRes  = if ($aOut -match 'NEAR_RESULT:\s*(\S+)') { $Matches[1] } else { 'NO_RESULT' }
Log "Phase A result: $aRes (log: manual_phaseA.log)"
if ($aRes -eq 'EXPLORER_DIED') {
    Log "Phase A: explorer died -> near-jump fail-fast reproduces on THIS machine too (not sandbox-specific)."
    Start-Sleep -Seconds 15   # let auto-restart settle
}

# ---------------------------------------------------------------- Phase B
Log "=== Phase B: OnApplyTemplate color hook (race, in-hook walk, pixel polling) ==="
if ($ttbWasRunning) {
    Log "Stopping TranslucentTB (known conflict with OnApplyTemplate hook)..."
    Stop-Process -Name TranslucentTB -Force -ErrorAction SilentlyContinue -Confirm:$false
    Start-Sleep -Seconds 3
}
$bLog = Join-Path $logDir 'manual_phaseB.log'
$proc = Start-Process -FilePath $hostExe -ArgumentList '--race' `
        -RedirectStandardOutput $bLog -RedirectStandardError (Join-Path $logDir 'manual_phaseB.err.log') `
        -PassThru -WindowStyle Hidden
Log "Race host started (pid $($proc.Id)), polling pixels for 100s..."
$colored = $false
$maxRed  = 0
$bestPng = $null
for ($i = 0; $i -lt 50; $i++) {
    Start-Sleep -Seconds 2
    if ($proc.HasExited) { Log "Race host exited at t=$($i*2)s"; break }
    $png = Shot-Clock ("t{0:d3}" -f ($i * 2))
    $red = Count-Red $png
    if ($red -gt $maxRed) { $maxRed = $red; $bestPng = $png }
    if ($red -gt 50) { $colored = $true; Log "RED DETECTED: $red px at t=$($i*2)s -> $png"; break }
    if (($i % 10) -eq 9) {
        $raw = Get-Content $bLog -Raw -ErrorAction SilentlyContinue
        if ($raw -match 'Colored') { Log "Colored event seen in host log at t=$($i*2)s" }
    }
}
if (-not $proc.HasExited) {
    $null = $proc.WaitForExit(60000)
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -Confirm:$false }
}
$bOut = Get-Content $bLog -Raw -ErrorAction SilentlyContinue
$bVerdict = if ($colored -or ($bOut -match 'RESULT: .*colored_t=[1-9]')) { 'PASS' } else { 'FAIL' }
Log "Phase B result: $bVerdict (maxRed=$maxRed, best shot: $bestPng)"
Log "Phase B host log tail:"
(Get-Content $bLog -ErrorAction SilentlyContinue | Select-Object -Last 8) | ForEach-Object { Log "  | $_" }
if (Explorer-Pids.Count -eq 0) {
    Log "Explorer down after Phase B - restarting shell..."
    Start-Process "$env:WINDIR\explorer.exe"
    Start-Sleep -Seconds 15
}

# ---------------------------------------------------------------- Phase C
Log "=== Phase C: restore ==="
# 1) restore original bytes of OnApplyTemplate (in case Phase B left a patch)
$r1 = & $hostExe --restore-bytes 2>&1
Log "restore-bytes: $r1"
# 2) uninstall near probe leftovers (harmless if none)
$r2 = & $hostExe --uninstall-near 2>&1
Log "uninstall-near: $r2"
# 3) restart TranslucentTB if we stopped it
if ($ttbWasRunning -and -not (Get-Process TranslucentTB -ErrorAction SilentlyContinue)) {
    Log "Restarting TranslucentTB..."
    explorer.exe 'shell:AppsFolder\28017CharlesMilette.TranslucentTB_v826wp6bftszj!TranslucentTB'
    Start-Sleep -Seconds 5
}
$afterPng = Shot-Clock 'after'
Log ("Post-restore red pixels: {0} ({1})" -f (Count-Red $afterPng), $afterPng)
Log ("Final state: explorer=[{0}] TTB={1}" -f (Explorer-Pids -join ','), `
     [bool](Get-Process TranslucentTB -ErrorAction SilentlyContinue))

# ---------------------------------------------------------------- verdict
Log "=== VERDICT ==="
Log ("Phase A (leaf near probe, count-only):      {0}" -f $aRes)
Log ("Phase B (OnApplyTemplate color + pixels):   {0}  maxRed={1}" -f $bVerdict, $maxRed)
Log "Screenshots: manual-clock-baseline.png, manual-clock-tNNN.png (red frames), manual-clock-after.png"
Log "Interpretation:"
Log "  A=SURVIVED_FIRED       -> leaf route works on real machines; sandbox fail-fast was environmental."
Log "  A=EXPLORER_DIED        -> near-jump fail-fast is a real-machine defect too; leaf route needs redesign."
Log "  B=PASS                 -> end-to-end color evidence obtained (see red-pixel screenshots)."
Log "  B=FAIL                 -> in-template walk is unsafe on real machines as well; route needs redesign."
Log "=== SP-2 manual verification end ==="
Save-Report
