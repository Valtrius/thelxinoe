param(
    [Parameter(Mandatory = $true)][string]$LabRoot,
    [string]$ExecutableName = 'thelxinoe-desktop.exe',
    [switch]$Inspect
)
$ErrorActionPreference = 'Stop'
$labDirectory = (Resolve-Path -LiteralPath $LabRoot).Path
if ((Split-Path $labDirectory -Leaf) -notmatch '^thelxinoe-update-\d+$') {
    throw 'Not an update lab installation'
}
$installation = [IO.Path]::GetFullPath((Join-Path $labDirectory 'installed'))
if ((Split-Path $installation -Parent) -ne $labDirectory) {
    throw 'Installation is outside the update lab'
}
if ((Test-Path -LiteralPath $installation) -and ((Get-Item -LiteralPath $installation).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'Refusing to uninstall through a linked installation directory'
}
$uninstaller = Join-Path $installation 'uninstall.exe'
if ($ExecutableName -notin @('thelxinoe-desktop.exe', "$(Split-Path $labDirectory -Leaf).exe")) {
    throw 'Executable does not belong to this update lab'
}
$executable = Join-Path $installation $ExecutableName
function Get-LabRegistration {
    $roots = @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )
    Get-ItemProperty $roots -ErrorAction SilentlyContinue | Where-Object {
        $_.DisplayName -like 'Thelxinoe Update Lab *' -and
        $_.InstallLocation -and $_.InstallLocation.Trim('"').TrimEnd('\') -eq $installation -and
        $_.UninstallString -and $_.UninstallString.Trim('"') -eq $uninstaller
    }
}
if (-not $Inspect) {
    if (Test-Path -LiteralPath $uninstaller) {
        if ($ExecutableName -eq 'thelxinoe-desktop.exe' -and (Get-Process -Name 'thelxinoe-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -ne $executable })) {
            throw 'This legacy lab uninstaller would close another desktop. Close the other desktop before stopping this lab.'
        }
        $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Lab uninstaller exited with code $($process.ExitCode)" }
    } elseif (-not (Test-Path -LiteralPath $executable)) {
        # A manually deleted lab can leave only its exact Windows registration.
        Get-LabRegistration | ForEach-Object { Remove-Item -LiteralPath $_.PSPath -Recurse -Force }
    } else {
        throw 'The lab executable exists but its uninstaller is missing'
    }
}
$state = @{ registered = @(Get-LabRegistration).Count -ne 0; executable = Test-Path -LiteralPath $executable }
if (-not $Inspect -and ($state.registered -or $state.executable)) { throw 'Lab uninstall did not complete' }
$state | ConvertTo-Json -Compress
