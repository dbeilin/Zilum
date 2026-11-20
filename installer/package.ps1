# Makes a release's two files, target\release\Zilum-Setup.exe and target\release\Zilum-Portable.zip,
# from the release exe. From the repo's root, after `cargo build --release`:
#
#   pwsh installer\package.ps1
#
# With -InstallInno it first installs the pinned Inno Setup, checked against its hash, as CI does;
# without, it uses the Inno Setup 6 already installed for this person.
param([switch]$InstallInno)

$ErrorActionPreference = 'Stop'
$release = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\target\release'))
$exe = Join-Path $release 'Zilum.exe'
if (-not (Test-Path $exe)) { throw 'Build the release exe first: cargo build --release' }

if ($InstallInno) {
    # The version the installer script is written for, and its installer's SHA-256 as GitHub lists it.
    $version = '6.7.3'
    $sha256 = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'
    $file = "innosetup-$version.exe"
    $url = "https://github.com/jrsoftware/issrc/releases/download/is-$($version -replace '\.', '_')/$file"
    $temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
    $download = Join-Path $temp $file
    $inno = Join-Path $temp "inno-setup-$version"
    Invoke-WebRequest -Uri $url -OutFile $download
    $actual = (Get-FileHash $download -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { throw "$file has SHA-256 $actual, not $sha256" }
    $setup = Start-Process $download -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER', "/DIR=`"$inno`"")
    if ($setup.ExitCode -ne 0) { throw "The Inno Setup installer exited with $($setup.ExitCode)" }
    $iscc = Join-Path $inno 'ISCC.exe'
} else {
    $iscc = Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'
}

& $iscc (Join-Path $PSScriptRoot 'Zilum.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC exited with $LASTEXITCODE" }

# The exe and an empty settings.ini at the zip's top level and nothing else, the file beside the
# exe being what makes it a Portable Zilum.
$portable = Join-Path $release 'portable'
$zip = Join-Path $release 'Zilum-Portable.zip'
Remove-Item $portable, $zip -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory $portable | Out-Null
Copy-Item $exe $portable
New-Item -ItemType File (Join-Path $portable 'settings.ini') | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($portable, $zip)
Remove-Item $portable -Recurse -Force
