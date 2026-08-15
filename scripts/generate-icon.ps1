# 生成 1024x1024 的应用图标（assets/app-icon.png），随后用 `npm run icons` 生成全套 Tauri 图标
Add-Type -AssemblyName System.Drawing

$size = 1024
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::FromArgb(255, 10, 14, 22))

# 渐变背景
$rect = New-Object System.Drawing.Rectangle(0, 0, $size, $size)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(255, 32, 46, 72),
    [System.Drawing.Color]::FromArgb(255, 8, 12, 22),
    45.0)
$g.FillRectangle($brush, $rect)

# 青色圆环（Harness 标识）
$pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(255, 76, 194, 255), 64)
$g.DrawEllipse($pen, 160, 160, 704, 704)

# 内部深色圆
$innerBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 16, 24, 38))
$g.FillEllipse($innerBrush, 300, 300, 424, 424)

# 中心亮点
$dotBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 76, 194, 255))
$g.FillEllipse($dotBrush, 452, 452, 120, 120)

$dir = Join-Path $PSScriptRoot "..\assets"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$out = Join-Path $dir "app-icon.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)

$g.Dispose(); $bmp.Dispose(); $pen.Dispose(); $brush.Dispose(); $innerBrush.Dispose(); $dotBrush.Dispose()
Write-Host "icon written: $out"
