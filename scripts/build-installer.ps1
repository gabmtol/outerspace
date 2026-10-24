# Builds the release app and its per-user NSIS installer (no admin needed), with outerspace-mcp.exe —
# the stdio bridge MCP clients launch — installed next to OuterSpace.exe.
#   powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1
# The installer lands in target/release/bundle/nsis/.
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent

function Invoke-Native([string]$File, [string[]]$Arguments, [string]$Directory) {
    # Windows PowerShell turns native stderr into terminating errors under "Stop"; judge by exit code.
    $ErrorActionPreference = "Continue"
    Push-Location $Directory
    try {
        & $File @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$File $Arguments failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

Invoke-Native "cargo" @("build", "--release", "-p", "outerspace-shim") $root

# Tauri sidecars are looked up with the target triple in their name; installed as outerspace-mcp.exe.
$binaries = Join-Path $root "app\src-tauri\binaries"
New-Item -ItemType Directory -Force $binaries | Out-Null
Copy-Item (Join-Path $root "target\release\outerspace-mcp.exe") (Join-Path $binaries "outerspace-mcp-x86_64-pc-windows-msvc.exe") -Force

Invoke-Native (Join-Path $root "app\node_modules\.bin\tauri.cmd") @("build", "--config", "src-tauri/tauri.bundle.json") (Join-Path $root "app")

Get-ChildItem (Join-Path $root "target\release\bundle\nsis") -Filter *.exe | ForEach-Object { "installer: $($_.FullName)" }
