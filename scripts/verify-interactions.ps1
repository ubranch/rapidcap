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
  delegate bool Cb(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int c, uint f);
  [DllImport("user32.dll")] public static extern IntPtr PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }

  /// The panel: the app's GPUI window at the panel's own size. MainWindowHandle
  /// is not good enough - while the panel is hidden it returns whichever other
  /// window the app has up, and a recording frame edge silently becomes "the
  /// panel" for every click that follows.
  public static IntPtr Panel(uint want) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != want || !IsWindowVisible(h)) return true;
      var c = new StringBuilder(64); GetClassNameW(h, c, 64);
      if (c.ToString() != "Zed::Window") return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L > 380 && r.R - r.L < 460 && r.B - r.T > 280 && r.B - r.T < 360) {
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

$proc = Get-Process -Name RapidCap -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Host "FAIL  RapidCap is not running"; exit 1 }
$panel = [RcUi]::Panel([uint32]$proc.Id)
if ($panel -eq [IntPtr]::Zero) { Write-Host "FAIL  the panel is not on screen"; exit 1 }

$settingsFile = Join-Path $env:APPDATA "RapidCap\settings.json"
$captureRoot = Join-Path ([Environment]::GetFolderPath('MyDocuments')) "RapidCap\Screenshots"
$frames = Join-Path $env:TEMP "rapidcap-frames"
if (Test-Path $frames) { Remove-Item "$frames\*" -Force -ErrorAction SilentlyContinue }
else { New-Item -ItemType Directory $frames | Out-Null }

$TOPMOST = [IntPtr](-1)
$NOTOPMOST = [IntPtr](-2)
$MOVE_ONLY = 0x0001 -bor 0x0010          # SWP_NOSIZE | SWP_NOACTIVATE
$ZORDER_ONLY = 0x0001 -bor 0x0002 -bor 0x0010

$script:pass = 0
$script:fail = 0
$script:frame = 0

function Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) { $script:pass++; Write-Host ("  PASS  {0}" -f $name) }
  else { $script:fail++; Write-Host ("  FAIL  {0} - {1}" -f $name, $detail) -ForegroundColor Red }
}

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
  if (($r.R - $r.L) -lt 300 -or ($r.B - $r.T) -lt 200) { return $null }
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
# The bar fades to 55% after three seconds alone, so anything that measures its
# colours has to wake it first.
function WakeHud($hud) {
  [void][RcUi]::SetCursorPos(($hud.L + [int]($hud.W / 2)), ($hud.T + [int]($hud.H0 / 2)))
  Start-Sleep -Milliseconds 400
}

# The pill is a neutral near-black (#1B1B1B). Testing for "dark" alone is not
# enough: the window around it is transparent, so a dark wallpaper reads as pill
# and every measurement below slides left.
function IsPill($c) {
  return ($c.R -lt 45 -and $c.G -lt 45 -and $c.B -lt 45 -and
          [Math]::Abs($c.R - $c.G) -lt 12 -and [Math]::Abs($c.G - $c.B) -lt 12)
}

# Width of the HUD pill on row $y.
function PillSpan($bmp, [int]$y) {
  $first = -1; $last = -1
  for ($x = 0; $x -lt $bmp.Width; $x++) {
    if (IsPill $bmp.GetPixel($x, $y)) {
      if ($first -lt 0) { $first = $x }
      $last = $x
    }
  }
  if ($first -lt 0) { return 0 }
  return $last - $first + 1
}

