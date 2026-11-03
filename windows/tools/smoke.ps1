# Windows smoke test (CI): demo folders in a throwaway data dir → launch each screen → screenshot of the app window.
# Never touches the runner user's real folders: MF_DATA_DIR points to a temp dir, folders are demo ones.
param(
  [string]$Exe = "target\release\multi-finder.exe",
  [string]$Out = "smoke",
  [string]$Locale = "en-US",
  [switch]$WithModel
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class MfWin {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Ansi)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmDeviceName;
    public short dmSpecVersion; public short dmDriverVersion; public short dmSize; public short dmDriverExtra;
    public int dmFields; public int dmPositionX; public int dmPositionY; public int dmDisplayOrientation; public int dmDisplayFixedOutput;
    public short dmColor; public short dmDuplex; public short dmYResolution; public short dmTTOption; public short dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string dmFormName;
    public short dmLogPixels; public int dmBitsPerPel; public int dmPelsWidth; public int dmPelsHeight;
    public int dmDisplayFlags; public int dmDisplayFrequency; public int dmICMMethod; public int dmICMIntent;
    public int dmMediaType; public int dmDitherType; public int dmReserved1; public int dmReserved2;
    public int dmPanningWidth; public int dmPanningHeight;
  }
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern int EnumDisplaySettings(string deviceName, int modeNum, ref DEVMODE devMode);
  [DllImport("user32.dll")] public static extern int ChangeDisplaySettings(ref DEVMODE devMode, int flags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  public static int SetResolution(int w, int h) {
    DEVMODE dm = new DEVMODE();
    dm.dmSize = (short)Marshal.SizeOf(typeof(DEVMODE));
    if (EnumDisplaySettings(null, -1, ref dm) == 0) return -100;
    dm.dmPelsWidth = w; dm.dmPelsHeight = h; dm.dmFields = 0x80000 | 0x100000;
    return ChangeDisplaySettings(ref dm, 0);
  }
}
"@

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$root = Join-Path $env:RUNNER_TEMP "mf-smoke"
if (-not $env:RUNNER_TEMP) { $root = Join-Path $env:TEMP "mf-smoke" }
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
$data = Join-Path $root "data"
$dl = Join-Path $root "Downloads"
$desk = Join-Path $root "Desktop"
New-Item -ItemType Directory -Force -Path $data, $dl, $desk | Out-Null

function New-RandomFile([string]$Path, [int]$Size, [datetime]$When) {
  New-Item -ItemType Directory -Force -Path (Split-Path $Path) | Out-Null
  $bytes = New-Object byte[] $Size
  (New-Object System.Random ($Path.GetHashCode())).NextBytes($bytes)
  [IO.File]::WriteAllBytes($Path, $bytes)
  (Get-Item $Path).LastWriteTime = $When
}
function New-TextFile([string]$Path, [string]$Text, [datetime]$When) {
  New-Item -ItemType Directory -Force -Path (Split-Path $Path) | Out-Null
  [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
  (Get-Item $Path).LastWriteTime = $When
}
function Copy-Same([string]$From, [string]$To) {
  New-Item -ItemType Directory -Force -Path (Split-Path $To) | Out-Null
  Copy-Item $From $To
  (Get-Item $To).LastWriteTime = (Get-Item $From).LastWriteTime
}

# demo content (invented names only)
New-TextFile "$dl\Q4 sales report.txt" "Q4 2026 Sales Report. Revenue by region, top accounts and the pipeline for next quarter." (Get-Date "2026-10-29 10:10")
New-TextFile "$dl\assignment-3-essay.txt" "Modern Philosophy (PHIL 210). Assignment 3: an essay on Descartes' Meditations. Due Friday." (Get-Date "2026-10-20 19:30")
New-TextFile "$dl\meeting-notes.md" "# Weekly team meeting`n- Launch checklist`n- Customer feedback review" (Get-Date "2026-10-30 17:00")
New-TextFile "$dl\kimchi-stew-recipe.md" "# Kimchi stew`nAged kimchi, pork belly, tofu, green onion. Simmer 20 minutes." (Get-Date "2026-09-12 19:00")
New-TextFile "$dl\resize_images.py" "from PIL import Image`nimport sys`nfor p in sys.argv[1:]:`n    Image.open(p).resize((800, 600)).save(p)" (Get-Date "2026-10-21 15:30")
New-RandomFile "$dl\Invoice-2026-10.pdf" 184320 (Get-Date "2026-10-28 10:12")
New-RandomFile "$dl\Screenshot 2026-10-28 101244.png" 942080 (Get-Date "2026-10-28 10:12")
New-RandomFile "$dl\Figma-125.0-setup.exe" 4194304 (Get-Date "2026-10-15 09:30")
New-RandomFile "$dl\Zoom-installer.msi" 3145728 (Get-Date "2026-09-20 11:00")
New-RandomFile "$dl\wallpaper-4k.jpg" 2621440 (Get-Date "2026-08-14 12:00")
New-RandomFile "$dl\Design assets\mockup-home.png" 1572864 (Get-Date "2026-10-01 10:05")
New-RandomFile "$dl\Lecture notes\week-01.pdf" 1048576 (Get-Date "2026-09-03 14:00")
Copy-Same "$dl\Figma-125.0-setup.exe" "$desk\Installers\Figma-125.0-setup.exe"
Copy-Same "$dl\wallpaper-4k.jpg" "$desk\wallpaper (1).jpg"
foreach ($i in 1001..1008) {
  New-RandomFile "$desk\Photos 2025\IMG_$i.jpg" (2000000 + $i * 911) (Get-Date "2025-07-15 12:00")
  Copy-Same "$desk\Photos 2025\IMG_$i.jpg" "$desk\Photos backup\IMG_$i.jpg"
}
New-RandomFile "$desk\Photos 2025\IMG_1009.jpg" 2310000 (Get-Date "2025-07-20 12:00")
New-RandomFile "$desk\Photos backup\IMG_1009.jpg" 2290000 (Get-Date "2025-07-18 12:00")
New-RandomFile "$desk\Photos 2025\IMG_1010.jpg" 2410000 (Get-Date "2025-08-01 12:00")
New-RandomFile "$desk\Photos 2025\IMG_1011.jpg" 2420000 (Get-Date "2025-08-01 12:05")
New-RandomFile "$desk\Photos backup\IMG_0999.jpg" 2380000 (Get-Date "2025-07-10 12:00")
New-RandomFile "$desk\Photos 2025\IMG_1012.jpg" 2450000 (Get-Date "2025-08-02 12:00")
Copy-Same "$desk\Photos 2025\IMG_1012.jpg" "$desk\Photos backup\edited\IMG_1012.jpg"

$settings = @{ folders = @($dl, $desk) } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $data "settings.json"), $settings)

