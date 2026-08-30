# Pixel conformance check for the RapidCap panel.
#
# Captures the running window, *measures* every edge, and compares the result
# with docs/design-system/screens/main-window.html. Nothing here assumes an
# offset: a hard-coded probe point tests the script, not the app.
#
# Usage:  pwsh scripts/verify-design.ps1
# Exit:   0 all checks pass, 1 otherwise.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class RcWin {
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int c, uint flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int index);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@ -ErrorAction SilentlyContinue

$proc = Get-Process -Name RapidCap -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Host "FAIL  RapidCap is not running"; exit 1 }
$h = [IntPtr]$proc.MainWindowHandle
if (-not [RcWin]::IsWindowVisible($h)) { Write-Host "FAIL  window is not visible"; exit 1 }

# Everything below measures a screen grab, which is in *physical* pixels, while
# the design is specified in *logical* ones. On a 100% display those are the
# same number and the difference never shows. At 125% every band is 1.25x the
# size the design names, and this script used to report ten failures against an
# app that was laying out correctly.
#
# The awareness is pinned rather than inherited: loading System.Drawing flips
# the process into per-monitor mode on its own, so an un-pinned run measures
# physical or logical depending on what happened to be loaded first.
$PER_MONITOR_V2 = [IntPtr](-4)
[void][RcWin]::SetThreadDpiAwarenessContext($PER_MONITOR_V2)
$scale = [RcWin]::GetDpiForWindow($h) / 96.0

# Logical -> physical, for a coordinate the design gives us.
function Px([double]$logical) { [int][Math]::Round($logical * $scale) }
# Physical -> logical, for a distance we measured off the bitmap.
function Lg([double]$physical) { $physical / $scale }

# Capture by moving the panel to a clear patch and pinning it topmost, rather
# than by activating it. A background console cannot take the foreground -
# Windows' foreground lock denies it - so SetForegroundWindow leaves whatever
# the user has open on top and the grab returns their desktop, not the panel.
$before = New-Object RcWin+RECT
[void][RcWin]::GetWindowRect($h, [ref]$before)
$TOPMOST = [IntPtr](-1)
$NOSIZE_NOACTIVATE = 0x0001 -bor 0x0010
[void][RcWin]::SetWindowPos($h, $TOPMOST, 20, 60, 0, 0, $NOSIZE_NOACTIVATE)
Start-Sleep -Milliseconds 900

$r = New-Object RcWin+RECT
[void][RcWin]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)
$W = $r.R - $r.L
$H = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap($W, $H)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
$g.Dispose()
$style = [int64][RcWin]::GetWindowLongPtrW($h, -16)
[void][RcWin]::SetWindowPos($h, $TOPMOST, $before.L, $before.T, 0, 0, $NOSIZE_NOACTIVATE)

$script:pass = 0
$script:fail = 0
function Hex([int]$x, [int]$y) { $c = $bmp.GetPixel($x, $y); '#{0:X2}{1:X2}{2:X2}' -f $c.R, $c.G, $c.B }

function Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) { $script:pass++; Write-Host ("  PASS  {0}" -f $name) }
  else { $script:fail++; Write-Host ("  FAIL  {0} - {1}" -f $name, $detail) -ForegroundColor Red }
}
function Expect([string]$name, $got, $want) {
  Check $name ($got -eq $want) "expected $want, measured $got"
}
# The DWM frame rect includes a resize border whose exact width Windows decides,
# so window-derived totals are allowed one pixel either way. Everything measured
# *inside* the client area is exact.
function ExpectNear([string]$name, $got, $want) {
  Check $name ([Math]::Abs($got - $want) -le 1) "expected $want +/-1, measured $got"
}
# Measured physical distance against a logical design value. Rounding is done
# per element by GPUI, so a logical pixel of slack is the honest tolerance.
function ExpectLogical([string]$name, $physical, $want) {
  $got = [Math]::Round((Lg $physical), 1)
  Check $name ([Math]::Abs($got - $want) -le 1) "expected $want logical px +/-1, measured $got ($physical physical)"
}

