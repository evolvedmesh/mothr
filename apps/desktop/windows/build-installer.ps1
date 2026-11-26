# Build Mothr's Windows installer from an unpacked release folder.
#
#   build-installer.ps1 -Version 0.2.0 -Source mothr_0.2.0_windows_x64 -OutDir .
#
# Writes <OutDir>\mothr_<Version>_windows_x64_setup.exe. Uses its own copy of Inno Setup (the
# official release from jrsoftware/issrc, pinned by version and checksum), installed into the
# temp folder on first use, never one that happens to be on PATH.
# Used by .github/workflows/release.yml (the real build) and ci.yml (a check of mothr.iss).
param(
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $Source,
    [string] $OutDir = "."
)
$ErrorActionPreference = "Stop"

$InnoVersion = "6.7.3"
$InnoSha256 = "9C73C3BAE7ED48D44112A0F48E66742C00090BDB5BEF71D9D3C056C66E97B732"

$tmp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$dir = Join-Path $tmp "inno-setup-$InnoVersion"
$iscc = Join-Path $dir "ISCC.exe"
if (-not (Test-Path $iscc)) {
    $setup = Join-Path $tmp "innosetup-$InnoVersion.exe"
    $tag = "is-" + $InnoVersion.Replace(".", "_")
    Invoke-WebRequest "https://github.com/jrsoftware/issrc/releases/download/$tag/innosetup-$InnoVersion.exe" -OutFile $setup
    $hash = (Get-FileHash $setup -Algorithm SHA256).Hash
    if ($hash -ne $InnoSha256) { throw "Inno Setup $InnoVersion doesn't match its checksum (got $hash)" }
    $p = Start-Process $setup -Wait -PassThru -ArgumentList @(
        "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-", "/CURRENTUSER", "/DIR=`"$dir`"")
    if ($p.ExitCode -ne 0) { throw "Installing Inno Setup failed ($($p.ExitCode))" }
    if (-not (Test-Path $iscc)) { throw "Inno Setup installed, but $iscc is missing" }
}

$script = Join-Path $PSScriptRoot "mothr.iss"
$src = (Resolve-Path $Source).Path
New-Item -ItemType Directory -Force $OutDir | Out-Null
$out = (Resolve-Path $OutDir).Path
& $iscc "/DAppVersion=$Version" "/DSourceDir=$src" "/O$out" $script
if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)" }
Get-Item (Join-Path $out "mothr_${Version}_windows_x64_setup.exe")
