# 远程诊断：枚举 tokenmeter 进程的顶层窗口 + 截取桌面（须在交互会话中运行）
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinEnum {
  public delegate bool CB(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(CB cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int m);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@

$proc = Get-Process tokenmeter -ErrorAction SilentlyContinue | Select-Object -First 1
$out = "pid=" + $(if ($proc) { $proc.Id } else { "none" }) + "`r`n"
if ($proc) {
  $list = @()
  [WinEnum]::EnumWindows({ param($h, $l)
    [uint32]$p = 0
    [WinEnum]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
    if ($p -eq $proc.Id) {
      $sb = New-Object System.Text.StringBuilder 512
      [WinEnum]::GetWindowText($h, $sb, 512) | Out-Null
      $script:list += "hwnd=$h visible=$([WinEnum]::IsWindowVisible($h)) title='$($sb.ToString())'"
    }
    return $true
  }, [IntPtr]::Zero) | Out-Null
  $out += ($list -join "`r`n")
}
[System.IO.File]::WriteAllText("C:\Users\hangbits\tokenmeter\wininfo.txt", $out)

$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
$bmp.Save("C:\Users\hangbits\tokenmeter\screen.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
