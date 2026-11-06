# Builds the release exe and the Inno Setup installer into dist\.
# Usage: powershell -ExecutionPolicy Bypass -File installer\build.ps1 [-SkipCargo] [-Iscc <path>]
# Honors CARGO_TARGET_DIR (the exe is taken from <target>\release).
param(
    [switch]$SkipCargo,
    [string]$Iscc
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$native = Join-Path $root 'native'

if (-not $SkipCargo) {
    Push-Location $native
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $native 'target' }
$exe = Join-Path $target 'release\wow-chat-translator.exe'
if (-not (Test-Path $exe)) { throw "Release exe not found: $exe" }

if (-not $Iscc) {
    $found = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($found) {
        $Iscc = $found.Source
    } else {
        $Iscc = @(
            (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
            (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
            (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
        ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    }
}
if (-not $Iscc) { throw 'ISCC.exe (Inno Setup 6) not found. Install it from https://jrsoftware.org/isdl.php' }

& $Iscc "/DSourceExe=$exe" (Join-Path $PSScriptRoot 'wow-chat-translator.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
Get-Item (Join-Path $root 'dist\WoW-Chat-Translator-Setup-*.exe') | Select-Object Name, Length
