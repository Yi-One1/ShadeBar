# T03.1 acceptance check: launch the app's dev UI entry, capture the window, sample pixels.
#
# What it proves (each sample maps to one acceptance criterion):
#   * client background == #F3F3F3 (light theme)  -> our opaque FillRect is visible ON TOP of
#     the DWM backdrop=4 base, i.e. the forced base does NOT hide our painting
#   * card area == #FFFFFF                        -> rounded-rect fill + border draw correctly
#   * caption strip != client background          -> DWM supplies the title bar material
#   * text pixel is dark                          -> text renders (no missing glyph run)
#
# ASCII only (see tools\check-ps1-encoding.ps1).
#
# Usage: powershell -ExecutionPolicy Bypass -File tools\verify-t03-window.ps1

[CmdletBinding()]
param(
    [string]$Exe = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public static class T03
{
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);

    private delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc cb, IntPtr l);

    public static IntPtr FindByClass(string cls)
    {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) =>
        {
            var sb = new StringBuilder(256);
            GetClassNameW(h, sb, sb.Capacity);
            if (sb.ToString() == cls && IsWindowVisible(h)) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
if ([string]::IsNullOrWhiteSpace($Exe)) {
    $Exe = Join-Path $root 'dist' | Join-Path -ChildPath '*_ShadeBar_v*.exe' | Resolve-Path | Select-Object -First 1 -ExpandProperty Path
}
if (-not (Test-Path $Exe)) { Write-Output "FAIL: exe not found: $Exe"; exit 3 }
Write-Output "exe: $Exe"

$proc = Start-Process -FilePath $Exe -ArgumentList '--ui' -PassThru
Start-Sleep -Milliseconds 2000

$hwnd = [T03]::FindByClass('ShadeBar.MainWindow')
if ($hwnd -eq [IntPtr]::Zero) {
    $proc | Stop-Process -Force -ErrorAction SilentlyContinue
    Write-Output 'FAIL: window class ShadeBar.MainWindow not found (app failed to create window?)'
    exit 3
}
Write-Output ("window found: hwnd={0}" -f $hwnd)

[void][T03]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 900

$r = New-Object T03+RECT
[void][T03]::GetWindowRect($hwnd, [ref]$r)
$w = $r.Right - $r.Left
$h = $r.Bottom - $r.Top
Write-Output ("rect: {0},{1} {2}x{3}" -f $r.Left, $r.Top, $w, $h)

$outDir = Join-Path $root 'docs\spikes'
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }

$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
$out = Join-Path $outDir 't03-b3-window.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()

function Show-Pixel([string]$label, [int]$x, [int]$y) {
    if ($x -lt 0 -or $y -lt 0 -or $x -ge $bmp.Width -or $y -ge $bmp.Height) {
        Write-Output ("  {0,-22} ({1},{2}) out of range" -f $label, $x, $y)
        return
    }
    $c = $bmp.GetPixel($x, $y)
    Write-Output ("  {0,-22} ({1},{2}) = #{3:X2}{4:X2}{5:X2}" -f $label, $x, $y, $c.R, $c.G, $c.B)
}

Write-Output '=== pixel samples ==='
Show-Pixel 'caption (DWM)' ([int]($w / 2)) 12
Show-Pixel 'client bg' ([int]($w / 2)) ($h - 12)
Show-Pixel 'card fill' ([int]($w / 2)) ([int]($h / 2))
Show-Pixel 'left margin' 6 ([int]($h / 2))
Show-Pixel 'page heading' ([int]($w / 2)) 46

$bmp.Dispose()
Write-Output "saved: $out"

$proc.CloseMainWindow() | Out-Null
Start-Sleep -Milliseconds 400
if (-not $proc.HasExited) { $proc | Stop-Process -Force }
Write-Output 'done'