if ($WithModel -and $env:MF_MODEL_FILE) {
  New-Item -ItemType Directory -Force -Path (Join-Path $data "models") | Out-Null
  Copy-Item $env:MF_MODEL_FILE (Join-Path $data "models\Qwen3-0.6B-Q8_0.gguf")
}

# a bigger screen than the runner's default 1024x768, so the 1280x800 window fits
foreach ($mode in @(@(1920, 1080), @(1600, 900), @(1440, 900), @(1280, 1024))) {
  $r = [MfWin]::SetResolution($mode[0], $mode[1])
  Write-Host "resolution $($mode[0])x$($mode[1]): $r"
  if ($r -eq 0) { break }
}
Start-Sleep -Seconds 2
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Host "screen $($screen.Width)x$($screen.Height)"

function Save-Image([System.Drawing.Rectangle]$Rect, [string]$Name) {
  $bmp = [System.Drawing.Bitmap]::new($Rect.Width, $Rect.Height)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($Rect.Location, [System.Drawing.Point]::Empty, $Rect.Size)
  $bmp.Save((Join-Path $Out "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}

$env:MF_DATA_DIR = $data
$env:MF_LOCALE = $Locale
$exePath = (Resolve-Path $Exe).Path
$failures = @()

function Invoke-Shot([string]$Name, [string[]]$AppArgs, [int]$Wait) {
  # Start-Process joins an argument array with spaces without quoting — quote the ones that contain spaces
  $line = ($AppArgs | ForEach-Object { if ($_ -match '\s') { '"' + $_ + '"' } else { $_ } }) -join ' '
  Write-Host "== $Name ($line)"
  $p = if ($line) {
    Start-Process -FilePath $exePath -ArgumentList $line -PassThru `
      -RedirectStandardError (Join-Path $Out "$Name.stderr.txt") -RedirectStandardOutput (Join-Path $Out "$Name.stdout.txt")
  } else {
    Start-Process -FilePath $exePath -PassThru `
      -RedirectStandardError (Join-Path $Out "$Name.stderr.txt") -RedirectStandardOutput (Join-Path $Out "$Name.stdout.txt")
  }
  Start-Sleep -Seconds $Wait
  if ($p.HasExited) {
    $script:failures += "$Name exited early with code $($p.ExitCode)"
    return
  }
  $p.Refresh()
  $h = $p.MainWindowHandle
  # keep the pointer off the app so no tooltip shows up in the picture
  [System.Windows.Forms.Cursor]::Position = [System.Drawing.Point]::new(1, 1)
  Start-Sleep -Milliseconds 600
  $rect = New-Object 'MfWin+RECT'
  if ($h -ne [IntPtr]::Zero -and [MfWin]::SetForegroundWindow($h) -and [MfWin]::GetWindowRect($h, [ref]$rect)) {
    Start-Sleep -Milliseconds 400
    $w = [Math]::Min($rect.Right, $screen.Width) - [Math]::Max($rect.Left, 0)
    $hh = [Math]::Min($rect.Bottom, $screen.Height) - [Math]::Max($rect.Top, 0)
    Save-Image ([System.Drawing.Rectangle]::new([Math]::Max($rect.Left, 0), [Math]::Max($rect.Top, 0), $w, $hh)) $Name
  } else {
    Save-Image $screen $Name
  }
  Save-Image $screen "$Name-screen"
  Stop-Process -Id $p.Id -Force
  Start-Sleep -Seconds 2
}

Invoke-Shot "1-all-folders" @("--screen", "collection") 15
Invoke-Shot "2-duplicates" @("--screen", "duplicates", "--run") 15
Invoke-Shot "3-compare" @("--screen", "compare", "--left", "$desk\Photos 2025", "--right", "$desk\Photos backup", "--run") 15
Invoke-Shot "4-search" @("--search", "IMG_101") 10
Invoke-Shot "5-leftovers" @("--screen", "leftovers", "--run") 20
if ($WithModel) {
  Invoke-Shot "6-categories" @("--screen", "categories", "--run") 150
} else {
  Invoke-Shot "6-categories" @("--screen", "categories") 10
}

Get-ChildItem $data | Format-Table Name, Length | Out-String | Set-Content (Join-Path $Out "data-dir.txt")
if ($failures.Count -gt 0) {
  $failures | Set-Content (Join-Path $Out "failures.txt")
  Write-Error ($failures -join "`n")
}
Write-Host "smoke OK"
