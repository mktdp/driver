[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "windows_common.ps1")

Write-Host "=== mktdp-driver Windows setup check ==="

$cargoExe = Resolve-Cargo
$rustcExe = Get-Command rustc -ErrorAction SilentlyContinue
if (-not $rustcExe) {
    $rustcCandidate = Join-Path $env:USERPROFILE ".cargo\bin\rustc.exe"
    if (Test-Path $rustcCandidate) {
        $rustcExe = [pscustomobject]@{ Source = $rustcCandidate }
    }
}
$cmakeExe = Resolve-CMake
$msysBash = "C:\msys64\usr\bin\bash.exe"
$mingwGcc = "C:\msys64\mingw64\bin\gcc.exe"
$mingwGxx = "C:\msys64\mingw64\bin\g++.exe"

Write-Host ""
Write-Host "[Toolchain]"
& $cargoExe --version
if ($LASTEXITCODE -ne 0) {
    throw "cargo is not usable yet."
}

if (-not $rustcExe) {
    throw "rustc was not found on PATH. Finish Rust installation first."
}
& $rustcExe.Source --version
if ($LASTEXITCODE -ne 0) {
    throw "rustc is not usable yet."
}
& $cmakeExe --version
if ($LASTEXITCODE -ne 0) {
    throw "cmake is not usable yet."
}

Write-Host ""
Write-Host "[Recommended toolchain for nbis-rs on Windows]"
if ((Test-Path $msysBash) -and (Test-Path $mingwGcc) -and (Test-Path $mingwGxx)) {
    Write-Host "MSYS2 + MinGW detected:"
    Write-Host "  $mingwGcc"
}
else {
    Write-Warning "MSYS2 MinGW toolchain was not detected. nbis-rs and Go cgo flows are usually easier with MSYS2."
    Write-Host "Install suggestion:"
    Write-Host "  winget install -e --id MSYS2.MSYS2"
    Write-Host "  C:\msys64\usr\bin\bash -lc \"pacman -Syu --noconfirm\""
    Write-Host "  C:\msys64\usr\bin\bash -lc \"pacman -S --noconfirm --needed mingw-w64-x86_64-gcc mingw-w64-x86_64-cmake make\""
}

Write-Host ""
Write-Host "[MSVC compatibility note]"
Write-Host "This repo now patches nbis-rs automatically from build.rs for Windows MSVC builds."
Write-Host "You can still build with Visual Studio C++ + CMake even without MSYS2."

Write-Host ""
Write-Host "[USB driver prerequisite]"
Write-Host "1. Plug in the U.are.U 4500 scanner."
Write-Host "2. Install WinUSB for VID 05BA / PID 000A (for example via Zadig)."
Write-Host "3. Replug the scanner after driver replacement."

Write-Host ""
Write-Host "[Next command after toolchain install completes]"
Write-Host ".\scripts\ci_check.ps1"
