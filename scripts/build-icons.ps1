# Renders the icon SVGs with headless Edge and builds the app icon set:
#   app/src-tauri/icons/*   (.ico + PNGs, via `tauri icon`)
#   app/src-tauri/icons/tray-{on,off,error}.png   (32 px, notification area)
#   app/ui/public/icon.svg  (title bar)
param(
    [string]$Edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$root = Split-Path $PSScriptRoot -Parent
$iconDir = Join-Path $root "assets\icon"
$build = Join-Path $root "target\icons"
$tauriIcons = Join-Path $root "app\src-tauri\icons"
New-Item -ItemType Directory -Force $build, $tauriIcons, (Join-Path $root "app\ui\public") | Out-Null

node (Join-Path $PSScriptRoot "gen-app-icon.mjs") $iconDir

function Render([string]$Svg, [string]$Png) {
    $url = "file:///" + ($Svg -replace '\\', '/')
    Start-Process -FilePath $Edge -Wait -NoNewWindow -ArgumentList @(
        "--headless=new", "--disable-gpu", "--hide-scrollbars",
        "--user-data-dir=`"$build\edge-profile`"",
        "--default-background-color=00000000", "--window-size=1024,1024",
        "--screenshot=`"$Png`"", $url)
}

function Shrink([string]$Source, [string]$Target, [int]$Size) {
    $src = [System.Drawing.Bitmap]::new($Source)
    $dst = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($dst)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($src, 0, 0, $Size, $Size)
    $dst.Save($Target, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $dst.Dispose(); $src.Dispose()
}

$appPng = Join-Path $build "outerspace-1024.png"
Render (Join-Path $iconDir "outerspace.svg") $appPng
Copy-Item $appPng (Join-Path $iconDir "outerspace-1024.png") -Force

# Windows PowerShell turns native stderr into terminating errors under "Stop"; judge by exit code instead.
$ErrorActionPreference = "Continue"
& (Join-Path $root "app\node_modules\.bin\tauri.cmd") icon $appPng -o $tauriIcons
if ($LASTEXITCODE -ne 0) { throw "tauri icon failed ($LASTEXITCODE)" }
$ErrorActionPreference = "Stop"
# Windows-only app: drop the mobile, macOS and Store assets `tauri icon` always emits.
Get-ChildItem $tauriIcons | Where-Object { $_.Name -match '^(android|ios|icon\.icns|Square.*|StoreLogo\.png)$' } |
    Remove-Item -Recurse -Force

foreach ($state in "on", "off", "error") {
    $png = Join-Path $build "tray-$state-1024.png"
    Render (Join-Path $iconDir "tray-$state.svg") $png
    Shrink $png (Join-Path $tauriIcons "tray-$state.png") 32
}

Copy-Item (Join-Path $iconDir "outerspace.svg") (Join-Path $root "app\ui\public\icon.svg") -Force
Write-Host "icons ready in $tauriIcons"