# --- measure, never assume -------------------------------------------------

# First row from the top whose centre pixel is the titlebar tone.
$top = 0
while ($top -lt $H -and (Hex ([int]($W / 2)) $top) -ne '#1C1C1C') { $top++ }

# Titlebar runs until the body tone appears. Sampled mid-width: the rounded
# corners are desktop pixels, not window pixels.
$y = $top
$midX = [int]($W / 2)
while ($y -lt $H -and (Hex $midX $y) -ne '#111111') { $y++ }
$titlebarH = $y - $top
$bodyTop = $y

# Walk a column inside the left card, well clear of the centred icon and label.
# x=196 lands in the 9px gap between the two cards; x=30 is 18px into the card.
# Stated in design (logical) pixels and converted, so the probe lands in the
# same place on a 100% and a 125% display.
$col = Px 30
# Card and chip bands are the runs of card tone bounded by body tone.
$bands = @()
$inBand = $false
$start = 0
for ($y = $bodyTop; $y -lt $H - 2; $y++) {
  $isCard = (Hex $col $y) -ne '#111111'
  if ($isCard -and -not $inBand) { $inBand = $true; $start = $y }
  elseif (-not $isCard -and $inBand) { $inBand = $false; $bands += [pscustomobject]@{ Top = $start; Height = $y - $start } }
}

Write-Host ""
Write-Host "RapidCap panel - pixel conformance" -ForegroundColor Cyan
Write-Host ("  window {0} x {1} physical at {2}x scale, client top at y={3}" -f $W, $H, $scale, $top)
Write-Host ""

Write-Host "Geometry"
# The 2px is the physical resize border, subtracted before converting.
ExpectLogical "panel width 400" ($W - 2) 400
ExpectLogical "titlebar height 44" $titlebarH 44
# The header contributes bands of its own (badge, segmented track), so the two
# card rows and the footer are the last three.
Check "at least three bands found" ($bands.Count -ge 3) "found $($bands.Count)"
if ($bands.Count -ge 3) {
  $bands = $bands[-3..-1]
  ExpectLogical "card row 1 height 64" $bands[0].Height 64
  ExpectLogical "card row 2 height 64" $bands[1].Height 64
  ExpectLogical "gap between card rows 9" ($bands[1].Top - ($bands[0].Top + $bands[0].Height)) 9
  ExpectLogical "gap card row 2 to footer 9" ($bands[2].Top - ($bands[1].Top + $bands[1].Height)) 9
  ExpectLogical "footer chip height 36" $bands[2].Height 36
  # pad 12 + header 40 + header margin-bottom 3 + the column gap 9 that the flex
  # container adds between every pair of children.
  ExpectLogical "body padding above card row 1" ($bands[0].Top - $bodyTop) (12 + 40 + 3 + 9)
  ExpectLogical "body padding below footer 12" ($H - 1 - ($bands[2].Top + $bands[2].Height)) 12
}

Write-Host ""
Write-Host "Tokens"
$cardTop = if ($bands.Count -ge 1) { $bands[0].Top } else { $bodyTop }
Check "titlebar #1C1C1C" ((Hex ([int]($W / 2)) ($top + (Px 20))) -eq '#1C1C1C') ("got " + (Hex ([int]($W / 2)) ($top + (Px 20))))
Check "body #111111" ((Hex (Px 6) ($bodyTop + (Px 20))) -eq '#111111') ("got " + (Hex (Px 6) ($bodyTop + (Px 20))))
Check "card border #202020" ((Hex $col $cardTop) -eq '#202020') ("got " + (Hex $col $cardTop))
# Sampled at x=60: the highlight is inset by the corner radius, so x=30 sits in
# the curve on the pill and would read the fill.
Check "card top highlight #414141" ((Hex (Px 60) ($cardTop + 1)) -eq '#414141') ("got " + (Hex (Px 60) ($cardTop + 1)))
Check "card fill #1C1C1C" ((Hex $col ($cardTop + (Px 20))) -eq '#1C1C1C') ("got " + (Hex $col ($cardTop + (Px 20))))

