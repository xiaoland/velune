# Draws Velune's Windows icon from the logo pack: each size from the optical
# master meant for it (small below 32 px, medium below 96 px, large above),
# in the theme colorway, which reads on light and dark taskbars alike. Windows
# icons carry no backplate, so the mark stands alone on a transparent canvas.
param(
    [Parameter(Mandatory)] [string] $Brand,
    [Parameter(Mandatory)] [string] $Output
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore, WindowsBase

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$frames = foreach ($size in $sizes) {
    $master = if ($size -lt 32) { 'small' } elseif ($size -lt 96) { 'medium' } else { 'large' }
    [xml] $svg = Get-Content -Raw -Encoding UTF8 (Join-Path $Brand "velune-$master-theme.svg")
    $paths = @($svg.svg.path)
    if ($paths.Count -ne 2) { throw "velune-$master-theme.svg is not a two-path optical master" }
    $visual = [System.Windows.Media.DrawingVisual]::new()
    $context = $visual.RenderOpen()
    # The masters share a 1024-unit canvas; small sizes keep a pixel of margin.
    $inset = if ($size -le 32) { 0.5 } else { $size * 0.03 }
    $scale = ($size - 2 * $inset) / 1024
    $context.PushTransform([System.Windows.Media.MatrixTransform]::new($scale, 0, 0, $scale, $inset, $inset))
    foreach ($path in $paths) {
        $brush = [System.Windows.Media.SolidColorBrush]::new([System.Windows.Media.ColorConverter]::ConvertFromString($path.fill))
        $context.DrawGeometry($brush, $null, [System.Windows.Media.Geometry]::Parse($path.d))
    }
    $context.Close()
    $bitmap = [System.Windows.Media.Imaging.RenderTargetBitmap]::new($size, $size, 96, 96, [System.Windows.Media.PixelFormats]::Pbgra32)
    $bitmap.Render($visual)
    $encoder = [System.Windows.Media.Imaging.PngBitmapEncoder]::new()
    $encoder.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($bitmap))
    $stream = [System.IO.MemoryStream]::new()
    $encoder.Save($stream)
    [pscustomobject]@{ Size = $size; Png = $stream.ToArray() }
}

# An .ico of PNG frames: a header, one directory entry per frame, then the frames.
$file = [System.IO.MemoryStream]::new()
$writer = [System.IO.BinaryWriter]::new($file)
$writer.Write([uint16] 0); $writer.Write([uint16] 1); $writer.Write([uint16] $frames.Count)
$offset = 6 + 16 * $frames.Count
foreach ($frame in $frames) {
    $side = if ($frame.Size -ge 256) { 0 } else { $frame.Size }
    $writer.Write([byte] $side); $writer.Write([byte] $side); $writer.Write([byte] 0); $writer.Write([byte] 0)
    $writer.Write([uint16] 1); $writer.Write([uint16] 32)
    $writer.Write([uint32] $frame.Png.Length); $writer.Write([uint32] $offset)
    $offset += $frame.Png.Length
}
foreach ($frame in $frames) { $writer.Write($frame.Png) }
$writer.Flush()
New-Item -ItemType Directory -Force (Split-Path $Output -Parent) | Out-Null
[System.IO.File]::WriteAllBytes($Output, $file.ToArray())
