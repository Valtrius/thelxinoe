# Starts a new development server without deleting or reusing previous data.
[CmdletBinding()]
param(
    [ValidateRange(1024, 65535)]
    [int]$Port = 18486,
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$npmCommand = (Get-Command npm.cmd -ErrorAction Stop).Source
$cargoCommand = (Get-Command cargo.exe -ErrorAction Stop).Source
$webRoot = Join-Path $repoRoot 'frontend/dist'

if ($SkipBuild -and -not (Test-Path -LiteralPath (Join-Path $webRoot 'index.html'))) {
    throw 'The frontend has not been built. Run without -SkipBuild first.'
}

$listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $Port)
try {
    $listener.Server.ExclusiveAddressUse = $true
    $listener.Start()
}
catch {
    throw "Cannot listen on 127.0.0.1:$Port. Stop its existing server or choose another -Port."
}
finally {
    $listener.Stop()
}

# Ignore inherited deployment/provider overrides for this run, then restore them
# when invoked from an existing PowerShell session.
$previousEnvironment = @{}
foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
    if ($entry.Key.StartsWith('THELXINOE_', [StringComparison]::OrdinalIgnoreCase)) {
        $previousEnvironment[$entry.Key] = $entry.Value
    }
}

$runName = '{0}-{1}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'), [guid]::NewGuid().ToString('N')
$runRoot = Join-Path $repoRoot ".local/dev-runs/$runName"
$runEnvironment = @{
    THELXINOE_STATE = Join-Path $runRoot 'server'
    THELXINOE_CACHE = Join-Path $runRoot 'cache'
    THELXINOE_MEDIA = Join-Path $runRoot 'media'
    THELXINOE_BACKUPS = Join-Path $runRoot 'backups'
    THELXINOE_WEB = $webRoot
    THELXINOE_BIND = "127.0.0.1:$Port"
    THELXINOE_PUBLIC_URL = "http://127.0.0.1:$Port"
    THELXINOE_DISCOVERY = 'false'
    THELXINOE_CONTROLLER_SOCKET = Join-Path $runRoot 'controller.sock'
}

Push-Location -LiteralPath $repoRoot
try {
    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    foreach ($name in $runEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $runEnvironment[$name], 'Process')
    }

    if (-not $SkipBuild) {
        & $npmCommand run build
        if ($LASTEXITCODE -ne 0) {
            throw "Frontend build failed (exit $LASTEXITCODE)."
        }
    }

    New-Item -ItemType Directory -Path $runEnvironment.THELXINOE_MEDIA -Force | Out-Null
    Write-Host "Fresh instance: $($runEnvironment.THELXINOE_PUBLIC_URL)"
    Write-Host "Data directory: $runRoot"
    Write-Host "Setup code file (created at startup): $($runEnvironment.THELXINOE_STATE)/secrets/setup-token"
    Write-Host 'Press Ctrl+C to stop. This run remains on disk; the next invocation starts empty.'

    & $cargoCommand run --locked -p thelxinoe-server
    if ($LASTEXITCODE -ne 0) {
        throw "Development server exited with code $LASTEXITCODE."
    }
}
finally {
    foreach ($name in $runEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    Pop-Location
}
