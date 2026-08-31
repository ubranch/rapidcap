# Generates crates/desktop/assets/rapidcap.ico from the product mark.
#
# The mark is the same one the panel header draws (crates/desktop/src/window.rs):
# a near-white rounded tile with a dark ring punched through the middle. Ratios
# come straight from the design system - a 34px tile, 10px corner radius, a 20px
# ring at 3px stroke - so the exe icon, the taskbar icon and the on-screen logo
# are one drawing at three sizes rather than three drawings that drift apart.
#
# Run after changing the mark:  pwsh -File scripts/make-icon.ps1

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$TILE = [System.Drawing.Color]::FromArgb(255, 0xFC, 0xFC, 0xFC)
$RING = [System.Drawing.Color]::FromArgb(255, 0x11, 0x11, 0x11)

function New-Mark([int] $s) {
    $bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)

    # Rounded tile. Four arcs, because GDI+ has no rounded-rectangle primitive.
    $d = [single](2.0 * 10.0 / 34.0 * $s)
    $edge = [single]$s
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc([single]0, [single]0, $d, $d, [single]180, [single]90)
    $path.AddArc($edge - $d, [single]0, $d, $d, [single]270, [single]90)
    $path.AddArc($edge - $d, $edge - $d, $d, $d, [single]0, [single]90)
    $path.AddArc([single]0, $edge - $d, $d, $d, [single]90, [single]90)
    $path.CloseFigure()
    $g.FillPath((New-Object System.Drawing.SolidBrush($TILE)), $path)

    # The ring: a dark disc with a tile-coloured disc on top. Below ~20px the
    # true 3/34 stroke lands under a pixel and the ring greys out into a smudge,
    # so it gets a floor.
    $outer = [single](20.0 / 34.0 * $s)
    $stroke = [single][Math]::Max(1.5, 3.0 / 34.0 * $s)
    $inner = [single]($outer - 2.0 * $stroke)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush($RING)), ($edge - $outer) / 2, ($edge - $outer) / 2, $outer, $outer)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush($TILE)), ($edge - $inner) / 2, ($edge - $inner) / 2, $inner, $inner)

    $g.Dispose()
    $path.Dispose()
    $bmp
}

# A BITMAPINFOHEADER icon image: rows bottom-up, BGRA, then an all-zero AND mask
# that the 32bpp alpha channel makes redundant but the format still demands.
function Get-BmpPayload($bmp) {
    $s = $bmp.Width
    $rect = New-Object System.Drawing.Rectangle(0, 0, $s, $s)
    $locked = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $raw = New-Object byte[] ($locked.Stride * $s)
    [System.Runtime.InteropServices.Marshal]::Copy($locked.Scan0, $raw, 0, $raw.Length)
    $stride = $locked.Stride
    $bmp.UnlockBits($locked)

    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter($ms)
    $bw.Write([uint32]40)
    $bw.Write([int32]$s)
    $bw.Write([int32]($s * 2))   # height counts the XOR and AND planes together
    $bw.Write([uint16]1)
    $bw.Write([uint16]32)
    $bw.Write([uint32]0)
    $bw.Write([uint32]($s * $s * 4))
    $bw.Write([int32]0); $bw.Write([int32]0); $bw.Write([uint32]0); $bw.Write([uint32]0)
    for ($y = $s - 1; $y -ge 0; $y--) { $bw.Write($raw, $y * $stride, $s * 4) }
    $maskStride = ((([int][Math]::Ceiling($s / 8.0)) + 3) -band -4)
    $bw.Write((New-Object byte[] ($maskStride * $s)))
    $bw.Flush()
    # Leading comma, or PowerShell unrolls the array into 1128 loose objects on
    # the way out and the caller writes a mangled payload.
    , $ms.ToArray()
}

# 16 through 64 as raw bitmaps because that is what every shell surface asks for
# first; 128 and 256 as PNG so the file stays under 100 KB.
$images = foreach ($size in 16, 20, 24, 32, 40, 48, 64, 128, 256) {
    $bmp = New-Mark $size
    if ($size -ge 128) {
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $data = $ms.ToArray()
    } else {
        $data = Get-BmpPayload $bmp
    }
    $bmp.Dispose()
    @{ Size = $size; Data = [byte[]]$data }
}

$out = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($out)
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($image in $images) {
    $dim = if ($image.Size -ge 256) { 0 } else { $image.Size }  # 0 means 256
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$image.Data.Length); $bw.Write([uint32]$offset)
    $offset += $image.Data.Length
}
foreach ($image in $images) { $bw.Write($image.Data) }
$bw.Flush()

$assets = Join-Path (Split-Path -Parent $PSScriptRoot) 'crates/desktop/assets'
$target = Join-Path $assets 'rapidcap.ico'
[System.IO.File]::WriteAllBytes($target, $out.ToArray())
Write-Output "$target  $($out.Length) bytes  $($images.Count) sizes"

# --- rapidcap.icns -----------------------------------------------------------
#
# The same mark again, on Apple's icon grid: an 824pt body centred in a 1024pt
# canvas. Windows icons are drawn edge to edge and the shell insets them; macOS
# expects the margin to be part of the artwork, and an edge-to-edge icon simply
# looks a size too big next to everything else in the Dock.

function New-MacMark([int] $s) {
    $body = [int][Math]::Round($s * 824.0 / 1024.0)
    $mark = New-Mark $body
    $bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::Transparent)
    # Drawn at its native size rather than scaled, so the ring keeps the edge
    # New-Mark antialiased for it.
    $g.DrawImageUnscaled($mark, [int](($s - $body) / 2), [int](($s - $body) / 2))
    $g.Dispose()
    $mark.Dispose()
    $bmp
}

# Every slot macOS looks in, from the menu bar to a 5K Dock. The @2x types are
# the same pixels as the 1x type of that size - ic13 is 256 and so is ic08 - so
# the renders are cached and only the four-character type code differs.
$slots = @(
    @{ Tag = 'icp4'; Size = 16 }, @{ Tag = 'icp5'; Size = 32 }
    @{ Tag = 'ic11'; Size = 32 }, @{ Tag = 'ic12'; Size = 64 }
    @{ Tag = 'ic07'; Size = 128 }, @{ Tag = 'ic13'; Size = 256 }
    @{ Tag = 'ic08'; Size = 256 }, @{ Tag = 'ic14'; Size = 512 }
    @{ Tag = 'ic09'; Size = 512 }, @{ Tag = 'ic10'; Size = 1024 }
)

$renders = @{}
$body = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($body)
foreach ($slot in $slots) {
    if (-not $renders.ContainsKey($slot.Size)) {
        $bmp = New-MacMark $slot.Size
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $renders[$slot.Size] = $ms.ToArray()
    }
    $data = $renders[$slot.Size]
    $bw.Write([System.Text.Encoding]::ASCII.GetBytes($slot.Tag))
    # Big-endian, and it counts the eight header bytes as well as the payload.
    $bw.Write([System.Buffers.Binary.BinaryPrimitives]::ReverseEndianness([int32]($data.Length + 8)))
    $bw.Write($data)
}
$bw.Flush()

$icns = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($icns)
$bw.Write([System.Text.Encoding]::ASCII.GetBytes('icns'))
$bw.Write([System.Buffers.Binary.BinaryPrimitives]::ReverseEndianness([int32]($body.Length + 8)))
$bw.Write($body.ToArray())
$bw.Flush()

$target = Join-Path $assets 'rapidcap.icns'
[System.IO.File]::WriteAllBytes($target, $icns.ToArray())
Write-Output "$target  $($icns.Length) bytes  $($slots.Count) slots"
