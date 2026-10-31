# Builds the release executable and the Windows installer: dist\AudioPaint-<version>-setup.exe
# Requires Inno Setup 6 (winget install JRSoftware.InnoSetup).
#
# The build uses its own target folder (target-dist) so it works while a copy of
# Audio Paint from target\release is open.

$root = Split-Path $PSScriptRoot -Parent
$env:Path += ";$env:USERPROFILE\.cargo\bin"

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
Write-Host "Building Audio Paint $version"

$env:CARGO_TARGET_DIR = "$root\target-dist"
cargo build --release --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { Write-Error "cargo build failed"; exit 1 }

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
