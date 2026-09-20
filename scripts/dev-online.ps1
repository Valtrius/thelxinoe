# Reuses the online development profile and its saved provider connections.
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$dockerCommand = (Get-Command docker.exe -ErrorAction Stop).Source
$stateRoot = Join-Path $repoRoot '.local/online/server'

foreach ($relativePath in @('thelxinoe.sqlite3', 'secrets/master.key')) {
    if (-not (Test-Path -LiteralPath (Join-Path $stateRoot $relativePath) -PathType Leaf)) {
        throw "The saved online profile is incomplete: $stateRoot/$relativePath is missing. Restore the profile with its database and master key before using dev:online."
    }
}

$version = (Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json).version
$runEnvironment = @{
    THELXINOE_TEST_HTTP_PORT = '18888'
    THELXINOE_TEST_HTTPS_PORT = '22443'
    THELXINOE_TEST_SUBNET = '172.31.254.0/24'
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
    Write-Host "Saved accounts and credentials: $stateRoot"
    Write-Host 'Sign in with your existing Thelxinoe account. Containers continue running after this command exits.'
}
finally {
    foreach ($name in $runEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    Pop-Location
}
