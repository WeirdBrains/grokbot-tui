# Build grokbot-tui from this checkout and install it (Windows). No `cargo install`,
# no downloaded prebuilt binaries. Building needs a Rust toolchain (MSVC).
#
#   .\install-local.ps1 [-Prefix DIR] [-Ffmpeg] [-Vendor] [-Offline] [-NoBuild] [-Uninstall]
#
# Default prefix: %LOCALAPPDATA%\Programs\grokbot-tui (binary in <prefix>\bin).
# -Ffmpeg needs FFmpeg static libs via vcpkg (x64-windows-static-md) and VCPKG_ROOT.
param(
  [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\grokbot-tui'),
  [switch]$Ffmpeg, [switch]$Vendor, [switch]$Offline, [switch]$NoBuild, [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'
$Root = $PSScriptRoot
$Name = 'grokbot-tui'
$BinDir = Join-Path $Prefix 'bin'
$DocDir = Join-Path $Prefix 'share\doc'
$DataDir = Join-Path $Prefix 'share'

function Update-UserPath([switch]$Remove) {
  $p = [Environment]::GetEnvironmentVariable('Path', 'User')
  $parts = @($p -split ';' | Where-Object { $_ -and $_ -ne $BinDir })
  if (-not $Remove) { $parts += $BinDir }
  [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
}

if ($Uninstall) {
  if (Test-Path $Prefix) { Remove-Item -Recurse -Force $Prefix }
  Update-UserPath -Remove
  Write-Host "Removed $Prefix (settings and history are kept)."
  exit 0
}

$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Bin = Join-Path $TargetDir "release\$Name.exe"

if (-not $NoBuild) {
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found: building from source needs a Rust toolchain (https://rustup.rs).'
  }
  $a = @('build', '--release', '--locked', '-p', $Name)
  if ($Ffmpeg) {
    $a += @('--features', 'ffmpeg')
    $env:VCPKGRS_TRIPLET = 'x64-windows-static-md'
    $env:RUSTFLAGS = "$env:RUSTFLAGS -C link-arg=mfuuid.lib -C link-arg=strmiids.lib -C link-arg=mfplat.lib -C link-arg=ole32.lib".Trim()
  }
  $vendorDir = Join-Path $Root 'vendor'
  if ($Vendor -and -not (Test-Path $vendorDir)) {
    Push-Location $Root; try { cargo vendor --locked vendor | Out-Null } finally { Pop-Location }
  }
  if ((Test-Path $vendorDir) -and ($Vendor -or $Offline)) {
    $vd = $vendorDir -replace '\\', '/'
    $a += @('--offline', '--config', 'source.crates-io.replace-with="vendored-sources"',
            '--config', "source.vendored-sources.directory=`"$vd`"")
  } elseif ($Offline) { $a += '--offline' }
  Push-Location $Root
  try { & cargo @a; if ($LASTEXITCODE) { throw "cargo build failed ($LASTEXITCODE)" } } finally { Pop-Location }
}

if (-not (Test-Path $Bin)) { throw "no binary at $Bin; build first (drop -NoBuild)" }
New-Item -ItemType Directory -Force $BinDir, $DocDir | Out-Null
Copy-Item -Force $Bin $BinDir
foreach ($f in 'README.md', 'CHANGELOG.md', 'THIRD_PARTY.md', 'LICENSE') {
  $s = Join-Path $Root $f; if (Test-Path $s) { Copy-Item -Force $s $DocDir }
}
foreach ($d in 'themes', 'keymap', 'notify-rules') {
  $s = Join-Path $Root $d
  if (Test-Path $s) { Copy-Item -Recurse -Force $s $DataDir }
}
Update-UserPath
Write-Host "Installed $(Join-Path $BinDir "$Name.exe"). Open a new terminal so PATH picks it up."
