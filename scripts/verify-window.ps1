# Behaviour check for the titlebar: drag, minimise, close.
#
# All three go through Win32 paths that GPUI either does not implement on
# Windows or implements in a way this window cancels, so none of them can be
# covered by a unit test - they are only true of a live HWND. This drives the
# real panel with real mouse input and measures what the window does.
#
# Usage:  pwsh scripts/verify-window.ps1
# Exit:   0 all checks pass, 1 otherwise. Quits RapidCap on the way out.

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class RcInput {
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(IntPtr c, string n);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int c, uint f);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
}
"@ -ErrorAction SilentlyContinue

# Pinned to DPI-*unaware* on purpose, which is the opposite of what
# verify-design.ps1 wants and for the opposite reason. That script measures
# rendered pixels, so it needs the physical grid. This one drives input and
# matches the panel on its size, and every constant below - the 380..460 gate,
# the cursor positions - is written in logical pixels. Left un-pinned, the
# thread's awareness depends on which assemblies happened to load, so the panel
# measures 400 wide on one run and 500 on another and the finder stops finding
# it. Windows virtualises both the rects and the cursor here, consistently.
$UNAWARE = [IntPtr](-1)
[void][RcInput]::SetThreadDpiAwarenessContext($UNAWARE)

$proc = Get-Process -Name RapidCap -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Host "FAIL  RapidCap is not running"; exit 1 }
# MainWindowHandle reads 0 once the panel is hidden to the tray, so look the
# window up by title, the same way the app does.
function Panel { [RcInput]::FindWindowW([IntPtr]::Zero, "RapidCap") }
$h = Panel
if ($h -eq [IntPtr]::Zero) { Write-Host "FAIL  no RapidCap window"; exit 1 }

$script:pass = 0
$script:fail = 0
function Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) { $script:pass++; Write-Host ("  PASS  {0}" -f $name) }
  else { $script:fail++; Write-Host ("  FAIL  {0} - {1}" -f $name, $detail) -ForegroundColor Red }
}

# Synthetic input is not always allowed: a remote-desktop host swallows injected
# clicks. Without this gate the whole run turns red and blames the app.
[void][RcInput]::SetCursorPos(900, 500)
Start-Sleep -Milliseconds 250
$probe = New-Object RcInput+POINT
[void][RcInput]::GetCursorPos([ref]$probe)
if ([Math]::Abs($probe.X - 900) -gt 3 -or [Math]::Abs($probe.Y - 500) -gt 3) {
  Write-Host ("SKIP  this session does not take synthetic mouse input - the pointer landed at {0},{1}" -f $probe.X, $probe.Y) -ForegroundColor Yellow
  exit 2
}

# Park the panel on a clear patch, topmost, so the synthetic clicks land on it.
$TOPMOST = [IntPtr](-1)
$NOSIZE_NOACTIVATE = 0x0001 -bor 0x0010
[void][RcInput]::SetWindowPos($h, $TOPMOST, 300, 300, 0, 0, $NOSIZE_NOACTIVATE)
Start-Sleep -Milliseconds 400
$origin = New-Object RcInput+POINT
[void][RcInput]::ClientToScreen($h, [ref]$origin)

Write-Host ""
Write-Host "RapidCap titlebar - behaviour" -ForegroundColor Cyan
Write-Host ""

# --- drag ------------------------------------------------------------------
$before = New-Object RcInput+RECT
[void][RcInput]::GetWindowRect($h, [ref]$before)
[void][RcInput]::SetCursorPos($origin.X + 60, $origin.Y + 22)
Start-Sleep -Milliseconds 250
[RcInput]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 120
foreach ($step in 1..6) {
  [void][RcInput]::SetCursorPos($origin.X + 60 + ($step * 20), $origin.Y + 22 + ($step * 10))
  Start-Sleep -Milliseconds 80
}
[RcInput]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 400
$after = New-Object RcInput+RECT
[void][RcInput]::GetWindowRect($h, [ref]$after)
$dx = $after.L - $before.L
$dy = $after.T - $before.T
Check "titlebar drag moves the panel" (([Math]::Abs($dx - 120) -le 2) -and ([Math]::Abs($dy - 60) -le 2)) "moved $dx,$dy, expected 120,60"
# The panel used to follow the cursor and then jump back the moment the button
# came up: DefWindowProc's move loop was being cancelled. Measured after the
# release, not during, so that failure cannot pass.
Check "the panel stays where it was dropped" ($dx -ne 0 -or $dy -ne 0) "snapped back to its starting rect"

# --- minimise --------------------------------------------------------------
[void][RcInput]::SetCursorPos($origin.X + 331 + ($after.L - $before.L), $origin.Y + 22 + ($after.T - $before.T))
Start-Sleep -Milliseconds 250
[RcInput]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 90
[RcInput]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 700
Check "minimise hides the panel to the tray" (-not [RcInput]::IsWindowVisible($h)) "the panel is still on screen"
Check "minimise does not leave a taskbar button" (-not [RcInput]::IsIconic($h)) "the window was iconified instead of hidden"

[void][RcInput]::ShowWindow($h, 9)
Start-Sleep -Milliseconds 600
Check "the tray brings the panel back" ([RcInput]::IsWindowVisible($h)) "SW_RESTORE did not show the panel"

# --- close -----------------------------------------------------------------
$rect = New-Object RcInput+RECT
[void][RcInput]::GetWindowRect($h, [ref]$rect)
[void][RcInput]::SetCursorPos($rect.L + 8 + 377, $rect.T + 22)
Start-Sleep -Milliseconds 250
[RcInput]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 90
[RcInput]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 1200
$alive = $null -ne (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)
Check "close quits the app" (-not $alive) "RapidCap is still running"

Write-Host ""
Write-Host ("  {0} passed, {1} failed" -f $script:pass, $script:fail)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
