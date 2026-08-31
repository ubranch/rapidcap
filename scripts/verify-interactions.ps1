# Clicks every control in the running app with real mouse input and checks what
# it actually did - the setting it wrote, the window it opened, the file it
# saved - not just that the pixels moved.
#
# Every step records a frame, so a run leaves a flip-book of every state the app
# passed through, including the region overlay and the recording HUD. Frames go
# to %TEMP%\rapidcap-frames.
#
# Titlebar drag, minimise and close live in verify-window.ps1: that script ends
# by quitting the app, so it has to run last.
#
# Usage:  pwsh scripts/verify-interactions.ps1
# Exit:   0 all checks pass, 1 otherwise. Leaves RapidCap running.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class RcUi {
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern uint GetDpiForSystem();
  delegate bool Cb(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int c);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int c, uint f);
  [DllImport("user32.dll")] public static extern IntPtr PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }

  /// The panel: the app's GPUI window at the panel's own size. MainWindowHandle
  /// is not good enough - while the panel is hidden it returns whichever other
  /// window the app has up, and a recording frame edge silently becomes "the
  /// panel" for every click that follows.
  public static IntPtr Panel(uint want, int minW, int maxW, int minH, int maxH) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != want || !IsWindowVisible(h)) return true;
      var c = new StringBuilder(64); GetClassNameW(h, c, 64);
      if (c.ToString() != "Zed::Window") return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L > minW && r.R - r.L < maxW && r.B - r.T > minH && r.B - r.T < maxH) {
        found = h; return false;
      }
      return true;
    }, IntPtr.Zero);
    return found;
  }

  /// The Explorer window showing `folder`, or zero. Its caption is
  /// "<folder> - File Explorer", so an exact FindWindow lookup never matches.
  public static IntPtr Explorer(string folder) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      if (!IsWindowVisible(h)) return true;
      var c = new StringBuilder(64); GetClassNameW(h, c, 64);
      if (c.ToString() != "CabinetWClass") return true;
      var t = new StringBuilder(300); GetWindowTextW(h, t, 300);
      if (t.ToString().StartsWith(folder)) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }

  /// Visible top-level windows belonging to `want`, as "hwnd left top width height".
  public static List<string> Windows(uint want) {
    var found = new List<string>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == want && IsWindowVisible(h)) {
        RECT r; GetWindowRect(h, out r);
        found.Add(string.Format("{0} {1} {2} {3} {4}", h.ToInt64(), r.L, r.T, r.R - r.L, r.B - r.T));
      }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@ -ErrorAction SilentlyContinue

# Pinned DPI-aware, and it has to be: `Graphics.CopyFromScreen` always reads
# the physical desktop, whatever the calling thread claims to be. Run unaware,
# the window rects come back virtualised onto the logical grid while the grab
# still reads physical, so every capture below lands about 20% short of the
# window it was aimed at and comes back as an empty bitmap - which the checks
# then report as "no recording dot" against an app that is drawing one.
#
# Pinned rather than inherited, because loading System.Drawing flips the process
# into per-monitor mode on its own: un-pinned, the panel measures 400 wide on
# one run and 500 on another and the finder stops finding it.
#
# Everything below therefore works in physical pixels. Constants that come from
# the design are written in design pixels and converted with `Du` at the point
# of use.
$PER_MONITOR_V2 = [IntPtr](-4)
[void][RcUi]::SetThreadDpiAwarenessContext($PER_MONITOR_V2)
$scale = [RcUi]::GetDpiForSystem() / 96.0

# Settings > Accessibility > Text size, the same value the app reads. The panel
# is authored in design pixels and multiplied by this on the way to the screen,
# so a script that only undoes the DPI scale aims every click short by whatever
# the slider is set to - at 130% a footer chip 250 design pixels down is clicked
# 94 physical pixels above itself, which lands on nothing.
$textScale = 1.0
$stored = (Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Accessibility' -Name TextScaleFactor -ErrorAction SilentlyContinue).TextScaleFactor
if ($stored) { $textScale = [Math]::Min([Math]::Max($stored / 100.0, 1.0), 2.25) }
$unit = $scale * $textScale

# Design unit -> physical, for a coordinate the design gives us.
function Du([double]$design) { [int][Math]::Round($design * $unit) }

$proc = Get-Process -Name RapidCap -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Host "FAIL  RapidCap is not running"; exit 1 }
$panel = [RcUi]::Panel([uint32]$proc.Id, (Du 380), (Du 460), (Du 240), (Du 340))
if ($panel -eq [IntPtr]::Zero) { Write-Host "FAIL  the panel is not on screen"; exit 1 }

