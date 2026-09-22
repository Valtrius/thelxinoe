# Reuses the online development profile and its saved provider connections.
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$dockerCommand = (Get-Command docker.exe -ErrorAction Stop).Source
$onlineRootValue = [Environment]::GetEnvironmentVariable('THELXINOE_ONLINE_ROOT', 'Process')
if ([string]::IsNullOrWhiteSpace($onlineRootValue)) {
    $onlineRootValue = './.local/online'
}
$onlineRoot = if ([IO.Path]::IsPathRooted($onlineRootValue)) {
    [IO.Path]::GetFullPath($onlineRootValue)
}
else {
    [IO.Path]::GetFullPath((Join-Path $repoRoot $onlineRootValue))
}
$stateRoot = Join-Path $onlineRoot 'server'
$cacheRoot = Join-Path $onlineRoot 'cache'
$databasePath = Join-Path $stateRoot 'thelxinoe.sqlite3'
$masterKeyPath = Join-Path $stateRoot 'secrets/master.key'

New-Item -ItemType Directory -Path $stateRoot -Force | Out-Null
New-Item -ItemType Directory -Path $cacheRoot -Force | Out-Null

$databaseExists = Test-Path -LiteralPath $databasePath -PathType Leaf
$masterKeyExists = Test-Path -LiteralPath $masterKeyPath -PathType Leaf
if ($databaseExists -ne $masterKeyExists) {
    $missing = if ($databaseExists) { 'secrets/master.key' } else { 'thelxinoe.sqlite3' }
    throw "The saved online profile is incomplete: $stateRoot/$missing is missing. Restore the matching database and master key before using dev:online."
}
$firstLaunch = -not $databaseExists

$version = (Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json).version
$runEnvironment = @{
    THELXINOE_TEST_HTTP_PORT = '18888'
    THELXINOE_TEST_HTTPS_PORT = '22443'
    THELXINOE_TEST_SUBNET = '172.31.254.0/24'
    THELXINOE_ONLINE_ROOT = $onlineRootValue
}
$previousEnvironment = @{}
foreach ($name in $runEnvironment.Keys) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

Push-Location -LiteralPath $repoRoot
try {
    foreach ($name in $runEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $runEnvironment[$name], 'Process')
    }

    if (-not $SkipBuild) {
        & $dockerCommand build --target server -t "thelxinoe-server:$version" .
        if ($LASTEXITCODE -ne 0) {
            throw "Server image build failed (exit $LASTEXITCODE)."
        }
    }

    & $dockerCommand compose -p thelxinoe-online -f compose.test.yaml -f compose.online.yaml up -d --wait
    if ($LASTEXITCODE -ne 0) {
        throw "Online development startup failed (exit $LASTEXITCODE)."
    }

    Write-Host 'Online development instance: https://localhost:22443'
    Write-Host "Saved profile data: $stateRoot"
    if ($firstLaunch) {
        foreach ($path in @($databasePath, $masterKeyPath)) {
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
                throw "Online development startup did not initialize $path."
            }
        }
        Write-Host 'This is a new online profile. Open the address above and create the first administrator.'
    }
    else {
        Write-Host 'Sign in with your existing Thelxinoe account.'
    }
    Write-Host 'Containers continue running after this command exits.'
}
finally {
    foreach ($name in $runEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    Pop-Location
}
