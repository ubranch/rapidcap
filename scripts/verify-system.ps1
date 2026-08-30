# The parts of RapidCap that live outside its own window: global hotkeys, the
# tray icon, keyboard navigation, the single-instance guard and the `--probe`
# contract. None of these can be reached by clicking the panel and none can be
# unit tested - they are agreements with Windows.
#
# Some of them need synthetic keyboard input, which not every session allows: a
# remote-desktop host swallows injected keystrokes entirely. Those checks report
# SKIP with the reason rather than a red failure, so a run on a locked-down
# session still says something true.
#
# Usage:  pwsh scripts/verify-system.ps1
# Exit:   0 all runnable checks pass, 1 otherwise. Leaves RapidCap running.

$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class RcSys {
  delegate bool Cb(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(Cb cb, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int c);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
  [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }

  /// The panel, by class and size. MainWindowHandle returns whatever window the
  /// app has up while the panel is hidden, which is never what a caller means.
  public static IntPtr Panel(uint want) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != want || !IsWindowVisible(h)) return true;
      var c = new StringBuilder(64); GetClassNameW(h, c, 64);
      if (c.ToString() != "Zed::Window") return true;
      RECT r; GetWindowRect(h, out r);
      if (r.R - r.L > 380 && r.R - r.L < 460 && r.B - r.T > 280 && r.B - r.T < 360) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }

  /// Visible app windows other than the panel: the overlay, the HUD, the frame.
  public static int Extra(uint want, IntPtr panel) {
    int n = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == want && IsWindowVisible(h) && h != panel) n++;
      return true;
    }, IntPtr.Zero);
    return n;
  }

  /// Does this window class exist in the process? The hotkey and tray libraries
  /// each own a hidden message window, so their presence is checkable.
  public static bool HasClass(uint want, string wanted) {
    bool found = false;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != want) return true;
      var c = new StringBuilder(64); GetClassNameW(h, c, 64);
      if (c.ToString() == wanted) { found = true; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@ -ErrorAction SilentlyContinue

$proc = Get-Process -Name RapidCap -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Host "FAIL  RapidCap is not running"; exit 1 }
$exe = $proc.Path
$panel = [RcSys]::Panel([uint32]$proc.Id)
if ($panel -eq [IntPtr]::Zero) { Write-Host "FAIL  the panel is not on screen"; exit 1 }

$script:pass = 0
$script:fail = 0
$script:skip = 0
function Check([string]$name, [bool]$ok, [string]$detail) {
  if ($ok) { $script:pass++; Write-Host ("  PASS  {0}" -f $name) }
  else { $script:fail++; Write-Host ("  FAIL  {0} - {1}" -f $name, $detail) -ForegroundColor Red }
}
function Skip([string]$name, [string]$why) {
  $script:skip++
  Write-Host ("  SKIP  {0} - {1}" -f $name, $why) -ForegroundColor Yellow
}
function Extra { [RcSys]::Extra([uint32]$proc.Id, $panel) }

Write-Host ""
Write-Host "RapidCap - the parts Windows owns" -ForegroundColor Cyan

if ((Extra) -gt 0) {
  Write-Host "FAIL  another RapidCap window is up - restart the app and try again"
  exit 1
}

