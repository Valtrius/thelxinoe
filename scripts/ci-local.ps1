param(
    [string[]]$Phases = @(),
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$runDirectory = if ($OutputDirectory) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    Join-Path (Get-Location) ('.local/ci/' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $PID)
}
New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$logPath = Join-Path $runDirectory 'output.log'
$resultPath = Join-Path $runDirectory 'result.json'
$started = (Get-Date).ToUniversalTime().ToString('o')
$exitCode = 1
try {
    $ErrorActionPreference = 'Continue'
    & node scripts/ci-local.mjs --output $runDirectory @Phases *>&1 | Tee-Object -FilePath $logPath
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
} catch {
    $_ | Out-String | Add-Content -LiteralPath $logPath
} finally {
    if (-not (Test-Path -LiteralPath $resultPath)) {
        @{
            passed = $false
            exit_code = $exitCode
            started = $started
            finished = (Get-Date).ToUniversalTime().ToString('o')
            error = 'The local CI coordinator did not produce a result. See output.log.'
        } | ConvertTo-Json | Set-Content -LiteralPath $resultPath
    }
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    $laneSummary = ($result.lanes | ForEach-Object { "$($_.phase): $($_.state)" }) -join "`n"
    Add-Type -AssemblyName System.Windows.Forms
    $outcome = if ($exitCode -eq 0) { 'passed' } else { 'failed' }
    $runId = Split-Path -Leaf $runDirectory
    [System.Windows.Forms.MessageBox]::Show(
        "Local CI $outcome.`n$laneSummary`nSummary: $runDirectory\index.html`nLog: $logPath",
        "Thelxinoe CI $runId finished",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Information
    ) | Out-Null
}
exit $exitCode
