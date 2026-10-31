# Builds the release executable and the Windows installer:
# dist\AudioPaint-<version>.<build>-setup.exe
# Requires Inno Setup 6 (winget install JRSoftware.InnoSetup).
#
# The build uses its own target folder (target-dist) so it works while a copy of
# Audio Paint from target\release is open. A release build with changed sources
# increments build_number.txt (see build.rs).

$root = Split-Path $PSScriptRoot -Parent
$env:Path += ";$env:USERPROFILE\.cargo\bin"

# Cargo reads .cargo\config.toml (static C runtime) only from the current directory
# upwards, so the build must run inside the project folder.
Set-Location $root
$env:CARGO_TARGET_DIR = "$root\target-dist"
cargo build --release
if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed"; exit 1 }

$exe = "$root\target-dist\release\audio_paint.exe"

# Refuse to package an executable that needs the Visual C++ Redistributable.
$dumpbin = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue |
    Select-Object -First 1 -ExpandProperty FullName
if ($dumpbin) {
    $runtimeDlls = & $dumpbin /dependents $exe | Select-String "VCRUNTIME|MSVCP|api-ms-win-crt"
    if ($runtimeDlls) {
        Write-Error "audio_paint.exe depends on the Visual C++ runtime ($($runtimeDlls[0].Line.Trim())); static linking failed"
        exit 1
    }
    Write-Host "Checked: no Visual C++ runtime DLLs needed"
} else {
    Write-Warning "dumpbin not found; skipped the Visual C++ runtime check"
}

# Take the version from the built executable, so the installer always matches it.
$version = (Get-Item $exe).VersionInfo.FileVersion
Write-Host "Packaging Audio Paint $version"

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) {
    Write-Error "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup"
    exit 1
}

& $iscc /Q "/DAppVersion=$version" "$PSScriptRoot\audio_paint.iss"
if ($LASTEXITCODE -ne 0) { Write-Error "Inno Setup failed"; exit 1 }

$setup = Join-Path $root "dist\AudioPaint-$version-setup.exe"
Write-Host "Installer: $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
