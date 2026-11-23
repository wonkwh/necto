# Copyright (c) 2026 Viva Republica, Inc.
#
# Installs the NSIS build silently and checks what a user gets: necto-cli.exe on PATH,
# the app answering it over the control pipe, and PATH restored on uninstall.
# Run from NectoWindows/src-tauri after `tauri build --config tauri.bundle.conf.json`.

$ErrorActionPreference = 'Stop'

function Get-UserPath {
    (Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames') -split ';'
}

function Wait-Until([scriptblock] $condition, [string] $what) {
    foreach ($attempt in 1..60) {
        if (& $condition) { return }
        Start-Sleep -Milliseconds 500
    }
    throw "Timed out waiting for $what"
}

$installer = Get-ChildItem 'target/release/bundle/nsis/*-setup.exe' | Select-Object -First 1
if (-not $installer) { throw 'No NSIS installer was built' }
Start-Process -Wait -FilePath $installer.FullName -ArgumentList '/S'

$dir = Join-Path $env:LOCALAPPDATA 'Necto'
$cli = Join-Path $dir 'necto-cli.exe'
if (-not (Test-Path $cli)) { throw "necto-cli.exe is not in $dir" }
if ((Get-UserPath) -notcontains $dir) { throw "The user PATH does not hold $dir" }
Write-Host "Installed with necto-cli.exe on PATH: $dir"

# Without the app, the CLI says so and fails.
& $cli device list 2>&1 | Out-Host
if ($LASTEXITCODE -ne 1) { throw "device list without Necto exited $LASTEXITCODE" }

# With the app, the CLI reaches it through the owner-only pipe.
$app = Get-ChildItem $dir -Filter '*.exe' | Where-Object { $_.Name -notin 'necto-cli.exe', 'uninstall.exe' } | Select-Object -First 1
$process = Start-Process -FilePath $app.FullName -PassThru
try {
    Wait-Until { & $cli device list *> $null; $LASTEXITCODE -eq 0 } 'the app to answer necto-cli'
    & $cli device list --json | Out-Host
    Write-Host "$($app.Name) answered necto-cli over the control pipe"
} finally {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $process.Id -Timeout 10 -ErrorAction SilentlyContinue
}

# The skill installs into a scratch profile, not the runner's.
$profileDir = Join-Path $env:RUNNER_TEMP 'necto-profile'
$saved = $env:USERPROFILE
$env:USERPROFILE = $profileDir
try {
    & $cli skills install --codex --claude | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "skills install exited $LASTEXITCODE" }
} finally {
    $env:USERPROFILE = $saved
}
foreach ($agent in '.agents', '.claude') {
    if (-not (Test-Path (Join-Path $profileDir "$agent\skills\necto\SKILL.md"))) { throw "No skill for $agent" }
}

# The uninstaller relaunches itself from a temporary copy, so -Wait returns early.
Start-Process -Wait -FilePath (Join-Path $dir 'uninstall.exe') -ArgumentList '/S'
Wait-Until { (Get-UserPath) -notcontains $dir } 'the uninstaller to take the folder off PATH'
Wait-Until { -not (Test-Path $cli) } 'the uninstaller to remove necto-cli.exe'
Write-Host 'Uninstalled and PATH restored'
