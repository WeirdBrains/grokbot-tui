# grokbot-tui @VERSION@ setup for Windows: pick a build, then install it.
#   standard - pure-Rust media (GIF, H.264 MP4 via the bundled OpenH264, common audio)
#   ffmpeg   - the same plus FFmpeg linked in (plays nearly any video), larger
# Both are GPL-3.0-or-later; see THIRD_PARTY.md (incl. the H.264 patent note).
# Usage: irm <release>/grokbot-tui-setup.ps1 | iex            (asks)
#        & ([scriptblock]::Create((irm <url>))) -Variant ffmpeg (no prompt)
param([ValidateSet('', 'standard', 'ffmpeg')][string]$Variant = '')
$ErrorActionPreference = 'Stop'
$Version = '@VERSION@'
$Base = "https://github.com/WeirdBrains/grokbot-tui/releases/download/v$Version"
if (-not $Variant) {
    Write-Host "grokbot-tui $Version - which build?"
    Write-Host '  1) standard (smaller; common video via the built-in decoder)'
    Write-Host '  2) ffmpeg   (FFmpeg compiled in: plays nearly any video)'
    $ans = Read-Host 'Choice [1]'
    $Variant = if ($ans -in @('2', 'f', 'ffmpeg')) { 'ffmpeg' } else { 'standard' }
}
function Install-Standard { Invoke-RestMethod "$Base/grokbot-tui-installer.ps1" | Invoke-Expression }
if ($Variant -eq 'standard') { Install-Standard; return }
$name = 'grokbot-tui-ffmpeg-x86_64-pc-windows-msvc'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    try { Invoke-WebRequest "$Base/$name.zip" -OutFile "$tmp\$name.zip" -UseBasicParsing }
    catch { Write-Warning "This release has no -ffmpeg build for Windows; installing standard."; Install-Standard; return }
    Invoke-WebRequest "$Base/$name.zip.sha256" -OutFile "$tmp\sum" -UseBasicParsing
    $want = ((Get-Content "$tmp\sum" -Raw).Split(' ')[0]).Trim().ToLower()
    $got = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if ($want -ne $got) { throw "checksum mismatch for $name.zip" }
    Expand-Archive "$tmp\$name.zip" -DestinationPath $tmp
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
    $bin = Join-Path $cargoHome 'bin'
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item "$tmp\$name\grokbot-tui.exe" $bin -Force
    $doc = Join-Path $env:LOCALAPPDATA 'grokbot-tui'
    New-Item -ItemType Directory -Force $doc | Out-Null
    Copy-Item "$tmp\$name\LICENSE", "$tmp\$name\THIRD_PARTY.md" $doc -Force
    Write-Host "Installed grokbot-tui $Version (ffmpeg build) to $bin\grokbot-tui.exe"
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not ($userPath -split ';' | Where-Object { $_ -eq $bin })) {
        [Environment]::SetEnvironmentVariable('Path', "$bin;$userPath", 'User')
        Write-Host "Added $bin to your user PATH (open a new terminal)."
    }
} finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