# The recording dot is an 8px disc with pill on both sides of it. The stop
# button is red too but 28px across, and the recording frame behind the
# transparent window is a red band - both fail the "pill within 8px" test.
function HasDot($bmp) {
  # The pill is not vertically centred in its window - it carries a drop shadow -
  # so the dot straddles rows above the midline. Sweep a band rather than trust
  # one row.
  for ($y = [int]($bmp.Height * 0.35); $y -le [int]($bmp.Height * 0.55); $y++) {
    for ($x = 8; $x -lt $bmp.Width - 8; $x++) {
      $c = $bmp.GetPixel($x, $y)
      if ($c.R -gt 180 -and $c.G -lt 90 -and $c.B -lt 90 -and
          (IsPill $bmp.GetPixel($x - 8, $y)) -and (IsPill $bmp.GetPixel($x + 8, $y))) {
        return $true
      }
    }
  }
  return $false
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
Snap "idle"

# --- titlebar --------------------------------------------------------------
Write-Host ""
Write-Host "Titlebar"
$EXSTYLE = -20
$WS_EX_TOPMOST = 0x8
# Drop the harness' own pin first, or the toggle has nothing to prove.
[void][RcUi]::SetWindowPos($panel, $NOTOPMOST, 0, 0, 0, 0, $ZORDER_ONLY)
Start-Sleep -Milliseconds 300
Click ($o.X + 281) ($o.Y + 22)
Snap "keep-on-top-on"
Check "keep on top pins the panel" ((([int64][RcUi]::GetWindowLongPtrW($panel, $EXSTYLE)) -band $WS_EX_TOPMOST) -ne 0) "WS_EX_TOPMOST is clear"
Click ($o.X + 281) ($o.Y + 22)
Snap "keep-on-top-off"
Check "keep on top releases the pin" ((([int64][RcUi]::GetWindowLongPtrW($panel, $EXSTYLE)) -band $WS_EX_TOPMOST) -eq 0) "WS_EX_TOPMOST is still set"
$o = Park

# --- countdown -------------------------------------------------------------
Write-Host ""
Write-Host "Countdown"
foreach ($slot in @(@(334, 3, 'countdown-3'), @(368, 5, 'countdown-5'), @(300, 0, 'countdown-off'))) {
  Click ($o.X + $slot[0]) ($o.Y + 76)
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
$edgeX = $o.X + 371
$edgeY = $o.Y + 213
[void][RcUi]::SetCursorPos(900, 500); Start-Sleep -Milliseconds 500
$rest = Grab $edgeX $edgeY 1 1 "gif-card-edge-rest"
[void][RcUi]::SetCursorPos($edgeX, $edgeY); Start-Sleep -Milliseconds 500
$lit = Grab $edgeX $edgeY 1 1 "gif-card-edge-hover"
$restPixel = $rest.GetPixel(0, 0)
$litPixel = $lit.GetPixel(0, 0)
$rest.Dispose(); $lit.Dispose()
Check "the strip the stepper used to own is part of the GIF card" ($litPixel.ToArgb() -ne $restPixel.ToArgb()) ("the fill stayed {0:X6} with the pointer on it - a dead zone is still there" -f ($restPixel.ToArgb() -band 0xFFFFFF))
[void][RcUi]::SetCursorPos(900, 500); Start-Sleep -Milliseconds 300

# --- footer ----------------------------------------------------------------
Write-Host ""
Write-Host "Footer"
Click ($o.X + 60) ($o.Y + 272) 800
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
Click ($o.X + 330) ($o.Y + 272)
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
function Overlay { @(Extra) | Where-Object { $_.H0 -gt 400 } | Select-Object -First 1 }
function Hud { @(Extra) | Where-Object { $_.H0 -lt 120 -and $_.W -gt 200 -and $_.W -lt 900 } | Select-Object -First 1 }
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
# A patch of desktop clear of the panel, sampled before and after the scrim
# goes up: the design calls for rgba(0,0,0,.55), which leaves 45% of whatever
# was underneath.
$bare = New-Object System.Drawing.Bitmap(1, 1)
$g = [System.Drawing.Graphics]::FromImage($bare)
$g.CopyFromScreen(1600, 300, 0, 0, $bare.Size)
$g.Dispose()
$under = $bare.GetPixel(0, 0)
$bare.Dispose()

Click ($o.X + 103) ($o.Y + 140) 1000
$overlay = WaitFor { Overlay } 6
Check "the Region card opens the overlay" ($null -ne $overlay) "no full-screen window appeared"
if ($overlay) {
  $shot = Grab $overlay.L $overlay.T $overlay.W $overlay.H0 "region-overlay"
  $dimmed = $shot.GetPixel(1600 - $overlay.L, 300 - $overlay.T)
  $wantR = [int]($under.R * 0.45)
  $wantG = [int]($under.G * 0.45)
  $wantB = [int]($under.B * 0.45)
  $close = ([Math]::Abs($dimmed.R - $wantR) -le 6) -and ([Math]::Abs($dimmed.G - $wantG) -le 6) -and ([Math]::Abs($dimmed.B - $wantB) -le 6)
  Check "the scrim dims the desktop by 55%" $close ("under {0},{1},{2} -> {3},{4},{5}, expected {6},{7},{8}" -f $under.R, $under.G, $under.B, $dimmed.R, $dimmed.G, $dimmed.B, $wantR, $wantG, $wantB)
  $shot.Dispose()

  DragRegion 600 400
  $shot = Grab $overlay.L $overlay.T $overlay.W $overlay.H0 "region-selecting"
  # The drag ran from 600,400 to 840,560 in screen pixels. Each corner carries a
  # 7px grip straddling the border, so there is near-white within a few pixels
  # of the corner - which side of it depends on rounding, so scan a small box.
  $corners = @(@(600, 400), @(840, 400), @(600, 560), @(840, 560))
  $grips = 0
  foreach ($corner in $corners) {
    $found = $false
    foreach ($dx in -5..5) {
      foreach ($dy in -5..5) {
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
Click ($o.X + 296) ($o.Y + 140) 1000
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
  Click ($o.X + $(if ($rec[1] -eq 'video') { 334 } else { 300 })) ($o.Y + 76)
  $countBefore = Captures
  Click ($o.X + $rec[0]) ($o.Y + 213) 1000
  $overlay = WaitFor { Overlay } 6
  Check ("the {0} card opens the target overlay" -f $rec[2]) ($null -ne $overlay) "no overlay appeared"
  if (-not $overlay) { $o = Park; continue }
  DragRegion 600 400
  DropRegion
  $hud = WaitFor { Hud } 10
  Check ("the {0} recording shows the HUD" -f $rec[2]) ($null -ne $hud) "no HUD pill appeared"
  if (-not $hud) { $o = Park; continue }

  if ($rec[1] -eq 'video') {
    # Countdown state: the one moment the bar is allowed to be wide, because it
    # names the target while there is still time to cancel.
    $shot = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-countdown"
    $counting = PillSpan $shot ([int]($hud.H0 / 2))
    Check "the countdown HUD is wider than the running bar" ($counting -gt 200) "pill is only ${counting}px wide"
    Check "the countdown HUD shows no recording dot" (-not (HasDot $shot)) "the red dot is up before recording starts"
    $shot.Dispose()
  }

  # Wait out any countdown so the checks below see the running bar.
  $running = WaitFor {
    WakeHud $hud
    $probe = Grab $hud.L $hud.T $hud.W $hud.H0 "probe"
    $ok = HasDot $probe
    $probe.Dispose()
    Remove-Item (Join-Path $frames ("{0:d2}-probe.png" -f $script:frame)) -Force -ErrorAction SilentlyContinue
    $script:frame--
    if ($ok) { $true } else { $null }
  } 12
  Check ("the {0} recording reaches the running state" -f $rec[2]) ($null -ne $running) "no recording dot after 12s"

  Start-Sleep -Seconds 2
  WakeHud $hud
  $shot = Grab $hud.L $hud.T $hud.W $hud.H0 ("hud-" + $rec[1])
  # The stop button is the only danger-red run on the pill's middle row.
  $y = [int]($hud.H0 / 2)
  $first = -1; $last = -1
  for ($x = 0; $x -lt $hud.W; $x++) {
    $c = $shot.GetPixel($x, $y)
    if ($c.R -gt 170 -and $c.R -lt 225 -and $c.G -lt 80 -and $c.B -lt 80) {
      if ($first -lt 0) { $first = $x }
      $last = $x
    }
  }
  $shot.Dispose()
  Check ("the {0} HUD draws a stop button" -f $rec[2]) ($first -ge 0) "no danger-red run on the pill"
  if ($first -ge 0 -and $rec[1] -eq 'video') {
    # Pause sits one 28px button plus a 4px gap to the left of stop.
    $stopX = $hud.L + [int](($first + $last) / 2)
    Click ($stopX - 32) ($hud.T + $y) 900
    WakeHud $hud
    $shot = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-paused"
    Check "pause greys the recording dot" (-not (HasDot $shot)) "the dot is still recording red while paused"
    $shot.Dispose()
    Click ($stopX - 32) ($hud.T + $y) 900
    WakeHud $hud
    $shot = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-resumed"
    Check "resume brings the recording dot back" (HasDot $shot) "the dot stayed grey after resume"
    $shot.Dispose()

    # Idle fade: three seconds with the pointer away and the bar drops to 55%.
    WakeHud $hud
    $lit = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-lit"
    [void][RcUi]::SetCursorPos(60, 60)
    Start-Sleep -Milliseconds 3800
    $dim = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-faded"
    $litPixel = $lit.GetPixel([int]($hud.W / 2), $y)
    $dimPixel = $dim.GetPixel([int]($hud.W / 2), $y)
    Check "the HUD fades when the pointer leaves" ($litPixel.ToArgb() -ne $dimPixel.ToArgb()) "the bar looks identical after 3s away"
    $lit.Dispose(); $dim.Dispose()

    [void][RcUi]::SetCursorPos($stopX, $hud.T + $y)
    Start-Sleep -Milliseconds 700
    $back = Grab $hud.L $hud.T $hud.W $hud.H0 "hud-rehover"
    $backPixel = $back.GetPixel([int]($hud.W / 2), $y)
    Check "the HUD comes back on hover" ($backPixel.ToArgb() -ne $dimPixel.ToArgb()) "the bar stayed faded with the pointer on it"
    $back.Dispose()
  }
  if ($first -ge 0) {
    WakeHud $hud
    Click ($hud.L + [int](($first + $last) / 2)) ($hud.T + $y) 1500
    Check ("stop ends the {0} recording" -f $rec[2]) ($null -eq (Hud)) "the HUD is still up"
    Check ("the {0} recording is written to disk" -f $rec[2]) (WaitForCapture $countBefore 60) "no new file under $captureRoot"
  }
  $o = Park
  Snap ("after-" + $rec[1])
}

Write-Host ""
Write-Host ("  {0} passed, {1} failed" -f $script:pass, $script:fail)
Write-Host ("  {0} frames in {1}" -f $script:frame, $frames)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
