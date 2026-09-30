param(
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Phases = @(),
    [string]$PhaseList = '',
    [string]$OutputDirectory = '',
    [switch]$Background,
    [switch]$NoOpen,
    [switch]$NoNotify,
    [switch]$Prepared
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($PhaseList) { $Phases = $PhaseList.Split(',') }
foreach ($phase in $Phases) {
    if ($phase -notin @('server', 'web', 'containers', 'desktop', 'updates', 'updates-server', 'updates-desktop')) {
        throw "Invalid CI phase: $phase"
    }
}
$runDirectory = if ($OutputDirectory) {
    [System.IO.Path]::GetFullPath($OutputDirectory)
} else {
    Join-Path (Get-Location) ('.local/ci/' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N'))
}
if (-not $Prepared) {
    if (Test-Path -LiteralPath $runDirectory) { throw 'CI output directory already exists. Choose a new directory.' }
    New-Item -ItemType Directory -Path $runDirectory | Out-Null
}
if ($Background) {
    $arguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', ('"' + $PSCommandPath + '"'),
        '-OutputDirectory', ('"' + $runDirectory + '"'), '-Prepared'
    )
    if ($Phases.Count) { $arguments += @('-PhaseList', ($Phases -join ',')) }
    if ($NoOpen) { $arguments += '-NoOpen' }
    if ($NoNotify) { $arguments += '-NoNotify' }
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = 'powershell.exe'
    $start.Arguments = $arguments -join ' '
    $start.UseShellExecute = $true
    $start.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $runner = [System.Diagnostics.Process]::Start($start)
    $readyPath = Join-Path $runDirectory 'ready.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path -LiteralPath $readyPath)) {
        $runner.Refresh()
        if ($runner.HasExited -or [DateTime]::UtcNow -gt $deadline) {
            $failurePath = Join-Path $runDirectory 'result.json'
            $failure = if (Test-Path -LiteralPath $failurePath) { (Get-Content -LiteralPath $failurePath -Raw | ConvertFrom-Json).error } else { 'Coordinator did not acknowledge startup.' }
            Write-Error "$failure Report: $runDirectory\index.html; log: $runDirectory\output.log"
            exit 1
        }
        Start-Sleep -Milliseconds 100
    }
    $ready = Get-Content -LiteralPath $readyPath -Raw | ConvertFrom-Json
    $branch = if ($ready.origin.branch) { $ready.origin.branch } else { 'Detached HEAD' }
    Write-Output "Local CI started independently (PID $($runner.Id)): $branch"
    Write-Output "Worktree: $($ready.origin.worktree)"
    Write-Output "Report: $runDirectory\index.html"
    exit 0
}
$logPath = Join-Path $runDirectory 'output.log'
$resultPath = Join-Path $runDirectory 'result.json'
$started = (Get-Date).ToUniversalTime().ToString('o')
$exitCode = 1
try {
    $nodeArguments = @('scripts/ci-local.mjs', '--output', $runDirectory)
    if ($NoOpen) { $nodeArguments += '--no-open' }
    $nodeArguments += $Phases
    $quotedArguments = $nodeArguments | ForEach-Object { '"' + $_ + '"' }
    $coordinator = Start-Process -FilePath (Get-Command node).Source -ArgumentList $quotedArguments `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput $logPath `
        -RedirectStandardError (Join-Path $runDirectory 'coordinator-errors.log')
    $coordinator.WaitForExit()
    $exitCode = $coordinator.ExitCode
} catch {
    $_ | Out-String | Add-Content -LiteralPath $logPath
} finally {
    if (Test-Path -LiteralPath $resultPath) {
        $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
        if (-not $result.finished) {
            $recoveryArguments = @('scripts/ci-local.mjs', '--recover', $runDirectory) | ForEach-Object { '"' + $_ + '"' }
            $recovery = Start-Process -FilePath (Get-Command node).Source -ArgumentList $recoveryArguments `
                -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runDirectory 'recovery.log') `
                -RedirectStandardError (Join-Path $runDirectory 'recovery-errors.log')
            $recovery.WaitForExit()
            $exitCode = 1
        }
    } else {
        @{
            passed = $false
            exit_code = $exitCode
            started = $started
            finished = (Get-Date).ToUniversalTime().ToString('o')
            origin = @{ worktree = (Get-Location).Path; branch = (& git branch --show-current) }
            error = 'The local CI coordinator did not produce a result. See output.log.'
        } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $resultPath
    }
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    if (-not $result.passed) { $exitCode = 1 }
    if (-not $NoNotify) {
        $laneSummary = ($result.lanes | ForEach-Object { "$($_.phase): $($_.state)" }) -join "`n"
        Add-Type -AssemblyName System.Windows.Forms
        $outcome = if ($exitCode -eq 0) { 'passed' } else { 'failed' }
        $branch = if ($result.origin.branch) { $result.origin.branch } else { 'Detached HEAD' }
        [System.Windows.Forms.MessageBox]::Show(
            "$branch`n$($result.origin.worktree)`nLocal CI $outcome.`n$laneSummary`n$($result.error)`nSummary: $runDirectory\index.html`nLog: $logPath",
            "Thelxinoe CI $($result.id) finished",
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Information
        ) | Out-Null
    }
}
exit $exitCode