# Can this session take injected keystrokes at all? A remote-desktop host eats
# them, and then every hotkey check below would report a failure that says more
# about the session than the app.
[RcSys]::keybd_event(0xA0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 150
$keysWork = ([RcSys]::GetAsyncKeyState(0xA0) -band 0x8000) -ne 0
[RcSys]::keybd_event(0xA0, 0, 2, [IntPtr]::Zero)
Start-Sleep -Milliseconds 150

# --- global hotkeys --------------------------------------------------------
Write-Host ""
Write-Host "Global hotkeys"
$log = Join-Path $env:LOCALAPPDATA ("RapidCap\Logs\rapidcap.log." + (Get-Date -Format 'yyyy-MM-dd'))
$taken = @()
if (Test-Path $log) {
  $lines = Get-Content $log
  $lastStart = ($lines | Select-String -Pattern 'RapidCap startup' | Select-Object -Last 1).LineNumber
  if ($lastStart) {
    $taken = @($lines[($lastStart - 1)..($lines.Count - 1)] |
      Select-String -Pattern 'global hotkey unavailable' -AllMatches |
      ForEach-Object { if ($_ -match 'key: (\w+)') { $matches[1] } })
  }
}
Check "the hotkey manager has a message window" ([RcSys]::HasClass([uint32]$proc.Id, "global_hotkey_app")) "no global_hotkey_app window"
Check "the app survives hotkeys another program already owns" ($taken.Count -lt 5) "all five hotkeys were refused"
if ($taken.Count -gt 0) {
  Write-Host ("  NOTE  {0} of 5 hotkeys are registered to other software on this machine: {1}" -f $taken.Count, ($taken -join ', ')) -ForegroundColor DarkYellow
}
if (-not $keysWork) {
  Skip "a registered hotkey fires" "this session swallows synthetic keystrokes - GetAsyncKeyState never sees them"
} elseif ($taken.Count -ge 5) {
  Skip "a registered hotkey fires" "no hotkey registered, nothing to fire"
} else {
  # Shift+Alt+PrintScreen is the video hotkey and the one least often taken.
  [RcSys]::keybd_event(0xA0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
  [RcSys]::keybd_event(0xA4, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
  [RcSys]::keybd_event(0x2C, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 80
  [RcSys]::keybd_event(0x2C, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
  [RcSys]::keybd_event(0xA4, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
  [RcSys]::keybd_event(0xA0, 0, 2, [IntPtr]::Zero)
  Start-Sleep -Seconds 2
  $opened = (Extra) -gt 0
  Check "a registered hotkey fires" $opened "Shift+Alt+PrintScreen opened nothing"
  if ($opened) {
    [RcSys]::keybd_event(0x1B, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
    [RcSys]::keybd_event(0x1B, 0, 2, [IntPtr]::Zero); Start-Sleep -Seconds 2
    Check "Esc closes what the hotkey opened" ((Extra) -eq 0) "the overlay stayed up"
  }
}

# --- tray ------------------------------------------------------------------
Write-Host ""
Write-Host "Tray"
Check "the tray icon is registered" ([RcSys]::HasClass([uint32]$proc.Id, "tray_icon_app")) "no tray_icon_app window"

# --- keyboard navigation ---------------------------------------------------
Write-Host ""
Write-Host "Keyboard"
if (-not $keysWork) {
  Skip "Tab then Enter fires the focused card" "this session swallows synthetic keystrokes"
} elseif ([RcSys]::GetForegroundWindow() -ne $panel) {
  Skip "Tab then Enter fires the focused card" "the panel cannot take the foreground in this session"
} else {
  [RcSys]::keybd_event(0x09, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
  [RcSys]::keybd_event(0x09, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 400
  [RcSys]::keybd_event(0x0D, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
  [RcSys]::keybd_event(0x0D, 0, 2, [IntPtr]::Zero); Start-Sleep -Seconds 2
  Check "Tab then Enter fires the focused card" ((Extra) -gt 0) "no overlay after Tab, Enter"
  [RcSys]::keybd_event(0x1B, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
  [RcSys]::keybd_event(0x1B, 0, 2, [IntPtr]::Zero); Start-Sleep -Seconds 2
}

# --- single instance -------------------------------------------------------
Write-Host ""
Write-Host "Single instance"
$before = @(Get-Process -Name RapidCap -ErrorAction SilentlyContinue).Count
Start-Process $exe
Start-Sleep -Seconds 3
$after = @(Get-Process -Name RapidCap -ErrorAction SilentlyContinue).Count
Check "a second launch does not leave a second copy running" ($after -eq $before) "$before process(es) became $after"
Check "the panel survives the second launch" ([RcSys]::Panel([uint32]$proc.Id) -ne [IntPtr]::Zero) "the panel went away"

# --- probe contract --------------------------------------------------------
Write-Host ""
Write-Host "Probe"
$probe = & $exe --probe | Out-String
$json = $null
try { $json = $probe | ConvertFrom-Json } catch {}
Check "--probe prints JSON" ($null -ne $json) "not parseable: $probe"
if ($json) {
  Check "--probe reports the app id" ($json.app_id -eq 'com.inspire.rapidcap') "got $($json.app_id)"
  Check "--probe reports the output folder" ($json.output -like '*RapidCap*Screenshots*') "got $($json.output)"
  Check "--probe lists five hotkeys" ($json.hotkeys.Count -eq 5) "got $($json.hotkeys.Count)"
}

Write-Host ""
Write-Host ("  {0} passed, {1} failed, {2} skipped" -f $script:pass, $script:fail, $script:skip)
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
