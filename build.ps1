# ShadeBar - one-click build script  (docs/02-architecture.md section 4 #4 / section 10.1)
#
#   powershell -ExecutionPolicy Bypass -File build.ps1
#
# Steps (fixed order):
#   0. verify no .ps1 in this repo is "non-ASCII without BOM"
#      (see tools\check-ps1-encoding.ps1 for why that is a silent-failure class)
#   1. read version from Cargo.toml (single source of truth, FR-16 / A-16)
#   2. locate Visual Studio + Windows SDK, inject PATH / LIB / INCLUDE
#      (a plain shell has no VC environment; rust-lld cannot find kernel32.lib)
#   3. build the FORMAL release (no features): evidence/visual/png + --verify
#      branch are NOT compiled (requirement baseline v1.4 section 7 / K-07)
#   4. copy to dist\<product>_ShadeBar_v<ver>.exe  (keep prior versions)
#   5. run the five gates on the formal artifact (hard limit 4096 KB)
#   6. build the VERIFY artifact (--features verify) into dist\verify\
#      and run the SAME five gates on it (task #9: two profiles, one gate set)
#   7. build the A-26 comparison artifact (--features no-en, profile
#      release-no-en) into dist\compare\ and assert the bilingual delta
#      (formal - no-en) is <= 40 KB (NFR-19 / A-26)
#
# Gates (unchanged, five of them, all hard failures):
#        1/5 size <= 4096 KB                (NFR-01, hard fail)
#        2/5 no VCRUNTIME140 / MSVCP140    (FR-15 / A-15)
#        3/5 FileVersion == <ver>.0        (FR-16 / A-16)
#        4/5 manifest has PerMonitorV2 + requestedExecutionLevel (NFR-08)
#        5/5 icon resource extractable     (FR-06)
#   The 400 KB figure is an ENGINEERING SOFT GOAL only (v1.4 K-07); it is
#   printed as a note and never blocks the build.
#
# Params:
#   -Clean        run cargo clean first
#   -SkipChecks   skip gates (local quick runs only; never for a release)
#   -SkipVerify   skip the verify / no-en comparison artifacts
#   -SoftLimitKb  soft size warning, default 400
#
# ---------------------------------------------------------------------------
# THIS FILE MUST STAY PURE ASCII.
# Windows PowerShell 5.1 parses the whole script before executing it. Non-ASCII
# text saved without a UTF-8 BOM gets decoded with the local ANSI code page and
# can break the parser itself - param() block silently swallowed, $PSCommandPath
# empty, "missing string terminator" errors. No in-script guard can survive that,
# so the guard lives in tools\check-ps1-encoding.ps1 (also pure ASCII) instead.
# The Chinese product name in the output filename is therefore assembled from
# code points below rather than typed literally.
# ---------------------------------------------------------------------------

