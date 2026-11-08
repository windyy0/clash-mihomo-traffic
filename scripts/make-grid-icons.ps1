# Generates the project's own traffic-grid mark using Windows drawing APIs.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$iconRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'assets'

function New-RoundedPath([float]$x, [float]$y, [float]$width, [float]$height, [float]$radius) {
    $path = [Drawing.Drawing2D.GraphicsPath]::new()
    $diameter = 2 * $radius
    $path.AddArc($x, $y, $diameter, $diameter, 180, 90)
    $path.AddArc($x + $width - $diameter, $y, $diameter, $diameter, 270, 90)
    $path.AddArc($x + $width - $diameter, $y + $height - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc($x, $y + $height - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    return ,$path
}

function New-GridBitmap([int]$size, [string]$variant) {
    $large = [Drawing.Bitmap]::new($size * 4, $size * 4)
    $graphics = [Drawing.Graphics]::FromImage($large)
    $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.ScaleTransform($size / 64.0, $size / 64.0)
    try {
        if ($variant -eq 'traffic') {
            $tile = New-RoundedPath 8 8 240 240 56
            $gradient = [Drawing.Drawing2D.LinearGradientBrush]::new(
                [Drawing.PointF]::new(0, 0), [Drawing.PointF]::new(256, 256),
                [Drawing.ColorTranslator]::FromHtml('#D65B31'), [Drawing.ColorTranslator]::FromHtml('#C13E6F'))
            try { $graphics.FillPath($gradient, $tile) } finally { $gradient.Dispose(); $tile.Dispose() }
            $bars = @(@(52,116), @(94,66), @(136,96), @(178,46))
            $barWidth = 26
            $baseline = 194
        } else {
            # Enlarge the transparent glyph optically for 16-DIP notification icons.
            $bars = @(@(32,128), @(84,56), @(136,96), @(188,24))
            $barWidth = 32
            $baseline = 232
        }
        for ($index = 0; $index -lt $bars.Count; $index++) {
            $color = if ($variant -eq 'tray-dark') { '#171719' } elseif ($variant -eq 'traffic' -and $index % 2) { '#FFE1C6' } else { '#FFFFFF' }
            $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml($color))
            $path = New-RoundedPath $bars[$index][0] $bars[$index][1] $barWidth ($baseline - $bars[$index][1]) ($barWidth / 2)
            try { $graphics.FillPath($brush, $path) } finally { $brush.Dispose(); $path.Dispose() }
        }
        $bitmap = [Drawing.Bitmap]::new($size, $size)
        $resampler = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $resampler.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $resampler.DrawImage($large, 0, 0, $size, $size)
        } finally { $resampler.Dispose() }
        return ,$bitmap
    } finally { $graphics.Dispose(); $large.Dispose() }
}

foreach ($variant in @('traffic', 'tray-light', 'tray-dark')) {
    $preview = New-GridBitmap 512 $variant
    try { $preview.Save((Join-Path $iconRoot "$variant.png"), [Drawing.Imaging.ImageFormat]::Png) } finally { $preview.Dispose() }
    $frames = foreach ($size in @(16, 20, 24, 32, 40, 48, 64, 128, 256)) {
        $bitmap = New-GridBitmap $size $variant
        $buffer = [IO.MemoryStream]::new()
        try {
            $bitmap.Save($buffer, [Drawing.Imaging.ImageFormat]::Png)
            @{ Size = $size; Bytes = $buffer.ToArray() }
        } finally { $bitmap.Dispose(); $buffer.Dispose() }
    }
    $stream = [IO.File]::Create((Join-Path $iconRoot "$variant.ico"))
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$frames.Count)
        $offset = 6 + 16 * $frames.Count
        foreach ($frame in $frames) {
            $dimension = if ($frame.Size -eq 256) { 0 } else { $frame.Size }
            $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
            $writer.Write([byte]0); $writer.Write([byte]0)
            $writer.Write([uint16]1); $writer.Write([uint16]32)
            $writer.Write([uint32]$frame.Bytes.Length); $writer.Write([uint32]$offset)
            $offset += $frame.Bytes.Length
        }
        foreach ($frame in $frames) { $writer.Write([byte[]]$frame.Bytes) }
    } finally { $writer.Dispose(); $stream.Dispose() }
}