# The Video card is one undivided surface now that the frame rate control is
# gone. Scan its row right-to-left: no split-button hairline, and the card fill
# runs all the way to the edge.
$cardRight = 0
if ($bands.Count -ge 2) {
  $mid = $bands[1].Top + (Px 32)
  for ($x = [int]($W / 2) - (Px 4); $x -gt (Px 20); $x--) {
    if ((Hex $x $mid) -ne '#111111') { $cardRight = $x; break }
  }
  $divider = 0
  for ($x = $cardRight; $x -gt (Px 20); $x--) {
    if ((Hex $x $mid) -eq '#444444') { $divider = $x; break }
  }
  Check "the Video card has no split-button divider" ($divider -eq 0) "a #444444 divider is still drawn at x=$divider"
  Check "card fill runs to the right edge" ((Hex ($cardRight - (Px 6)) $mid) -eq '#1C1C1C') ("got " + (Hex ($cardRight - (Px 6)) $mid))
}

# Segmented control: the active slot draws a 2px accent ring.
$foundAccent = $false
for ($y = $bodyTop; $y -lt $bodyTop + (Px 60) -and -not $foundAccent; $y++) {
  for ($x = [int]($W / 2); $x -lt $W - (Px 4); $x++) {
    if ((Hex $x $y) -eq '#3478F6') { $foundAccent = $true; break }
  }
}
Check "active countdown slot ring #3478F6" $foundAccent "no accent pixel in the header"

# --- artifacts -------------------------------------------------------------
#
# GPUI's overflow_hidden is a rectangular content mask - it does not follow a
# corner radius. Anything absolutely positioned inside a rounded parent has to
# clear the curve itself, or it draws across the outside of the shape. These
# are the two places where that has already gone wrong once.

Write-Host ""
Write-Host "Artifacts"
if ($bands.Count -ge 3) {
  $chipTop = $bands[2].Top
  $chipMid = $chipTop + (Px 18)
  $chipL = 0
  for ($x = (Px 6); $x -lt $W - (Px 6); $x++) { if ((Hex $x $chipMid) -ne '#111111') { $chipL = $x; break } }
  $chipR = 0
  for ($x = $chipL; $x -lt $W - (Px 6); $x++) { if ((Hex $x $chipMid) -eq '#111111') { $chipR = $x - 1; break } }
  Check "footer chip found" ($chipL -gt 0 -and $chipR -gt $chipL) "measured x $chipL..$chipR"

  # A pill's radius is half its height, so its top edge only goes flat 18px in.
  $hl = @()
  for ($x = 0; $x -lt $W; $x++) { if ((Hex $x ($chipTop + 1)) -eq '#414141') { $hl += $x } }
  Check "chip highlight is drawn" ($hl.Count -gt 0) "no #414141 on the chip top row"
  if ($hl.Count -gt 0) {
    Check "chip highlight clears the left cap" (($hl[0] - $chipL) -ge (Px 17)) ("highlight starts " + [Math]::Round((Lg ($hl[0] - $chipL)), 1) + " logical px into a chip whose cap is 18px wide - it is cutting across the curve")
    Check "chip highlight clears the right cap" (($chipR - $hl[-1]) -ge (Px 17)) ("highlight ends " + [Math]::Round((Lg ($chipR - $hl[-1])), 1) + " logical px short of the right cap")
  }
}

Write-Host ""
Write-Host "Window"
$THICKFRAME = 0x40000
$MAXIMIZEBOX = 0x10000
Check "panel is not resizable" (($style -band $THICKFRAME) -eq 0) "WS_THICKFRAME is set"
Check "panel cannot be maximised" (($style -band $MAXIMIZEBOX) -eq 0) "WS_MAXIMIZEBOX is set"

Write-Host ""
Write-Host ("  {0} passed, {1} failed" -f $script:pass, $script:fail)
$bmp.Dispose()
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
