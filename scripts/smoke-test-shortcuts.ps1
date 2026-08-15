# Smoke test: F11 toggles fullscreen, Esc exits fullscreen.
# Covers both the console page and the remote Harness page (waits for navigation).
# Usage: pwsh scripts/smoke-test-shortcuts.ps1
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left, Top, Right, Bottom; }
public class Win32Rect {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
}
"@

function Get-RectStr($hWnd) {
  $r = New-Object RECT
  if (-not $hWnd -or -not [Win32Rect]::GetWindowRect($hWnd, [ref]$r)) { return "0,0 0x0" }
  return "{0},{1} {2}x{3}" -f $r.Left, $r.Top, ($r.Right - $r.Left), ($r.Bottom - $r.Top)
}

# Click the window center so the webview gets keyboard focus (AppActivate alone is not enough)
function Give-WebviewFocus($h) {
  $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  [Win32Rect]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 300
  [Win32Rect]::SetCursorPos([int](($screen.Left + $screen.Right) / 2), [int](($screen.Top + $screen.Bottom) / 2)) | Out-Null
  Start-Sleep -Milliseconds 200
  [Win32Rect]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)  # left down
  [Win32Rect]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)  # left up
  Start-Sleep -Milliseconds 500
}

$exe = "E:\Programing\xzy-dsh-desktop\src-tauri\target\release\dsh-desktop.exe"
$p = Start-Process -FilePath $exe -PassThru
$ws = New-Object -ComObject WScript.Shell
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$fsStr = "$($screen.Left),$($screen.Top) $($screen.Width)x$($screen.Height)"

$fail = 0
function Check($label, $cond) {
  if ($cond) { Write-Host "PASS: $label" } else { Write-Host "FAIL: $label"; $script:fail = 1 }
}

# Wait until the real (largest) window of the process settles.
# MainWindowHandle must be refreshed or it can point at the 14x14 placeholder.
$stable = $null; $h = 0
for ($i = 1; $i -le 20; $i++) {
  Start-Sleep -Milliseconds 500
  $p.Refresh()
  $h = $p.MainWindowHandle
  $r = Get-RectStr $h
  if ($r -eq $stable -and $r -ne "0,0 0x0" -and ($r -split "x")[1] -as [int] -gt 200) { break }
  $stable = $r
}
Write-Host "initial window: $stable  (screen: $fsStr)"
Check "window is not fullscreen initially" ($stable -ne $fsStr)

# Wait for the dsh web service on 3081, then give the app time to navigate to Harness
$svc = $null
for ($i = 1; $i -le 30; $i++) {
  Start-Sleep -Seconds 1
  $svc = Get-NetTCPConnection -LocalPort 3081 -State Listen -ErrorAction SilentlyContinue
  if ($svc) { break }
}
if (-not $svc) { Write-Host "WARN: dsh web (3081) did not come up; shortcuts may only be tested on console page" }
Write-Host "dsh web up after ~$i s; waiting 12s for auto-navigation to Harness..."
Start-Sleep -Seconds 12

# Helper: click to focus webview, send key, wait, return new rect
function SendKeyAndRect($key, $hWnd) {
  Give-WebviewFocus $hWnd
  $ws.SendKeys($key) | Out-Null
  Start-Sleep -Milliseconds 2000
  $p.Refresh()
  $h = $p.MainWindowHandle
  return Get-RectStr $h
}

Write-Host "--- now on Harness page ---"
$r1 = SendKeyAndRect '{F11}' $h
Write-Host "after F11: $r1"
Check "F11 enters fullscreen (covers screen)" ($r1 -eq $fsStr)

$r2 = SendKeyAndRect '{F11}' $h
Write-Host "after 2nd F11: $r2"
Check "F11 exits fullscreen (windowed again)" ($r2 -ne $fsStr)

$r3 = SendKeyAndRect '{F11}' $h
Check "pre-Esc: fullscreen entered" ($r3 -eq $fsStr)

$r4 = SendKeyAndRect '{ESC}' $h
Write-Host "after Esc: $r4"
Check "Esc exits fullscreen" ($r4 -ne $fsStr)

# Cleanup: kill app and any leftover dsh web service on port 3081
Get-Process -Name dsh-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 1500
$conn = Get-NetTCPConnection -LocalPort 3081 -State Listen -ErrorAction SilentlyContinue
if ($conn) {
  $conn | Select-Object -ExpandProperty OwningProcess -Unique | ForEach-Object {
    taskkill /PID $_ /T /F 2>$null | Out-Null
    Write-Host "cleaned leftover port-3081 process PID $_"
  }
}
if ($fail -eq 0) { Write-Host "=== ALL PASS ===" } else { Write-Host "=== FAILURES ==="; exit 1 }
