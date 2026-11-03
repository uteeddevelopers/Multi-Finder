# Windows smoke test (CI): demo folders in a throwaway data dir → launch each screen → screenshot.
# Never touches the runner user's real folders: MF_DATA_DIR points to a temp dir, folders are demo ones.
param(
  [string]$Exe = "target\release\multi-finder.exe",
  [string]$Out = "smoke",
  [string]$Locale = "ko-KR",
  [switch]$WithModel
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

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
function Copy-Same([string]$From, [string]$To) {
  New-Item -ItemType Directory -Force -Path (Split-Path $To) | Out-Null
  Copy-Item $From $To
  (Get-Item $To).LastWriteTime = (Get-Item $From).LastWriteTime
}

# demo content (invented names only)
$d = Get-Date "2026-10-28 10:12"
New-RandomFile "$dl\Invoice-2026-10.pdf" 184320 $d
New-RandomFile "$dl\Screenshot 2026-10-28 101244.png" 942080 $d
New-RandomFile "$dl\Figma-125.0-setup.exe" 4194304 (Get-Date "2026-10-15 09:30")
New-RandomFile "$dl\Zoom-installer.msi" 3145728 (Get-Date "2026-09-20 11:00")
New-RandomFile "$dl\wallpaper-4k.jpg" 2621440 (Get-Date "2026-08-14 12:00")
New-RandomFile "$dl\Design assets\mockup-home.png" 1572864 (Get-Date "2026-10-01 10:05")
New-RandomFile "$dl\Lecture notes\week-01.pdf" 1048576 (Get-Date "2026-09-03 14:00")
Set-Content -Path "$dl\project-brief.txt" -Value "Q4 project brief: goals, budget, schedule and the client contact list." -Encoding UTF8
Set-Content -Path "$dl\meeting-notes.md" -Value "# 주간 회의록`n- 신규 기능 일정 확인`n- 고객사 미팅 준비" -Encoding UTF8
Copy-Same "$dl\Figma-125.0-setup.exe" "$desk\Installers\Figma-125.0-setup.exe"
Copy-Same "$dl\wallpaper-4k.jpg" "$desk\wallpaper (1).jpg"
foreach ($i in 1001..1008) {
  New-RandomFile "$desk\Photos 2025\IMG_$i.jpg" (2000000 + $i * 911) (Get-Date "2025-07-15 12:00")
  Copy-Same "$desk\Photos 2025\IMG_$i.jpg" "$desk\Photos backup\IMG_$i.jpg"
}
New-RandomFile "$desk\Photos 2025\IMG_1009.jpg" 2310000 (Get-Date "2025-07-20 12:00")
New-RandomFile "$desk\Photos backup\IMG_1009.jpg" 2290000 (Get-Date "2025-07-18 12:00")
New-RandomFile "$desk\Photos 2025\IMG_1010.jpg" 2410000 (Get-Date "2025-08-01 12:00")
New-RandomFile "$desk\Photos backup\IMG_0999.jpg" 2380000 (Get-Date "2025-07-10 12:00")

$settings = @{ folders = @($dl, $desk) } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path $data "settings.json"), $settings)

if ($WithModel -and $env:MF_MODEL_FILE) {
  New-Item -ItemType Directory -Force -Path (Join-Path $data "models") | Out-Null
  Copy-Item $env:MF_MODEL_FILE (Join-Path $data "models\Qwen3-0.6B-Q8_0.gguf")
}

try { Set-DisplayResolution -Width 1600 -Height 1000 -Force -ErrorAction Stop } catch { Write-Host "resolution unchanged: $_" }
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Host "screen $($screen.Width)x$($screen.Height)"

function Save-Screen([string]$Name) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save((Join-Path $Out "$Name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
}

$env:MF_DATA_DIR = $data
$env:MF_LOCALE = $Locale
$exePath = (Resolve-Path $Exe).Path
$failures = @()

function Invoke-Shot([string]$Name, [string[]]$AppArgs, [int]$Wait) {
  Write-Host "== $Name ($($AppArgs -join ' '))"
  $p = Start-Process -FilePath $exePath -ArgumentList $AppArgs -PassThru `
    -RedirectStandardError (Join-Path $Out "$Name.stderr.txt") -RedirectStandardOutput (Join-Path $Out "$Name.stdout.txt")
  Start-Sleep -Seconds $Wait
  if ($p.HasExited) {
    $script:failures += "$Name exited early with code $($p.ExitCode)"
  } else {
    Save-Screen $Name
    Stop-Process -Id $p.Id -Force
    Start-Sleep -Seconds 2
  }
}

$noArgs = @("--screen", "collection")
Invoke-Shot "1-all-folders" $noArgs 15
Invoke-Shot "2-duplicates" @("--screen", "duplicates", "--run") 15
Invoke-Shot "3-compare" @("--screen", "compare", "--left", "$desk\Photos 2025", "--right", "$desk\Photos backup", "--run") 15
Invoke-Shot "4-search" @("--search", "IMG_10") 10
Invoke-Shot "5-leftovers" @("--screen", "leftovers", "--run") 20
if ($WithModel) {
  Invoke-Shot "6-categories" @("--screen", "categories", "--run") 150
} else {
  Invoke-Shot "6-categories" @("--screen", "categories") 10
}

# index sanity: the demo folders must be in the index
Get-ChildItem $data | Format-Table Name, Length | Out-String | Set-Content (Join-Path $Out "data-dir.txt")
if ($failures.Count -gt 0) {
  $failures | Set-Content (Join-Path $Out "failures.txt")
  Write-Error ($failures -join "`n")
}
Write-Host "smoke OK"
