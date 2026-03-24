Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-RepoRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ScriptPath
    )

    return (Resolve-Path (Join-Path (Split-Path -Parent $ScriptPath) "..")).Path
}

function Resolve-Cargo {
    $cargoCmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargoCmd) {
        return $cargoCmd.Source
    }

    $candidate = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $candidate) {
        return $candidate
    }

    throw "cargo was not found on PATH. Finish Rust installation or add `~\.cargo\bin` to PATH."
}

function Resolve-CMake {
    $cmakeCmd = Get-Command cmake -ErrorAction SilentlyContinue
    if ($cmakeCmd) {
        return $cmakeCmd.Source
    }

    $candidates = @(
        "C:\Program Files\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Professional\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            if (($env:Path -split ';') -notcontains (Split-Path $candidate -Parent)) {
                $env:Path = "$(Split-Path $candidate -Parent);$env:Path"
            }
            return $candidate
        }
    }

    throw "cmake was not found. Install CMake and ensure `cmake.exe` is on PATH."
}

function Initialize-CargoTargetDir {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    if ($env:CARGO_TARGET_DIR) {
        return $env:CARGO_TARGET_DIR
    }

    $preferred = Join-Path $env:SystemDrive "mktdp-target"
    try {
        New-Item -Path $preferred -ItemType Directory -Force | Out-Null
        $env:CARGO_TARGET_DIR = $preferred
        return $env:CARGO_TARGET_DIR
    }
    catch {
        $fallback = Join-Path $env:TEMP "mktdp-target"
        New-Item -Path $fallback -ItemType Directory -Force | Out-Null
        $env:CARGO_TARGET_DIR = $fallback
        return $env:CARGO_TARGET_DIR
    }
}

function Initialize-WindowsCompatHeaders {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $compatInclude = Join-Path $RepoRoot "compat\windows"
    $compatHeader = Join-Path $compatInclude "unistd.h"
    if (-not (Test-Path $compatHeader)) {
        return
    }

    $flag = "-I$compatInclude"
    if ([string]::IsNullOrWhiteSpace($env:CFLAGS)) {
        $env:CFLAGS = $flag
    }
    elseif ($env:CFLAGS -notlike "*$compatInclude*") {
        $env:CFLAGS = "$flag $env:CFLAGS"
    }

    if ([string]::IsNullOrWhiteSpace($env:CXXFLAGS)) {
        $env:CXXFLAGS = $flag
    }
    elseif ($env:CXXFLAGS -notlike "*$compatInclude*") {
        $env:CXXFLAGS = "$flag $env:CXXFLAGS"
    }
}

function Patch-NbisRsWindowsMsvc {
    Write-Host "nbis-rs patching is handled by build.rs during cargo builds."
}

function Get-CargoTargetRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    if ($env:CARGO_TARGET_DIR) {
        return $env:CARGO_TARGET_DIR
    }
    return (Join-Path $RepoRoot "target")
}

function Get-CargoProfileDir {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [string]$Profile = "debug"
    )

    return (Join-Path (Get-CargoTargetRoot -RepoRoot $RepoRoot) $Profile)
}

function Invoke-Cargo {
    param(
        [Parameter(Mandatory = $true)]
        [string]$CargoPath,
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Args
    )

    & $CargoPath @Args
    if ($LASTEXITCODE -ne 0) {
        throw "cargo failed with exit code ${LASTEXITCODE}: cargo $($Args -join ' ')"
    }
}

function Resolve-RustImportLibrary {
    param(
        [Parameter(Mandatory = $true)]
        [string]$TargetDir
    )

    $candidates = @(
        (Join-Path $TargetDir "mktdp_driver.lib"),
        (Join-Path $TargetDir "mktdp_driver.dll.lib"),
        (Join-Path $TargetDir "libmktdp_driver.dll.a")
    )

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    throw "Rust import library not found in $TargetDir. Expected one of: $($candidates -join ', ')"
}

function Resolve-VsDevCmd {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $installPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installPath)) {
            $candidate = Join-Path $installPath.Trim() "Common7\Tools\VsDevCmd.bat"
            if (Test-Path $candidate) {
                return $candidate
            }
        }
    }

    $fallbacks = @(
        "C:\Program Files\Microsoft Visual Studio\18\Community\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\18\Professional\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\18\Enterprise\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\18\BuildTools\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\2022\Professional\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\Common7\Tools\VsDevCmd.bat",
        "C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
    )

    foreach ($candidate in $fallbacks) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    return $null
}

function Build-CSmokeExecutable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [string]$Profile = "debug",
        [string]$OutputName = "c_smoke_test.exe"
    )

    $targetDir = Get-CargoProfileDir -RepoRoot $RepoRoot -Profile $Profile
    $source = Join-Path $RepoRoot "tests\test.c"
    $includeDir = Join-Path $RepoRoot "include"
    $outputPath = Join-Path $targetDir $OutputName

    New-Item -Path $targetDir -ItemType Directory -Force | Out-Null

    $cl = Get-Command cl -ErrorAction SilentlyContinue
    if ($cl) {
        $importLib = Resolve-RustImportLibrary -TargetDir $targetDir
        $compileCmd = ('cl /nologo /std:c11 /W4 /O2 /I"{0}" "{1}" "{2}" /Fe:"{3}"' -f $includeDir, $source, $importLib, $outputPath)
        cmd.exe /d /c $compileCmd | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "cl failed with exit code $LASTEXITCODE."
        }
        return $outputPath
    }

    $vsDevCmd = Resolve-VsDevCmd
    if ($vsDevCmd) {
        $importLib = Resolve-RustImportLibrary -TargetDir $targetDir
        $compileCmd = ('cl /nologo /std:c11 /W4 /O2 /I"{0}" "{1}" "{2}" /Fe:"{3}"' -f $includeDir, $source, $importLib, $outputPath)
        $cmdLine = ('call "{0}" -arch=x64 -host_arch=x64 >nul && {1}' -f $vsDevCmd, $compileCmd)
        cmd.exe /d /c $cmdLine | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "cl (via VsDevCmd) failed with exit code $LASTEXITCODE."
        }
        return $outputPath
    }

    $gcc = Get-Command gcc -ErrorAction SilentlyContinue
    if ($gcc) {
        & $gcc.Source -std=c11 -Wall -Wextra -O2 "-I$includeDir" $source "-L$targetDir" -lmktdp_driver "-o$outputPath" | Out-Host
        if ($LASTEXITCODE -ne 0) {
            throw "gcc failed with exit code $LASTEXITCODE."
        }
        return $outputPath
    }

    throw "No supported C compiler found. Install Visual Studio C++ Build Tools (cl) or MinGW GCC."
}

function Add-DllSearchPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [string]$Profile = "debug"
    )

    $targetDir = Get-CargoProfileDir -RepoRoot $RepoRoot -Profile $Profile
    if (Test-Path $targetDir) {
        $env:Path = "$targetDir;$env:Path"
    }
}
