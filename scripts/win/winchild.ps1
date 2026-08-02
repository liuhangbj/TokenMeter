param([int]$TargetPid = 0)
# Diagnostic: list tokenmeter top-level windows and WebView2 child rects
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W2 {
  public delegate bool CB(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(CB cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, CB cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int m);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
if ($TargetPid -gt 0) {
  $proc = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
} else {
  $proc = Get-Process tokenmeter -ErrorAction SilentlyContinue |
    Where-Object { $_.SessionId -ne 0 } | Select-Object -First 1
}
if (-not $proc) { Write-Host "no process"; exit }
$tops = @()
[W2]::EnumWindows({ param($h, $l)
  [uint32]$p = 0
  [W2]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
  if ($p -eq $proc.Id) {
    $sb = New-Object System.Text.StringBuilder 256
    [W2]::GetWindowText($h, $sb, 256) | Out-Null
    $script:tops += ,@($h, $sb.ToString())
  }
  return $true
}, [IntPtr]::Zero) | Out-Null
foreach ($t in $tops) {
  $r = New-Object W2+RECT
  [W2]::GetWindowRect($t[0], [ref]$r) | Out-Null
  Write-Host "TOP hwnd=$($t[0]) title='$($t[1])' rect=($($r.Left),$($r.Top))-($($r.Right),$($r.Bottom)) w=$($r.Right-$r.Left) h=$($r.Bottom-$r.Top)"
  $kids = @()
  [W2]::EnumChildWindows($t[0], { param($h, $l)
    $cr = New-Object W2+RECT
    [W2]::GetWindowRect($h, [ref]$cr) | Out-Null
    $script:kids += "  child=$h rect=($($cr.Left),$($cr.Top))-($($cr.Right),$($cr.Bottom)) w=$($cr.Right-$cr.Left) h=$($cr.Bottom-$cr.Top)"
    return $true
  }, [IntPtr]::Zero) | Out-Null
  $kids | ForEach-Object { Write-Host $_ }
}
