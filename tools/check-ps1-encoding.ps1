# ShadeBar - PS1 encoding guard (ASCII only, must stay ASCII)
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tools\check-ps1-encoding.ps1
#
# Why a separate script:
#   Windows PowerShell 5.1 parses the WHOLE script before executing anything. If a
#   .ps1 contains non-ASCII text and was saved WITHOUT a UTF-8 BOM, PowerShell 5.1
#   decodes it with the local ANSI code page, which can break the parser itself:
#      - param() block silently swallowed
#      - $PSCommandPath empty
#      - "string is missing the terminator" parse errors
#   No in-script self check can survive that, because the parser runs first.
#   Therefore the guard MUST be a separate, pure-ASCII script (this one).
#
# Rule enforced: every *.ps1 tracked in this repo must be either
#   (a) pure ASCII, or
#   (b) UTF-8 with BOM.
# Anything else is a latent, silent failure.
#
# Exit codes: 0 = all good, 2 = violations found, 3 = usage/internal error.

[CmdletBinding()]
param(
    [string]$Root = ''
)

$ErrorActionPreference = 'Stop'

try {
    if ([string]::IsNullOrEmpty($Root)) {
        $Root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
    }
    if (-not (Test-Path $Root)) {
        Write-Host "ERROR: root not found: $Root" -ForegroundColor Red
        exit 3
    }

    $skipDirs = @('target', '.git', 'node_modules', 'dist', 'shots', '.workbuddy')
    $scripts = Get-ChildItem -Path $Root -Filter '*.ps1' -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
            $rel = $_.FullName.Substring($Root.Length).TrimStart('\', '/')
            $skip = $false
            foreach ($d in $skipDirs) {
                if ($rel -like "$d\*" -or $rel -like "*\$d\*") { $skip = $true; break }
            }
            -not $skip
        }

    $violations = New-Object System.Collections.Generic.List[string]
    foreach ($f in $scripts) {
        $bytes = [System.IO.File]::ReadAllBytes($f.FullName)
        $hasBom = ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)

        $hasNonAscii = $false
        foreach ($b in $bytes) {
            if ($b -gt 0x7F) { $hasNonAscii = $true; break }
        }

        if ($hasNonAscii -and -not $hasBom) {
            $rel = $f.FullName.Substring($Root.Length).TrimStart('\', '/')
            $violations.Add($rel)
        }
    }

    Write-Host ("[check-ps1-encoding] scanned {0} .ps1 file(s)" -f $scripts.Count)
    if ($violations.Count -eq 0) {
        Write-Host '[check-ps1-encoding] OK: every non-ASCII script has a UTF-8 BOM' -ForegroundColor Green
        exit 0
    }

    Write-Host ''
    Write-Host '[check-ps1-encoding] VIOLATION: non-ASCII characters without UTF-8 BOM:' -ForegroundColor Red
    foreach ($v in $violations) {
        Write-Host ("  - {0}" -f $v) -ForegroundColor Red
    }
    Write-Host ''
    Write-Host 'These files will be mis-parsed by Windows PowerShell 5.1 (param block lost,' -ForegroundColor Red
    Write-Host '$PSCommandPath empty, possible parse errors). Fix by re-saving as UTF-8 with BOM:' -ForegroundColor Red
    Write-Host '  $p="<file>"; $t=[IO.File]::ReadAllText($p,[Text.Encoding]::UTF8);' -ForegroundColor Red
    Write-Host '  [IO.File]::WriteAllText($p,$t,(New-Object Text.UTF8Encoding($true)))' -ForegroundColor Red
    Write-Host ''
    Write-Host 'Or simply remove all non-ASCII characters from the script (preferred for scripts).' -ForegroundColor Red
    exit 2
}
catch {
    Write-Host ("ERROR: {0}" -f $_.Exception.Message) -ForegroundColor Red
    exit 3
}
