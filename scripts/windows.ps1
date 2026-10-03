param([switch]$SkipTests, [string]$IsccPath)
$ErrorActionPreference = 'Stop'
$staging = $null
Push-Location (Split-Path $PSScriptRoot)
try {
    if (-not $IsccPath) {
        $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($command) { $IsccPath = $command.Source }
        foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
            if (-not $base) { continue }
            foreach ($version in @(7, 6)) {
                $candidate = Join-Path $base "Inno Setup $version/ISCC.exe"
                if (-not $IsccPath -and (Test-Path $candidate)) { $IsccPath = $candidate }
            }
        }
    }
    if (-not $IsccPath) { throw 'Install Inno Setup 6.3+ or pass -IsccPath to ISCC.exe' }
    $target = 'x86_64-pc-windows-msvc'
    if (-not $SkipTests) {
        cargo test --locked --target $target
        if ($LASTEXITCODE -ne 0) { throw 'Windows tests failed' }
    }
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $package = $metadata.packages | Where-Object name -eq 'carlitos'
    $messages = @(cargo build --locked --release --target $target --message-format=json-render-diagnostics |
        ForEach-Object { $_ | ConvertFrom-Json })
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
    $executable = ($messages | Where-Object {
        $_.reason -eq 'compiler-artifact' -and $_.package_id -eq $package.id -and
        $_.target.name -eq 'carlitos' -and $_.executable
    } | Select-Object -Last 1).executable
    $assets = ($messages | Where-Object {
        $_.reason -eq 'build-script-executed' -and $_.package_id -eq $package.id
    } | Select-Object -Last 1).out_dir
    if (-not $executable -or -not $assets) { throw 'Cargo did not report the installer inputs' }
    $releases = Join-Path (Get-Location) 'target/releases'
    $staging = Join-Path (Get-Location) "target/windows-tools/installer.$([guid]::NewGuid())"
    New-Item -ItemType Directory -Force $staging | Out-Null
    & $IsccPath "/DAppVersion=$($package.version)" "/DSourceExe=$executable" "/DAssetsDir=$assets" "/O$staging" data/windows/installer.iss
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed' }
    $hash = (Get-FileHash "$staging/Carlitos.exe" -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  Carlitos.exe" | Set-Content "$staging/Carlitos.exe.sha256" -Encoding ascii
    New-Item -ItemType Directory -Force $releases | Out-Null
    Move-Item "$staging/Carlitos.exe", "$staging/Carlitos.exe.sha256" $releases -Force
    foreach ($file in @('setup.exe', 'setup.exe.sha256')) {
        $path = Join-Path $releases $file
        if (Test-Path $path) { Remove-Item $path -Force }
    }
    # Remove only the old build products, never the portable library beside them.
    $portable = Join-Path $releases 'windows-x64'
    foreach ($file in @('Carlitos.exe', 'Carlitos.exe.sha256', 'Carlitos.exe.ppdb')) {
        $path = Join-Path $portable $file
        if (Test-Path $path) { Remove-Item $path -Force }
    }
    if ((Test-Path $portable) -and -not (Get-ChildItem $portable -Force)) {
        Remove-Item $portable
    }
    Write-Host "Installer: $releases/Carlitos.exe ($hash)"
} finally {
    if ($staging -and (Test-Path $staging)) { Remove-Item $staging -Recurse -Force }
    Pop-Location
}
