#Requires -Version 7.0
param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference = 'Stop'
# Run in a disposable Windows account/CI runner: this exercises real installation.
$Installer = (Resolve-Path $Installer).Path
$local = [Environment]::GetFolderPath('LocalApplicationData')
$data = Join-Path $local 'Carlitos'
$install = Join-Path $local 'Programs/Carlitos'
$startLink = Join-Path ([Environment]::GetFolderPath('Programs')) 'Carlitos.lnk'
$desktopLink = Join-Path ([Environment]::GetFolderPath('DesktopDirectory')) 'Carlitos.lnk'
$registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.mny315.Carlitos_is1'
foreach ($existing in @($data, $install, $startLink, $desktopLink, $registry)) {
    if (Test-Path $existing) { throw "Use a clean Windows account; already exists: $existing" }
}
$logs = Join-Path $env:TEMP 'Carlitos installer test'
New-Item -ItemType Directory -Force $logs | Out-Null
function Run-Checked([string]$File, [string]$Arguments) {
    # Exercise the application's startup defaults, not the test host's renderer
    # or font overrides (which can hide missing drivers/fonts in a fresh install).
    $start = [System.Diagnostics.ProcessStartInfo]::new($File, $Arguments)
    $start.UseShellExecute = $false
    foreach ($name in @('SLINT_BACKEND', 'SLINT_DEFAULT_FONT', 'SLINT_FONT_PATH')) {
        $start.Environment.Remove($name) | Out-Null
    }
    $process = [System.Diagnostics.Process]::Start($start)
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw "Timed out: $File" }
    if ($process.ExitCode -ne 0) { throw "$File exited with $($process.ExitCode)" }
}
function Require-File([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Missing file: $Path" }
}
function Install-App([string]$Log) {
    Run-Checked $Installer "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TASKS=desktopicon /LOG=`"$logs/$Log.log`""
}
Install-App 'install'
$exe = Join-Path $install 'Carlitos.exe'
foreach ($path in @($exe, "$install/carlitos-installed", "$install/licenses.txt", $startLink, $desktopLink)) {
    Require-File $path
}
$entry = Get-ItemProperty $registry
if ($entry.DisplayName -ne 'Carlitos') { throw "Unexpected installed application name: $($entry.DisplayName)" }
if (-not $entry.UninstallString) { throw 'Missing uninstall command' }
$shell = New-Object -ComObject WScript.Shell
foreach ($link in @($startLink, $desktopLink)) {
    if ($shell.CreateShortcut($link).TargetPath -ne $exe) { throw "Wrong shortcut target: $link" }
}

# Launch without --data-dir or shortcut parameters, so installed-mode selection is exercised.
Run-Checked $exe '--no-desktop --language ru --quit-after 3'
Require-File "$data/library.sqlite3"
Require-File "$data/settings.json"
if (Test-Path "$install/Carlitos-data") { throw 'Installed app used portable data' }
$settings = Get-Content "$data/settings.json" -Raw | ConvertFrom-Json
$settings.theme = 'light'
$settings | ConvertTo-Json | Set-Content "$data/settings.json" -Encoding utf8
$before = (Get-FileHash "$data/settings.json").Hash
$libraryBefore = (Get-FileHash "$data/library.sqlite3").Hash

# Reinstall must reuse the installation and preserve both preferences and library.
Install-App 'upgrade'
if ((Get-FileHash "$data/settings.json").Hash -ne $before) { throw 'Upgrade changed settings' }
if ((Get-FileHash "$data/library.sqlite3").Hash -ne $libraryBefore) { throw 'Upgrade changed library' }
Run-Checked $exe '--no-desktop --quit-after 3'
if ((Get-Content "$data/settings.json" -Raw | ConvertFrom-Json).theme -ne 'light') {
    throw 'Upgraded app lost preferences'
}

# The same executable remains portable when copied without the install marker.
$portable = Join-Path $logs 'Portable copy'
New-Item -ItemType Directory -Force $portable | Out-Null
Copy-Item $exe "$portable/Carlitos.exe"
Run-Checked "$portable/Carlitos.exe" '--no-desktop --quit-after 3'
Require-File "$portable/Carlitos-data/library.sqlite3"
Require-File "$portable/Carlitos-data/settings.json"

$before = (Get-FileHash "$data/settings.json").Hash
$libraryBefore = (Get-FileHash "$data/library.sqlite3").Hash
Run-Checked "$install/unins000.exe" "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=`"$logs/uninstall.log`""
# Inno's uninstaller can finish cleaning up from a temporary child process.
$deadline = (Get-Date).AddSeconds(30)
while ((Test-Path $install) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
foreach ($removed in @($install, $startLink, $desktopLink, $registry)) {
    if (Test-Path $removed) { throw "Uninstall left behind: $removed" }
}
if ((Get-FileHash "$data/settings.json").Hash -ne $before) { throw 'Uninstall changed settings' }
if ((Get-FileHash "$data/library.sqlite3").Hash -ne $libraryBefore) { throw 'Uninstall changed library' }
Write-Host "Installer lifecycle passed; retained user data in $data; logs in $logs"
