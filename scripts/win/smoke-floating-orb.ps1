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
using System.Threading;

public static class TokenMeterFloatingOrbProbe {
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
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    public static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    private static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
    [DllImport("user32.dll")]
    private static extern bool SystemParametersInfo(uint action, uint param, out RECT value, uint flags);

    private const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    private const uint MOUSEEVENTF_LEFTUP = 0x0004;
    private const uint MOUSEEVENTF_MOVE = 0x0001;

    public static IntPtr Find(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != processId || !IsWindowVisible(hwnd)) return true;
            var title = new StringBuilder(256);
            GetWindowText(hwnd, title, title.Capacity);
            if (title.ToString() == "TokenMeter Floating Orb") {
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

    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(80);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(80);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
    }

    public static void Drag(int startX, int startY, int endX, int endY) {
        SetCursorPos(startX, startY);
        Thread.Sleep(100);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
        for (int step = 1; step <= 18; step++) {
            int x = startX + ((endX - startX) * step / 18);
            int y = startY + ((endY - startY) * step / 18);
            SetCursorPos(x, y);
            mouse_event(MOUSEEVENTF_MOVE, 0, 0, 0, UIntPtr.Zero);
            Thread.Sleep(35);
        }
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
    }
}
"@

function Get-OrbRect([IntPtr]$Hwnd) {
  $rect = New-Object TokenMeterFloatingOrbProbe+RECT
  if (-not [TokenMeterFloatingOrbProbe]::GetWindowRect($Hwnd, [ref]$rect)) {
    throw "GetWindowRect 失败"
  }
  return $rect
}

function Wait-OrbSize([IntPtr]$Hwnd, [double]$LogicalWidth, [double]$LogicalHeight, [double]$Scale) {
  $expectedWidth = [math]::Round($LogicalWidth * $Scale)
  $expectedHeight = [math]::Round($LogicalHeight * $Scale)
  for ($attempt = 0; $attempt -lt 40; $attempt++) {
    $rect = Get-OrbRect $Hwnd
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ([math]::Abs($width - $expectedWidth) -le 18 -and
        [math]::Abs($height - $expectedHeight) -le 18) {
      return $rect
    }
    Start-Sleep -Milliseconds 125
  }
  throw "悬浮球尺寸未切换到 ${LogicalWidth}x${LogicalHeight} logical px"
}

$env:TOKENMETER_AUTO_ORB = "1"
$env:TOKENMETER_LOG_FILE = Join-Path $OutDir "floating-orb.log"
$foregroundBefore = [TokenMeterFloatingOrbProbe]::GetForegroundWindow()
$process = Start-Process -FilePath $Exe -PassThru