$settingsFile = Join-Path $env:APPDATA "RapidCap\settings.json"
$captureRoot = Join-Path ([Environment]::GetFolderPath('MyDocuments')) "RapidCap\Screenshots"
$frames = Join-Path $env:TEMP "rapidcap-frames"
if (Test-Path $frames) { Remove-Item "$frames\*" -Force -ErrorAction SilentlyContinue }
else { New-Item -ItemType Directory $frames | Out-Null }

$TOPMOST = [IntPtr](-1)
$MOVE_ONLY = 0x0001 -bor 0x0010          # SWP_NOSIZE | SWP_NOACTIVATE

$script:pass = 0
$script:fail = 0
$script:frame = 0

function Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) { $script:pass++; Write-Host ("  PASS  {0}" -f $name) }
  else { $script:fail++; Write-Host ("  FAIL  {0} - {1}" -f $name, $detail) -ForegroundColor Red }
}

# Somewhere the panel is not. The panel is parked at 300,200 and is 400x288
# design pixels, so it reaches 300 + 400 * $unit across - past 900 on any
# machine with the text size raised, which is what a literal 900,500 used to
# assume it was clear of. A rest sample taken there was really a hover sample.
$AWAY_X = 300 + (Du 460)
$AWAY_Y = 200

function Away { [void][RcUi]::SetCursorPos($AWAY_X, $AWAY_Y) }

function Park {
  [void][RcUi]::SetWindowPos($panel, $TOPMOST, 300, 200, 0, 0, $MOVE_ONLY)
  Start-Sleep -Milliseconds 400
  $o = New-Object RcUi+POINT
  [void][RcUi]::ClientToScreen($panel, [ref]$o)
  return $o
}

function Grab([int]$x, [int]$y, [int]$w, [int]$h, [string]$label) {
  $script:frame++
  $bmp = New-Object System.Drawing.Bitmap($w, $h)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
  $g.Dispose()
  $bmp.Save((Join-Path $frames ("{0:d2}-{1}.png" -f $script:frame, $label)), [System.Drawing.Imaging.ImageFormat]::Png)
  return $bmp
}

# Records the panel. Returns $null when the panel is off screen, so a check can
# say so instead of measuring a stale rect.
function Shot([string]$label) {
  if (-not [RcUi]::IsWindowVisible($panel)) { return $null }
  $r = New-Object RcUi+RECT
  [void][RcUi]::DwmGetWindowAttribute($panel, 9, [ref]$r, 16)
  # A minimised window still reports IsWindowVisible, and its bounds are the
  # -32000 placeholder. Capturing that yields a 146x28 white smear that quietly
  # passes for a panel screenshot.
  if (($r.R - $r.L) -lt (Du 300) -or ($r.B - $r.T) -lt (Du 200)) { return $null }
  return Grab $r.L $r.T ($r.R - $r.L) ($r.B - $r.T) $label
}

function Snap([string]$label) {
  $bmp = Shot $label
  if ($bmp) { $bmp.Dispose() }
}

# Polls, because encoding a capture takes as long as it takes.
function WaitForCapture([int]$was, [int]$seconds) {
  foreach ($tick in 1..$seconds) {
    Start-Sleep -Seconds 1
    if ((Captures) -gt $was) { return $true }
  }
  return $false
}

# Real input, not posted messages: GPUI answers hit tests from the last mouse
# position, so the cursor has to actually travel to the control.
function Click([int]$sx, [int]$sy, [int]$settle = 450) {
  [void][RcUi]::SetCursorPos($sx, $sy)
  Start-Sleep -Milliseconds 200
  [RcUi]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 90
  [RcUi]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds $settle
}

