# 随影 ShadeBar — 应用程序图标生成器
#
# 已被 gen_app_ico.py 取代，仅作溯源：本环境 Add-Type -AssemblyName System.Drawing 被安全策略拦截，脚本跑不起来。
# 现行生成方式：python3 resources/gen_app_ico.py（纯标准库，方案 A 对角对分）。
# 用途：生成 resources/app.ico（§4 #6）。图标是二进制资产，必须保留生成源，
#       否则以后无法一致地重做（改配色、加尺寸）。
#
# 设计（docs/02-architecture.md D11）：
#   竖向对分的圆角方形 —— 左半 #1A1A1A，右半 #FFFFFF，中间留一条竖向透空缝隙。
#   与 TranslucentTB（横向任务栏条 + 弧形高光）在"轮廓"（方 vs 横条）与
#   "分割方向"（竖 vs 横）上均不同，满足 A-06 / FR-06。
#   外圈描边取中性灰 #595959 —— 深浅背景下都能看出边界。
#
# 输出：多尺寸 PNG 压缩 ICO（16/24/32/48/256）。
#
# 注意：本文件必须保存为 UTF-8 **带 BOM**，否则 Windows PowerShell 5.1 会按
#       本地 ANSI 代码页解析中文注释，导致 param 块被吞掉且 $PSCommandPath 为空。
#
# 用法：powershell -ExecutionPolicy Bypass -File resources\gen-app-ico.ps1

[CmdletBinding()]
param(
    [string]$OutFile = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

if ([string]::IsNullOrEmpty($OutFile)) {
    $OutFile = Join-Path (Split-Path -Parent $PSCommandPath) 'app.ico'
}

$sizes = @(16, 24, 32, 48, 256)

# 单个尺寸的位图 -> PNG 字节数组
function New-IconPng {
    param([int]$Size)

    $bmp = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $path = $null
    $pen = $null
    $darkBrush = $null
    $lightBrush = $null
    $ms = $null

    try {
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.Clear([System.Drawing.Color]::Transparent)

        # 几何比例全部按边长归一，保证各尺寸视觉一致
        $inset  = [Math]::Max(0.0, [double]$Size * 0.035)
        $radius = [Math]::Max(1.0, [double]$Size * 0.22)
        $gap    = [Math]::Max(1.0, [double]$Size * 0.075)   # 竖向透空缝隙宽度
        $stroke = [Math]::Max(1.0, [double]$Size * 0.055)   # 外圈描边宽度

        $left = [single]$inset
        $top = [single]$inset
        $side = [single]($Size - 2.0 * $inset)
        $d = [single]($radius * 2.0)

        $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
        $path.AddArc($left, $top, $d, $d, 180.0, 90.0)
        $path.AddArc([single]($left + $side - $d), $top, $d, $d, 270.0, 90.0)
        $path.AddArc([single]($left + $side - $d), [single]($top + $side - $d), $d, $d, 0.0, 90.0)
        $path.AddArc($left, [single]($top + $side - $d), $d, $d, 90.0, 90.0)
        $path.CloseFigure()

        # 只画在圆角矩形内部
        $g.SetClip($path)

        $darkBrush  = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 0x1A, 0x1A, 0x1A))
        $lightBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(255, 0xFF, 0xFF, 0xFF))

        $midLeft  = [single](([double]$Size - $gap) / 2.0)
        $midRight = [single](([double]$Size + $gap) / 2.0)

        $g.FillRectangle($darkBrush, [single]0.0, [single]0.0, $midLeft, [single]$Size)
        $g.FillRectangle($lightBrush, $midRight, [single]0.0, [single]($Size - $midRight), [single]$Size)

        $g.ResetClip()

        # 外圈描边（中性灰，深浅背景都有边界）
        $pen = [System.Drawing.Pen]::new([System.Drawing.Color]::FromArgb(255, 0x59, 0x59, 0x59), [single]$stroke)
        $pen.Alignment = [System.Drawing.Drawing2D.PenAlignment]::Inset
        $g.DrawPath($pen, $path)

        $ms = [System.IO.MemoryStream]::new()
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bytes = $ms.ToArray()
        # `, $bytes` 让字节数组整体作为一个对象返回，避免被展开成 int 流
        return , $bytes
    }
    finally {
        if ($null -ne $ms) { $ms.Dispose() }
        if ($null -ne $pen) { $pen.Dispose() }
        if ($null -ne $darkBrush) { $darkBrush.Dispose() }
        if ($null -ne $lightBrush) { $lightBrush.Dispose() }
        if ($null -ne $path) { $path.Dispose() }
        $g.Dispose()
        $bmp.Dispose()
    }
}

# ---- 组装 ICO 容器 ----
# 用 List[byte[]] 强制元素类型为 byte[]，避免 Object[] 在 BinaryWriter 上没有匹配重载。
$pngs = [System.Collections.Generic.List[byte[]]]::new()
foreach ($s in $sizes) {
    $pngs.Add([byte[]](New-IconPng -Size $s))
}

$fs = [System.IO.File]::Create($OutFile)
$bw = [System.IO.BinaryWriter]::new($fs)
try {
    $bw.Write([UInt16]0)               # reserved
    $bw.Write([UInt16]1)               # type = 1 (icon)
    $bw.Write([UInt16]$pngs.Count)     # image count

    $offset = 6 + 16 * $pngs.Count
    for ($i = 0; $i -lt $pngs.Count; $i++) {
        $s = $sizes[$i]
        if ($s -ge 256) { $dim = 0 } else { $dim = $s }   # 256 用 0 表示
        $bw.Write([Byte]$dim)          # width
        $bw.Write([Byte]$dim)          # height
        $bw.Write([Byte]0)             # color count
        $bw.Write([Byte]0)             # reserved
        $bw.Write([UInt16]1)           # planes
        $bw.Write([UInt16]32)          # bit count
        $bw.Write([UInt32]$pngs[$i].Length)
        $bw.Write([UInt32]$offset)
        $offset += $pngs[$i].Length
    }

    foreach ($png in $pngs) { $bw.Write($png) }
}
finally {
    $bw.Dispose()
    $fs.Dispose()
}

$len = (Get-Item $OutFile).Length
Write-Output ("[gen-app-ico] wrote {0} ({1} bytes, sizes: {2})" -f $OutFile, $len, ($sizes -join '/'))
