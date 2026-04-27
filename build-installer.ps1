# Builds the release binary then compiles the Inno Setup installer.
#
# Requirements:
#   - Rust + MSVC toolchain  (cargo build --release)
#   - Inno Setup 6           https://jrsoftware.org/isinfo.php
#
# Usage:
#   .\build-installer.ps1
#   .\build-installer.ps1 -IsccPath "C:\Tools\InnoSetup6\iscc.exe"

param([string]$IsccPath = "")

$ErrorActionPreference = "Stop"

Write-Host "Building release binary..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (-not $IsccPath) {
    $candidates = @(
        "$env:ProgramFiles\Inno Setup 6\iscc.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\iscc.exe"
    )
    $IsccPath = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}

if (-not $IsccPath) {
    Write-Error "iscc.exe not found. Install Inno Setup 6 or pass -IsccPath 'C:\path\to\iscc.exe'."
    exit 1
}

$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
Write-Host "Version: $version" -ForegroundColor Cyan

Write-Host "Compiling installer ($IsccPath)..." -ForegroundColor Cyan
& $IsccPath /DAppVersion=$version installer.iss
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "Done: installer\DictashipSetup.exe" -ForegroundColor Green