function Settings { Get-Content $settingsFile -Raw | ConvertFrom-Json }
function Captures { @(Get-ChildItem $captureRoot -Recurse -File -ErrorAction SilentlyContinue).Count }
function Next($choices, $current) {
  for ($i = 0; $i -lt $choices.Count; $i++) { if ($choices[$i] -eq $current) { return $choices[($i + 1) % $choices.Count] } }
  return $choices[0]
}
# Every visible app window except the panel: the overlay, the HUD, the frame.
function Extra {
  @([RcUi]::Windows([uint32]$proc.Id)) |
    ForEach-Object { $p = $_ -split ' '; [pscustomobject]@{ H = [IntPtr][int64]$p[0]; L = [int]$p[1]; T = [int]$p[2]; W = [int]$p[3]; H0 = [int]$p[4] } } |
    Where-Object { $_.H -ne $panel }
}

# Synthetic input is not always allowed: a remote-desktop host swallows injected
# clicks and keeps moving the pointer under you. Without this gate the whole run
# turns red and blames the app for the session.
function AssertInputWorks {
  [void][RcUi]::SetCursorPos(900, 500)
  Start-Sleep -Milliseconds 250
  $p = New-Object RcUi+POINT
  [void][RcUi]::GetCursorPos([ref]$p)
  if ([Math]::Abs($p.X - 900) -gt 3 -or [Math]::Abs($p.Y - 500) -gt 3) {
    Write-Host ("SKIP  this session does not take synthetic mouse input - the pointer landed at {0},{1} instead of 900,500. Nothing below would be about RapidCap." -f $p.X, $p.Y) -ForegroundColor Yellow
    exit 2
  }
}

AssertInputWorks

$leftover = @(Extra)
if ($leftover.Count -gt 0) {
  Write-Host ("FAIL  {0} other RapidCap window(s) are up - a capture or recording is still running. Restart the app and try again." -f $leftover.Count)
  exit 1
}

Write-Host ""
Write-Host "RapidCap - every control, clicked" -ForegroundColor Cyan
Write-Host ("  frames -> {0}" -f $frames)

$o = Park
# Printed because every click below is an offset from it: when a run goes red
# across the board, this line says whether the clicks were even aimed at the app.
$r0 = New-Object RcUi+RECT
[void][RcUi]::GetWindowRect($panel, [ref]$r0)
Write-Host ("  panel  {0},{1} {2}x{3} outer, client origin {4},{5}, scale {6}" -f $r0.L, $r0.T, ($r0.R - $r0.L), ($r0.B - $r0.T), $o.X, $o.Y, $scale)
Snap "idle"

# --- countdown -------------------------------------------------------------
Write-Host ""
Write-Host "Countdown"
foreach ($slot in @(@(334, 3, 'countdown-3'), @(368, 5, 'countdown-5'), @(300, 0, 'countdown-off'))) {
  Click ($o.X + (Du $slot[0])) ($o.Y + (Du 54))
  Snap $slot[2]
  $got = (Settings).countdown_seconds
  Check ("{0} writes countdown_seconds = {1}" -f $slot[2], $slot[1]) ($got -eq $slot[1]) "settings.json says $got"
}

# --- frame rate ------------------------------------------------------------
# There is no frame rate control any more: the rates are fixed and the whole
# card is one hitbox. Two things have to hold. The rates the settings file
# carries are the ones the header badge states, and the strip on the right of a
# card where the stepper used to sit now belongs to the card - hovering it lifts
# the whole card to the hover fill. Hover rather than click: clicking there
# opens the overlay, and the release lands on the overlay and picks a window,
# which would leave a live recording running through the rest of this run.
Write-Host ""
Write-Host "Frame rate"
$fps = Settings
Check "video records at 30 fps" ($fps.video.fps -eq 30) "settings.json says $($fps.video.fps)"
Check "GIF records at 15 fps" ($fps.gif.fps -eq 15) "settings.json says $($fps.gif.fps)"
$edgeX = $o.X + (Du 371)
$edgeY = $o.Y + (Du 191)
Away; Start-Sleep -Milliseconds 500
$rest = Grab $edgeX $edgeY 1 1 "gif-card-edge-rest"
[void][RcUi]::SetCursorPos($edgeX, $edgeY); Start-Sleep -Milliseconds 500
$lit = Grab $edgeX $edgeY 1 1 "gif-card-edge-hover"
$restPixel = $rest.GetPixel(0, 0)
$litPixel = $lit.GetPixel(0, 0)
$rest.Dispose(); $lit.Dispose()
Check "the strip the stepper used to own is part of the GIF card" ($litPixel.ToArgb() -ne $restPixel.ToArgb()) ("the fill stayed {0:X6} with the pointer on it - a dead zone is still there" -f ($restPixel.ToArgb() -band 0xFFFFFF))
Away; Start-Sleep -Milliseconds 300

