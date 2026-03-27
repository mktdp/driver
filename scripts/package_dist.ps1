[CmdletBinding()]
param(
    [string]$CargoExe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $PSScriptRoot "windows_common.ps1")
Set-Location $repoRoot

function Get-OsName {
    if ($env:OS -eq "Windows_NT") {
        return "Windows"
    }

    $uname = Get-Command uname -ErrorAction SilentlyContinue
    if ($uname) {
        return (& $uname.Source -s).Trim()
    }

    return ""
}

function Get-SharedLibraryName {
    $osName = Get-OsName
    switch -Wildcard ($osName) {
        "Linux*" { return "libmktdp_driver.so" }
        "Darwin*" { return "libmktdp_driver.dylib" }
        "MINGW*" { return "mktdp_driver.dll" }
        "MSYS*" { return "mktdp_driver.dll" }
        "CYGWIN*" { return "mktdp_driver.dll" }
        "Windows*" { return "mktdp_driver.dll" }
        default { throw "unsupported OS: $osName" }
    }
}

function Fix-NbisLib64Symlinks {
    param(
        [string]$Profile,
        [string]$TargetRoot
    )

    if ((Get-OsName) -like "Windows*") {
        return $false
    }

    $pattern = Join-Path $TargetRoot "$Profile\build\nbis-rs-*"
    $fixed = $false

    foreach ($nbisBuild in Get-ChildItem -Path $pattern -Directory -ErrorAction SilentlyContinue) {
        $staging = Join-Path $nbisBuild.FullName "out\build\install_staging\nfiq2"
        if (-not (Test-Path $staging -PathType Container)) {
            continue
        }

        $lib64Dir = Join-Path $staging "lib64"
        $libDir = Join-Path $staging "lib"
        if ((Test-Path $lib64Dir -PathType Container) -and -not (Test-Path $libDir)) {
            try {
                New-Item -Path $libDir -ItemType SymbolicLink -Target "lib64" -ErrorAction Stop | Out-Null
                Write-Host "  linked $libDir -> lib64"
                $fixed = $true
            }
            catch {
                Write-Warning "failed to link $libDir -> lib64: $($_.Exception.Message)"
            }
        }
    }

    return $fixed
}

if (-not $CargoExe) {
    $CargoExe = Resolve-Cargo
}
$cmakeExe = Resolve-CMake
$targetRoot = Initialize-CargoTargetDir -RepoRoot $repoRoot
Initialize-WindowsCompatHeaders -RepoRoot $repoRoot
Patch-NbisRsWindowsMsvc -CargoPath $CargoExe
Write-Host "Using CARGO_TARGET_DIR: $targetRoot"
Write-Host "Using CMake: $cmakeExe"

Write-Host "[1/4] Building release library..."
try {
    Invoke-Cargo -CargoPath $CargoExe build --release
}
catch {
    Write-Host "release build failed, attempting nbis lib64 -> lib symlink fix..."
    [void](Fix-NbisLib64Symlinks -Profile "release" -TargetRoot $targetRoot)
    Invoke-Cargo -CargoPath $CargoExe build --release
}

[void](Fix-NbisLib64Symlinks -Profile "release" -TargetRoot $targetRoot)

$libName = Get-SharedLibraryName
$releaseDir = Join-Path $targetRoot "release"
$releaseLib = Join-Path $releaseDir $libName
if (-not (Test-Path $releaseLib -PathType Leaf)) {
    throw "expected library not found: $releaseLib"
}

Write-Host "[2/4] Assembling dist/..."
$distDir = Join-Path $repoRoot "dist"
if (Test-Path $distDir) {
    Remove-Item -Path $distDir -Recurse -Force
}
New-Item -Path (Join-Path $distDir "include") -ItemType Directory -Force | Out-Null

Copy-Item -Path $releaseLib -Destination (Join-Path $distDir $libName) -Force
Copy-Item -Path (Join-Path $repoRoot "include\fingerprint.h") -Destination (Join-Path $distDir "include\fingerprint.h") -Force
Copy-Item -Path (Join-Path $repoRoot "README.md") -Destination (Join-Path $distDir "README.md") -Force
Copy-Item -Path (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $distDir "LICENSE") -Force

Write-Host "[3/4] Release size audit..."
Get-Item -Path $releaseLib | Select-Object Name, Length | Format-Table -AutoSize
Get-ChildItem -Path $distDir -Recurse -File | Select-Object FullName, Length | Format-Table -AutoSize

Write-Host "[4/4] Done."
