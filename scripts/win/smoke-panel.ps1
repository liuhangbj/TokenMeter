param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$OutDir
)

$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class TokenMeterWindowProbe {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int maxCount);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")]
    private static extern bool SystemParametersInfo(uint action, uint param, out RECT value, uint flags);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    public static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);

    public static IntPtr FindVisibleTokenMeterWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var title = new StringBuilder(256);
            GetWindowText(hwnd, title, title.Capacity);
            if (title.ToString() == "TokenMeter") {
                found = hwnd;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static RECT PrimaryWorkArea() {
        RECT rect;
        if (!SystemParametersInfo(0x0030, 0, out rect, 0)) {
            throw new InvalidOperationException("SPI_GETWORKAREA failed");
        }
        return rect;
    }
}
"@

$env:TOKENMETER_AUTO_PANEL = "1"
$env:TOKENMETER_LOG_FILE = Join-Path $OutDir "visible-app.log"
$process = Start-Process -FilePath $Exe -PassThru

try {
  Start-Sleep -Seconds 8
  $process.Refresh()
  if ($process.HasExited) { throw "App 启动可见面板后退出，code=$($process.ExitCode)" }

  $hwnd = [TokenMeterWindowProbe]::FindVisibleTokenMeterWindow([uint32]$process.Id)
  if ($hwnd -eq [IntPtr]::Zero) { throw "未找到标题为 TokenMeter 的可见窗口" }

  $rect = New-Object TokenMeterWindowProbe+RECT
  if (-not [TokenMeterWindowProbe]::GetWindowRect($hwnd, [ref]$rect)) {
    throw "GetWindowRect 失败"
  }
  $work = [TokenMeterWindowProbe]::PrimaryWorkArea()
  $dpi = [TokenMeterWindowProbe]::GetDpiForWindow($hwnd)
  if ($dpi -eq 0) { $dpi = 96 }
  $scale = $dpi / 96.0

  $width = $rect.Right - $rect.Left
  $height = $rect.Bottom - $rect.Top
  $expectedWidth = [math]::Round(380 * $scale)
  $minimumHeight = [math]::Round(120 * $scale)
  $maximumHeight = [math]::Round(800 * $scale)
  $rightGap = $work.Right - $rect.Right
  $bottomGap = $work.Bottom - $rect.Bottom
  $exStyle = [TokenMeterWindowProbe]::GetWindowLongPtr($hwnd, -20).ToInt64()

  $lines = @(
    "pid=$($process.Id)",
    "hwnd=$hwnd",
    "dpi=$dpi scale=$scale",
    "rect=($($rect.Left),$($rect.Top))-($($rect.Right),$($rect.Bottom)) width=$width height=$height",
    "workArea=($($work.Left),$($work.Top))-($($work.Right),$($work.Bottom))",
    "rightGap=$rightGap bottomGap=$bottomGap",
    "exStyle=0x$($exStyle.ToString('X'))"
  )
  $lines | Set-Content -Path (Join-Path $OutDir "window.txt")
  $lines | ForEach-Object { Write-Output $_ }

  # 无装饰窗口仍可能包含少量 DWM 边界，保留 28px 物理像素容差。
  if ([math]::Abs($width - $expectedWidth) -gt 28) {
    throw "窗口宽度异常：actual=$width expected=$expectedWidth"
  }
  # 高度改为内容自适应；这里只验证原生约束范围，具体内容高度由前端测量决定。
  if ($height -lt ($minimumHeight - 28) -or $height -gt ($maximumHeight + 28)) {
    throw "窗口自适应高度异常：actual=$height expectedRange=$minimumHeight..$maximumHeight"
  }
  if ($rightGap -lt -4 -or $rightGap -gt 32) {
    throw "窗口未贴近工作区右边：gap=$rightGap"
  }
  if ($bottomGap -lt -4 -or $bottomGap -gt 32) {
    throw "窗口未贴近任务栏上方：gap=$bottomGap"
  }

  $bitmap = New-Object System.Drawing.Bitmap $width, $height
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  try {
    $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $OutDir "panel.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  } finally {
    $graphics.Dispose()
    $bitmap.Dispose()
  }
  Write-Output "PASS: Windows 可见面板自适应尺寸与右下角定位正确"
} finally {
  Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
  Remove-Item Env:TOKENMETER_AUTO_PANEL -ErrorAction SilentlyContinue
  Remove-Item Env:TOKENMETER_LOG_FILE -ErrorAction SilentlyContinue
}