# --- footer ----------------------------------------------------------------
Write-Host ""
Write-Host "Footer"
# The footer runs audio chip, output chip, status well. The output chip is the
# middle one - aiming at the left edge toggles the audio instead, silently, and
# then waits seven seconds for an Explorer window nothing asked for.
Click ($o.X + (Du 175)) ($o.Y + (Du 258)) 800
Snap "output-chip"
$explorer = [IntPtr]::Zero
foreach ($tick in 1..15) {
  $explorer = [RcUi]::Explorer("Screenshots")
  if ($explorer -ne [IntPtr]::Zero) { break }
  Start-Sleep -Milliseconds 500
}
Check "the output chip opens the Screenshots folder" ($explorer -ne [IntPtr]::Zero) "no Explorer window titled Screenshots after 7s"
if ($explorer -ne [IntPtr]::Zero) { [void][RcUi]::PostMessageW($explorer, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
$o = Park

$snapshot = (Settings) | ConvertTo-Json -Depth 5
Click ($o.X + (Du 330)) ($o.Y + (Du 250))
Snap "status-well"
Check "the status well is not a button" (((Settings) | ConvertTo-Json -Depth 5) -eq $snapshot) "clicking the status well changed a setting"

# --- region overlay --------------------------------------------------------
# Drag out a region on the overlay and let go. Returns the HUD-or-nothing state
# to the caller; the same gesture arms a screenshot, a video and a GIF.
function DragRegion([int]$x, [int]$y) {
  [void][RcUi]::SetCursorPos($x, $y)
  Start-Sleep -Milliseconds 300
  [RcUi]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
  foreach ($step in 1..8) {
    [void][RcUi]::SetCursorPos(($x + $step * 30), ($y + $step * 20))
    Start-Sleep -Milliseconds 60
  }
}
function DropRegion {
  [RcUi]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 900
}
# The overlay is the full-screen window; the HUD is the short wide pill. The
# recording frame is also short and wide, so the HUD is the narrower of the two.
function Overlay { @(Extra) | Where-Object { $_.H0 -gt (Du 400) } | Select-Object -First 1 }
function Hud { @(Extra) | Where-Object { $_.H0 -lt (Du 120) -and $_.W -gt (Du 200) -and $_.W -lt (Du 900) } | Select-Object -First 1 }
function WaitFor([scriptblock]$probe, [int]$seconds) {
  foreach ($tick in 1..($seconds * 4)) {
    $found = & $probe
    if ($found) { return $found }
    Start-Sleep -Milliseconds 250
  }
  return $null
}

Write-Host ""
Write-Host "Region capture"
$countBefore = Captures
# The whole desktop, sampled before the overlay goes up so the scrim has
# something to be measured against. A single reference pixel was not enough:
# whatever is behind the overlay keeps repainting - a browser, a clock, a
# progress bar - and the pixel read seconds before the overlay appeared is not
# the pixel the scrim ended up over.
$bare = New-Object System.Drawing.Bitmap([int][RcUi]::GetSystemMetrics(78), [int][RcUi]::GetSystemMetrics(79))
$g = [System.Drawing.Graphics]::FromImage($bare)
$g.CopyFromScreen([int][RcUi]::GetSystemMetrics(76), [int][RcUi]::GetSystemMetrics(77), 0, 0, $bare.Size)
$g.Dispose()

Click ($o.X + (Du 103)) ($o.Y + (Du 118)) 1000
$overlay = WaitFor { Overlay } 6
Check "the Region card opens the overlay" ($null -ne $overlay) "no full-screen window appeared"
if (-not $overlay) { $bare.Dispose() }
if ($overlay) {
  $shot = Grab $overlay.L $overlay.T $overlay.W $overlay.H0 "region-overlay"
  # Black at alpha 0x8c over whatever was there, so every channel should land at
  # 1 - 140/255 of its bare value. Measured across a grid rather than at one
  # point, and reduced to a median: the parts of the desktop that repainted
  # between the two grabs are outliers, and the ratio the scrim actually applied
  # is what survives them. Channels under 24 are skipped - near-black rounds too
  # coarsely to carry a ratio.
  # The bare grab starts at the virtual screen's origin and the overlay grab at
  # the overlay's, which are the same point on one monitor and are not on more
  # than one.
  $ox = $overlay.L - [RcUi]::GetSystemMetrics(76)
  $oy = $overlay.T - [RcUi]::GetSystemMetrics(77)
  $ratios = [Collections.Generic.List[double]]::new()
  for ($sx = 0; $sx -lt [Math]::Min($bare.Width - $ox, $shot.Width); $sx += 40) {
    for ($sy = 0; $sy -lt [Math]::Min($bare.Height - $oy, $shot.Height); $sy += 40) {
      $u = $bare.GetPixel($sx + $ox, $sy + $oy)
      $t = $shot.GetPixel($sx, $sy)
      if ($u.R -ge 24) { $ratios.Add($t.R / $u.R) }
      if ($u.G -ge 24) { $ratios.Add($t.G / $u.G) }
      if ($u.B -ge 24) { $ratios.Add($t.B / $u.B) }
    }
  }
  $shot.Dispose()
  $bare.Dispose()
  $keep = 1.0 - (140.0 / 255.0)
  $sorted = $ratios | Sort-Object
  $median = if ($sorted.Count) { $sorted[[int]($sorted.Count / 2)] } else { [double]::NaN }
  Check "the scrim dims the desktop by 55%" ([Math]::Abs($median - $keep) -le 0.03) ("the median of {0} samples kept {1:n3} of the desktop, not {2:n3}" -f $sorted.Count, $median, $keep)

  DragRegion 600 400
  $shot = Grab $overlay.L $overlay.T $overlay.W $overlay.H0 "region-selecting"
  # The four corner grips are near-white on a dimmed desktop, so a small box
  # around each corner of the rectangle that was just dragged should hold one.
  $corners = @(@(600, 400), @(840, 400), @(600, 560), @(840, 560))
  $grips = 0
  $slack = Du 5
  foreach ($corner in $corners) {
    $found = $false
    foreach ($dx in -$slack..$slack) {
      foreach ($dy in -$slack..$slack) {
        $c = $shot.GetPixel($corner[0] - $overlay.L + $dx, $corner[1] - $overlay.T + $dy)
        if ($c.R -gt 220 -and $c.G -gt 220 -and $c.B -gt 220) { $found = $true; break }
      }
      if ($found) { break }
    }
    if ($found) { $grips++ }
  }
  Check "the drag rect draws four corner grips" ($grips -eq 4) "found $grips of 4"
  $shot.Dispose()
  DropRegion
  Check "the region drag writes a capture" (WaitForCapture $countBefore 20) "no new file under $captureRoot"
  Check "the overlay closes after the capture" ($null -eq (Overlay)) "the overlay is still up"
  # The badge said 240 x 160. A capture that saves anything else is a capture
  # that cropped the wrong rectangle.
  $saved = Get-ChildItem $captureRoot -Recurse -File | Sort-Object LastWriteTime | Select-Object -Last 1
  if ($saved -and $saved.Extension -eq '.png') {
    $img = [System.Drawing.Image]::FromFile($saved.FullName)
    $dims = "{0} x {1}" -f $img.Width, $img.Height
    $img.Dispose()
    Check "the saved region is the size the badge promised" ($dims -eq '240 x 160') "saved $dims"
  } else {
    Check "the saved region is the size the badge promised" $false "newest capture is not a PNG"
  }
}
$o = Park

# --- active window capture -------------------------------------------------
Write-Host ""
Write-Host "Window capture"
$countBefore = Captures
Click ($o.X + (Du 296)) ($o.Y + (Du 118)) 1000
$overlay = WaitFor { Overlay } 6
Check "the Window card opens the overlay" ($null -ne $overlay) "no full-screen window appeared"
if ($overlay) {
  # Hover first: the overlay is meant to snap to the window under the pointer
  # and name it, before any click commits to it.
  [void][RcUi]::SetCursorPos(700, 500); Start-Sleep -Milliseconds 300
  [void][RcUi]::SetCursorPos(710, 505); Start-Sleep -Milliseconds 800
  $shot = Grab $overlay.L $overlay.T $overlay.W $overlay.H0 "window-hover"
  $accent = $false
  for ($y = 0; $y -lt $overlay.H0 -and -not $accent; $y += 3) {
    for ($x = 0; $x -lt $overlay.W; $x += 3) {
      $c = $shot.GetPixel($x, $y)
      if ($c.R -gt 40 -and $c.R -lt 70 -and $c.G -gt 100 -and $c.G -lt 140 -and $c.B -gt 225) { $accent = $true; break }
    }
  }
  $shot.Dispose()
  Check "hovering a window outlines it in the accent" $accent "no #3478F6 border on the overlay"

  # A click, not a drag: on the overlay that means "capture the window under
  # the pointer".
  Click 700 500 1200
  Check "clicking a window captures it" (WaitForCapture $countBefore 20) "no new file under $captureRoot"
  Check "the overlay closes after the window capture" ($null -eq (Overlay)) "the overlay is still up"
}
$o = Park
Snap "after-window-capture"

# --- recordings ------------------------------------------------------------
# Video and GIF do not start on the click: they arm a selection first, so the
# card opens the same overlay the Region card does, and the countdown only runs
# once a target has been picked. countdown_seconds is 0 by now.
foreach ($rec in @(@(86, 'video', 'Video'), @(279, 'gif', 'GIF'))) {
  Write-Host ""
  Write-Host ("{0} recording" -f $rec[2])
  # Video runs with a 3s countdown so the countdown bar can be checked; GIF runs
  # with none, so the rest of the pass is not waiting on it.
  Click ($o.X + (Du $(if ($rec[1] -eq 'video') { 334 } else { 300 }))) ($o.Y + (Du 54))
  $countBefore = Captures
  Click ($o.X + (Du $rec[0])) ($o.Y + (Du 191)) 1000
  $overlay = WaitFor { Overlay } 6
  Check ("the {0} card opens the target overlay" -f $rec[2]) ($null -ne $overlay) "no overlay appeared"
  if (-not $overlay) { $o = Park; continue }
  DragRegion 600 400
  DropRegion
  $hud = WaitFor { Hud } 10
  Check ("the {0} recording shows the HUD" -f $rec[2]) ($null -ne $hud) "no HUD pill appeared"
  if (-not $hud) { $o = Park; continue }

  # Where the bar sits, measured off its window rect rather than its pixels.
  # The bar carries WDA_EXCLUDEFROMCAPTURE - it has to, or it lands in the file
  # being recorded - and that hides it from `CopyFromScreen` as well, so a
  # screenshot of it reads straight through to the desktop behind. Its position
  # is still readable, and position is what kept going wrong: the gap was a
  # design-pixel constant compared against device coordinates, and the centre
  # was truncated instead of rounded.
  $region = @{ L = 600; T = 400; W = 240; H = 160 }
  $wantW = Du 360; $wantH = Du 44
  Check ("the {0} HUD is the size the letterbox asks for" -f $rec[2]) `
    ([Math]::Abs($hud.W - $wantW) -le 1 -and [Math]::Abs($hud.H0 - $wantH) -le 1) `
    ("{0}x{1}, expected {2}x{3}" -f $hud.W, $hud.H0, $wantW, $wantH)
  $hudCentre = $hud.L + $hud.W / 2.0
  $regionCentre = $region.L + $region.W / 2.0
  Check ("the {0} HUD is centred on the region" -f $rec[2]) `
    ([Math]::Abs($hudCentre - $regionCentre) -le 1) `
    ("bar centre {0}, region centre {1}" -f $hudCentre, $regionCentre)
  # GAP is 9 design pixels below the region's bottom edge.
  $gap = $hud.T - ($region.T + $region.H)
  Check ("the {0} HUD sits one gap below the region" -f $rec[2]) `
    ([Math]::Abs($gap - (Du 9)) -le 1) ("gap is {0}px, expected {1}px" -f $gap, (Du 9))

  # Both buttons sit at fixed offsets from the pill's right edge: 4px of pill
  # padding, then a 28px stop button, then a 4px gap, then a 28px pause button.
  # The pill is 248 design pixels wide and centred in its window.
  $y = [int]($hud.H0 / 2)
  $pillRight = $hud.L + [int]($hud.W / 2) + (Du 124)
  $stopX = $pillRight - (Du 18)
  $pauseX = $stopX - (Du 32)

  # Wait out the countdown, then hold the recording open long enough for its
  # duration to say something.
  Start-Sleep -Seconds $(if ($rec[1] -eq 'video') { 5 } else { 2 })
  $rolling = 2
  Start-Sleep -Seconds $rolling
  $held = 0
  if ($rec[1] -eq 'video') {
    # Pause suspends the encoder process, so a paused stretch is missing from
    # the finished file. That is the only observable pause has: the bar carries
    # WDA_EXCLUDEFROMCAPTURE - it has to, or it lands in the recording - so a
    # screenshot of it reads through to the desktop. `hud_face` covers what each
    # state draws; the duration below covers whether it did anything.
    Click $pauseX ($hud.T + $y) 900
    Check "the pause button is where the pill's geometry puts it" ($null -ne (Hud)) "the HUD vanished - the pause click hit the stop button"
    $held = 4
    Start-Sleep -Seconds $held
    Click $pauseX ($hud.T + $y) 900
    Check "the HUD survives resume" ($null -ne (Hud)) "the HUD vanished on the resume click"
    Start-Sleep -Seconds $rolling
    $rolling += 2
  }
  # Stop is not instant on a take that was paused: each unpaused stretch is its
  # own FFmpeg run, and they are concatenated before the file exists, so the bar
  # stays up through the join. Waited on rather than slept past, and the wait is
  # reported, so a join that starts costing real time shows up as a number
  # instead of as a flake.
  Click $stopX ($hud.T + $y) 300
  $began = Get-Date
  $gone = WaitFor { if ($null -eq (Hud)) { $true } } 15
  $took = [int]((Get-Date) - $began).TotalMilliseconds
  Check ("stop ends the {0} recording" -f $rec[2]) ($gone -eq $true) "the HUD was still up 15s after the stop click"
  if ($gone) { Write-Host ("        the bar came down {0}ms after the stop click" -f $took) -ForegroundColor DarkGray }
  Check ("the {0} recording is written to disk" -f $rec[2]) (WaitForCapture $countBefore 60) "no new file under $captureRoot"

  if ($rec[1] -eq 'video') {
    $file = Get-ChildItem $captureRoot -Recurse -File | Sort-Object LastWriteTime | Select-Object -Last 1
    $probe = Get-Command ffprobe -ErrorAction SilentlyContinue
    if (-not $probe) {
      Check "pause keeps the paused seconds out of the file" $false "ffprobe is not on PATH, so the recording's duration cannot be read"
    } else {
      $seconds = [double](& $probe.Source -v error -show_entries format=duration -of csv=p=0 $file.FullName)
      # Four seconds of frames either side of a four-second pause. A pause that
      # did nothing leaves all eight in the file; the clicks and the countdown
      # add slop, so the window is wide and the two outcomes still cannot
      # overlap.
      Check "pause keeps the paused seconds out of the file" `
        ($seconds -gt 2.0 -and $seconds -lt ($rolling + $held - 1.0)) `
        ("the file runs {0:n1}s - {1}s of frames were recorded around a {2}s pause" -f $seconds, $rolling, $held)
    }
  }
  $o = Park
  Snap ("after-" + $rec[1])
}

Write-Host ""
Write-Host ("  {0} passed, {1} failed" -f $script:pass, $script:fail)
Write-Host ("  {0} frames in {1}" -f $script:frame, $frames)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