try {
  $hwnd = [IntPtr]::Zero
  for ($attempt = 0; $attempt -lt 60; $attempt++) {
    $process.Refresh()
    if ($process.HasExited) { throw "App 启动悬浮球后退出，code=$($process.ExitCode)" }
    $hwnd = [TokenMeterFloatingOrbProbe]::Find([uint32]$process.Id)
    if ($hwnd -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 250
  }
  if ($hwnd -eq [IntPtr]::Zero) { throw "未找到 TokenMeter Floating Orb 可见窗口" }

  Start-Sleep -Seconds 2
  $dpi = [TokenMeterFloatingOrbProbe]::GetDpiForWindow($hwnd)
  if ($dpi -eq 0) { $dpi = 96 }
  $scale = $dpi / 96.0
  $expanded = Wait-OrbSize $hwnd 156 150 $scale
  $work = [TokenMeterFloatingOrbProbe]::PrimaryWorkArea()
  $rightGap = $work.Right - $expanded.Right
  $exStyle = [TokenMeterFloatingOrbProbe]::GetWindowLongPtr($hwnd, -20).ToInt64()

  $wsExTopmost = 0x00000008
  $wsExToolWindow = 0x00000080
  $wsExAppWindow = 0x00040000
  $wsExNoActivate = 0x08000000
  if (($exStyle -band $wsExTopmost) -eq 0) { throw "悬浮球不是置顶窗口" }
  if (($exStyle -band $wsExToolWindow) -eq 0) { throw "悬浮球缺少 TOOLWINDOW，可能出现在任务栏" }
  if (($exStyle -band $wsExAppWindow) -ne 0) { throw "悬浮球带有 APPWINDOW，会出现在任务栏" }
  if (($exStyle -band $wsExNoActivate) -eq 0) { throw "悬浮球会抢占前台焦点" }
  if ($rightGap -lt -4 -or $rightGap -gt 32) { throw "悬浮球未贴近工作区右边：gap=$rightGap" }
  if ([TokenMeterFloatingOrbProbe]::GetForegroundWindow() -eq $hwnd) {
    throw "悬浮球意外成为前台窗口"
  }

  # 默认 Classic 主题的收起按钮位于窗口右上区域。
  $collapseX = $expanded.Right - [math]::Round(33 * $scale)
  $collapseY = $expanded.Top + [math]::Round(36 * $scale)
  [TokenMeterFloatingOrbProbe]::Click($collapseX, $collapseY)
  $collapsed = Wait-OrbSize $hwnd 78 76 $scale

  # 普通点击必须展开；它和拖动共用同一个小球，这是 Windows 最容易丢 click 的路径。
  $collapsedCenterX = [math]::Round(($collapsed.Left + $collapsed.Right) / 2)
  $collapsedCenterY = [math]::Round(($collapsed.Top + $collapsed.Bottom) / 2)
  [TokenMeterFloatingOrbProbe]::Click($collapsedCenterX, $collapsedCenterY)
  $expandedAgain = Wait-OrbSize $hwnd 156 150 $scale

  # 再次收起后拖向左边，验证小球可移动并会吸附工作区边缘。
  $collapseX = $expandedAgain.Right - [math]::Round(33 * $scale)
  $collapseY = $expandedAgain.Top + [math]::Round(36 * $scale)
  [TokenMeterFloatingOrbProbe]::Click($collapseX, $collapseY)
  $collapsed = Wait-OrbSize $hwnd 78 76 $scale
  $startX = [math]::Round(($collapsed.Left + $collapsed.Right) / 2)
  $startY = [math]::Round(($collapsed.Top + $collapsed.Bottom) / 2)
  $endX = $work.Left + [math]::Round(90 * $scale)
  [TokenMeterFloatingOrbProbe]::Drag($startX, $startY, $endX, $startY)
  Start-Sleep -Milliseconds 900
  $dragged = Get-OrbRect $hwnd
  $leftGap = $dragged.Left - $work.Left
  if ($leftGap -lt -4 -or $leftGap -gt 36) {
    throw "收起悬浮球拖动后未吸附左边：gap=$leftGap"
  }

  $width = $dragged.Right - $dragged.Left
  $height = $dragged.Bottom - $dragged.Top
  $bitmap = New-Object System.Drawing.Bitmap $width, $height
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  try {
    $graphics.CopyFromScreen($dragged.Left, $dragged.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $OutDir "floating-orb-collapsed.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  } finally {
    $graphics.Dispose()
    $bitmap.Dispose()
  }

  $lines = @(
    "pid=$($process.Id)",
    "hwnd=$hwnd",
    "dpi=$dpi scale=$scale",
    "foregroundBefore=$foregroundBefore foregroundAfter=$([TokenMeterFloatingOrbProbe]::GetForegroundWindow())",
    "expanded=156x150 collapsed=78x76",
    "rightGap=$rightGap leftGapAfterDrag=$leftGap",
    "exStyle=0x$($exStyle.ToString('X'))"
  )
  $lines | Set-Content -Path (Join-Path $OutDir "floating-orb-window.txt")
  $lines | ForEach-Object { Write-Output $_ }
  Write-Output "PASS: Windows 悬浮球窗口、焦点、任务栏、收起展开、拖动吸附均正常"
} finally {
  Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
  Remove-Item Env:TOKENMETER_AUTO_ORB -ErrorAction SilentlyContinue
  Remove-Item Env:TOKENMETER_LOG_FILE -ErrorAction SilentlyContinue
}