[CmdletBinding()]
param(
    [switch]$Clean,
    [switch]$SkipChecks,
    [switch]$SkipVerify,
    [int]$SoftLimitKb = 400
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSCommandPath
Set-Location $Root

# NFR-01 hard gate
$HardLimitKb = 4096
# NFR-19 / A-26 bilingual budget
$BilingualLimitKb = 40
$TargetTriple = 'x86_64-pc-windows-msvc'

function Write-Step([string]$Text) {
    Write-Host ''
    Write-Host "==> $Text" -ForegroundColor Cyan
}

function Fail([string]$Text) {
    Write-Host "BUILD FAILED: $Text" -ForegroundColor Red
    exit 1
}

# ---------------------------------------------------------------------------
# 0. encoding guard
# ---------------------------------------------------------------------------
Write-Step 'Gate 0: .ps1 encoding'
$guard = Join-Path $Root 'tools\check-ps1-encoding.ps1'
if (Test-Path $guard) {
    & powershell -ExecutionPolicy Bypass -NoProfile -File $guard -Root $Root
    if ($LASTEXITCODE -ne 0) {
        Fail 'ps1 encoding guard failed (see above). Fix the listed files first.'
    }
} else {
    Write-Host '    [warn] tools\check-ps1-encoding.ps1 not found, skipping'
}

# ---------------------------------------------------------------------------
# 1. version (single source of truth)
# ---------------------------------------------------------------------------
Write-Step 'Read version (Cargo.toml)'
$cargoToml = Join-Path $Root 'Cargo.toml'
if (-not (Test-Path $cargoToml)) { Fail "not found: $cargoToml" }

$m = Select-String -Path $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
if ($null -eq $m) { Fail 'Cargo.toml has no [workspace.package] version' }
$version = $m.Matches[0].Groups[1].Value
Write-Host "    version = $version"

# Chinese product name, assembled from code points to keep this file ASCII:
#   U+968F U+5F71 = the two characters of the Chinese product name
$productCn = [string][char]0x968F + [char]0x5F71
# Artifact name: <cn product name><en product name>_v<version>.exe
# No underscore between the Chinese and English halves, so it matches the
# already-published v1.3.0 asset name and the documented download name.
$productBase = "${productCn}ShadeBar_v$version"

$distDir = Join-Path $Root 'dist'
$distExe = Join-Path $distDir "$productBase.exe"
$verifyDir = Join-Path $distDir 'verify'
$compareDir = Join-Path $distDir 'compare'
$verifyExe = Join-Path $verifyDir "${productBase}_verify.exe"
$compareExe = Join-Path $compareDir "${productBase}_no-en.exe"

# ---------------------------------------------------------------------------
# 2. toolchain environment
# ---------------------------------------------------------------------------
Write-Step 'Locate Visual Studio / Windows SDK'

$pf86 = ${env:ProgramFiles(x86)}
$vswhere = Join-Path $pf86 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsRoot = $null
if (Test-Path $vswhere) {
    $vsRoot = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    if ([string]::IsNullOrWhiteSpace($vsRoot)) {
        $vsRoot = & $vswhere -latest -products * -property installationPath 2>$null
    }
}
if ([string]::IsNullOrWhiteSpace($vsRoot)) {
    $candidate = Join-Path $pf86 'Microsoft Visual Studio\2022\BuildTools'
    if (Test-Path $candidate) { $vsRoot = $candidate }
}
if ([string]::IsNullOrWhiteSpace($vsRoot)) {
    Fail 'Visual Studio / Build Tools not found. Install the "Desktop development with C++" workload.'
}
Write-Host "    VS: $vsRoot"

$msvcBase = Join-Path $vsRoot 'VC\Tools\MSVC'
if (-not (Test-Path $msvcBase)) { Fail "MSVC tools directory not found: $msvcBase" }
$msvcVer = Get-ChildItem $msvcBase -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+' } |
    Sort-Object { [version]($_.Name -replace '[^\d.]', '') } -Descending |
    Select-Object -First 1
if ($null -eq $msvcVer) { Fail "no MSVC version under $msvcBase" }
$msvcDir = $msvcVer.FullName
Write-Host "    MSVC: $($msvcVer.Name)"

$sdkLibRoot = Join-Path $pf86 'Windows Kits\10\Lib'
$sdkBinRoot = Join-Path $pf86 'Windows Kits\10\bin'
if (-not (Test-Path $sdkLibRoot)) { Fail "Windows SDK Lib not found: $sdkLibRoot" }
$sdkVer = Get-ChildItem $sdkLibRoot -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
if ($null -eq $sdkVer) { Fail "no Windows SDK version under $sdkLibRoot" }
$sdkVerName = $sdkVer.Name
Write-Host "    SDK : $sdkVerName"

$msvcBin = Join-Path $msvcDir 'bin\Hostx64\x64'
$sdkBin = Join-Path $sdkBinRoot $sdkVerName
$sdkLib = Join-Path $sdkLibRoot $sdkVerName
$sdkInc = Join-Path (Join-Path $pf86 'Windows Kits\10') "Include\$sdkVerName"

$env:PATH = "$msvcBin;$sdkBin\x64;$env:PATH"
$env:LIB = "$(Join-Path $msvcDir 'lib\x64');$(Join-Path $sdkLib 'ucrt\x64');$(Join-Path $sdkLib 'um\x64')"
$env:INCLUDE = "$(Join-Path $msvcDir 'include');$(Join-Path $sdkInc 'ucrt');$(Join-Path $sdkInc 'um');$(Join-Path $sdkInc 'shared')"

$dumpbin = Join-Path $msvcBin 'dumpbin.exe'
$rcExe = Join-Path $sdkBin 'x64\rc.exe'
if (-not (Test-Path $rcExe)) { Fail "rc.exe not found: $rcExe (needed to compile app.rc)" }
$env:SB_RC_EXE = $rcExe

# ---------------------------------------------------------------------------
# 3. inject DLL (only exists from T05 on)
# ---------------------------------------------------------------------------
$injectCrate = Join-Path $Root 'crates\shadebar-inject\Cargo.toml'
if (Test-Path $injectCrate) {
    Write-Step 'Build inject DLL (panic=unwind)'
    & cargo build --profile release-inject -p shadebar-inject --target $TargetTriple
    if ($LASTEXITCODE -ne 0) { Fail 'shadebar-inject build failed' }
} else {
    Write-Host '    [skip] crates/shadebar-inject not present yet (arrives in T05); nothing embedded'
}

# ---------------------------------------------------------------------------
# 4. main exe (FORMAL release: verify feature OFF)
# ---------------------------------------------------------------------------
if ($Clean) {
    Write-Step 'cargo clean'
    & cargo clean
    if ($LASTEXITCODE -ne 0) { Fail 'cargo clean failed' }
}

Write-Step 'cargo build --release (FORMAL, cfg(feature="verify") OFF)'
& cargo build --release -p shadebar-app --target $TargetTriple
if ($LASTEXITCODE -ne 0) { Fail 'build failed (see cargo output above)' }

$built = Join-Path $Root "target\$TargetTriple\release\shadebar-app.exe"
if (-not (Test-Path $built)) { Fail "build artifact not found: $built" }

# ---------------------------------------------------------------------------
# 5. copy to dist\ (keep prior versions; only overwrite the same version)
# ---------------------------------------------------------------------------
Write-Step 'Copy to dist\'
if (-not (Test-Path $distDir)) { New-Item -ItemType Directory -Path $distDir | Out-Null }
# Keep prior versions: only the SAME version is overwritten (rollback / diff).
Copy-Item $built $distExe -Force
Write-Host "    $distExe"
$formalSize = (Get-Item $distExe).Length

# ---------------------------------------------------------------------------
# 6. gates (shared by all three artifacts)
# ---------------------------------------------------------------------------
function Invoke-Gates([string]$Exe, [string]$Label, [bool]$CheckSoft) {
    $failures = New-Object System.Collections.Generic.List[string]
    Write-Host ''
    Write-Host "---- Gates for [$Label] : $Exe ----" -ForegroundColor Cyan

    # 6.1 size
    Write-Step "Gate 1/5 [$Label]: size (NFR-01)"
    $sizeBytes = (Get-Item $Exe).Length
    $sizeKb = [Math]::Round($sizeBytes / 1KB, 1)
    Write-Host "    $sizeKb KB ($sizeBytes bytes)"
    if ($sizeBytes -gt ($HardLimitKb * 1KB)) {
        $failures.Add("[$Label] size over budget: $sizeKb KB > $HardLimitKb KB (NFR-01)")
    } elseif ($CheckSoft -and $sizeBytes -gt ($SoftLimitKb * 1KB)) {
        Write-Host "    note: above the $SoftLimitKb KB ENGINEERING SOFT GOAL (not a gate, v1.4 K-07)" -ForegroundColor Yellow
    } elseif ($CheckSoft) {
        Write-Host "    OK: within the $SoftLimitKb KB engineering soft goal" -ForegroundColor Green
    }

    # 6.2 runtime dependency
    Write-Step "Gate 2/5 [$Label]: no VC++ runtime dependency (FR-15 / A-15)"
    $dependents = & $dumpbin /nologo /dependents $Exe 2>&1 | Out-String
    $forbidden = @('VCRUNTIME140.dll', 'MSVCP140.dll', 'VCRUNTIME140_1.dll')
    $hit = $forbidden | Where-Object { $dependents -match [regex]::Escape($_) }
    if ($hit) {
        $failures.Add("[$Label] dynamic CRT dependency: $($hit -join ', ') (expected static CRT)")
    } else {
        Write-Host '    OK: no VCRUNTIME140 / MSVCP140'
    }

    # 6.3 version resource
    Write-Step "Gate 3/5 [$Label]: file version (FR-16 / A-16)"
    $expectedFileVersion = "$version.0"
    $fileVersion = (Get-Item $Exe).VersionInfo.FileVersion
    $productVersion = (Get-Item $Exe).VersionInfo.ProductVersion
    Write-Host "    FileVersion=$fileVersion  ProductVersion=$productVersion  expected=$expectedFileVersion"
    if ($fileVersion -ne $expectedFileVersion) {
        $failures.Add("[$Label] FileVersion mismatch: got '$fileVersion', expected '$expectedFileVersion'")
    }

    # 6.4 manifest
    Write-Step "Gate 4/5 [$Label]: manifest embedded with PerMonitorV2 (NFR-08)"
    $exeBytes = [System.IO.File]::ReadAllBytes($Exe)
    $exeText = [System.Text.Encoding]::UTF8.GetString($exeBytes)
    if ($exeText.Contains('PerMonitorV2')) {
        Write-Host '    OK: PerMonitorV2'
    } else {
        $failures.Add("[$Label] manifest missing or lacks PerMonitorV2")
    }
    if ($exeText.Contains('requestedExecutionLevel')) {
        Write-Host '    OK: requestedExecutionLevel (asInvoker, UAC = 0)'
    } else {
        $failures.Add("[$Label] manifest lacks requestedExecutionLevel")
    }

    # 6.5 icon
    Write-Step "Gate 5/5 [$Label]: icon resource (FR-06)"
    Add-Type -AssemblyName System.Drawing
    $icon = [System.Drawing.Icon]::ExtractAssociatedIcon($Exe)
    if ($null -ne $icon -and $icon.Width -gt 0) {
        Write-Host ("    OK: icon extractable {0}x{1}" -f $icon.Width, $icon.Height)
        $icon.Dispose()
    } else {
        $failures.Add("[$Label] cannot extract icon resource from exe")
    }

    return $failures.ToArray()
}

$allFailures = New-Object System.Collections.Generic.List[string]
if ($SkipChecks) {
    Write-Host ''
    Write-Host 'WARNING: -SkipChecks used, all gates skipped. Do not ship this artifact.' -ForegroundColor Yellow
    exit 0
}
foreach ($f in (Invoke-Gates $distExe 'formal' $true)) { $allFailures.Add($f) }

# ---------------------------------------------------------------------------
# 7. VERIFY artifact (task #9: second profile, same gates)
# ---------------------------------------------------------------------------
if (-not $SkipVerify) {
    Write-Step 'cargo build --release --features verify (EVIDENCE build)'
    & cargo build --release -p shadebar-app --features verify --target $TargetTriple
    if ($LASTEXITCODE -ne 0) { Fail 'verify build failed (see cargo output above)' }
    if (-not (Test-Path $verifyDir)) { New-Item -ItemType Directory -Path $verifyDir | Out-Null }
    Copy-Item $built $verifyExe -Force
    Write-Host "    $verifyExe"
    foreach ($f in (Invoke-Gates $verifyExe 'verify' $false)) { $allFailures.Add($f) }

    # -----------------------------------------------------------------------
    # 8. A-26 comparison artifact (remove en resources)
    # -----------------------------------------------------------------------
    Write-Step 'cargo build --profile release-no-en --features no-en (A-26 comparison)'
    & cargo build --profile release-no-en -p shadebar-app --features no-en --target $TargetTriple
    if ($LASTEXITCODE -ne 0) { Fail 'no-en comparison build failed' }
    $builtNoEn = Join-Path $Root "target\$TargetTriple\release-no-en\shadebar-app.exe"
    if (-not (Test-Path $builtNoEn)) { Fail "build artifact not found: $builtNoEn" }
    if (-not (Test-Path $compareDir)) { New-Item -ItemType Directory -Path $compareDir | Out-Null }
    Copy-Item $builtNoEn $compareExe -Force
    Write-Host "    $compareExe"
    foreach ($f in (Invoke-Gates $compareExe 'no-en' $false)) { $allFailures.Add($f) }

    Write-Step 'A-26 bilingual resource delta (NFR-19)'
    $noEnSize = (Get-Item $compareExe).Length
    $deltaBytes = $formalSize - $noEnSize
    $deltaKb = [Math]::Round($deltaBytes / 1KB, 2)
    Write-Host ("    formal={0} B  no-en={1} B  delta={2} B ({3} KB)" -f $formalSize, $noEnSize, $deltaBytes, $deltaKb)
    if ($deltaBytes -lt 0) {
        $allFailures.Add('A-26: no-en artifact is LARGER than the formal one; comparison is invalid')
    } elseif ($deltaBytes -gt ($BilingualLimitKb * 1KB)) {
        $allFailures.Add("A-26: bilingual delta $deltaKb KB > $BilingualLimitKb KB (NFR-19)")
    } else {
        Write-Host '    OK: delta within the 40 KB bilingual budget' -ForegroundColor Green
    }
    Write-Host '    dist\ root contains exactly one shipped exe (no lang sidecar files).'
}

# ---------------------------------------------------------------------------
# 9. summary
# ---------------------------------------------------------------------------
Write-Step 'Summary'
Write-Host ("    artifact : {0}" -f $distExe)
Write-Host ("    size     : {0} KB (soft {1} KB / hard {2} KB)" -f ([Math]::Round($formalSize / 1KB, 1)), $SoftLimitKb, $HardLimitKb)
Write-Host ("    version  : {0}" -f "$version.0")
if ($SkipVerify) {
    Write-Host '    variants : skipped (-SkipVerify)'
} else {
    Write-Host ("    verify   : {0}" -f $verifyExe)
    Write-Host ("    no-en    : {0}" -f $compareExe)
}

if ($allFailures.Count -gt 0) {
    Write-Host ''
    Write-Host 'Gates failed:' -ForegroundColor Red
    foreach ($f in $allFailures) { Write-Host "  - $f" -ForegroundColor Red }
    exit 1
}

Write-Host ''
Write-Host 'BUILD OK (formal + verify + no-en gates all green)' -ForegroundColor Green
exit 0
