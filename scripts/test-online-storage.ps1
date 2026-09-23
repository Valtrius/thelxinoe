$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'online-storage.ps1')

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('thelxinoe-online-storage-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
try {
    $first = Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $true
    if ($first -notmatch '^thelxinoe-online-[a-f0-9]{32}$') { throw 'Fresh profiles must not use the legacy volume.' }
    $reused = Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $false
    if ($reused -ne $first) { throw 'Restarting a saved profile must retain its controller storage.' }
    $reset = Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $true
    if ($reset -eq $first) { throw 'Resetting a profile at the same path must allocate new storage.' }

    Remove-Item -LiteralPath (Join-Path $testRoot 'controller-storage-id')
    $legacy = Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $false
    if ($legacy -ne 'thelxinoe-online') { throw 'Existing profiles must retain their saved legacy volumes.' }
    $legacyAgain = Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $false
    if ($legacyAgain -ne $legacy) { throw 'Legacy selection must remain stable.' }

    Set-Content -LiteralPath (Join-Path $testRoot 'controller-storage-id') -Value '../invalid'
    $rejected = $false
    try { Resolve-OnlineStoragePrefix -StateRoot $testRoot -BasePrefix 'thelxinoe-online' -FirstLaunch $false | Out-Null }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Invalid saved identities must be rejected.' }
    Write-Host 'Online storage isolation, reuse, reset, and legacy preservation passed.'
}
finally {
    $resolvedTestRoot = [IO.Path]::GetFullPath($testRoot)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if (-not $resolvedTestRoot.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        (Split-Path -Leaf $resolvedTestRoot) -notmatch '^thelxinoe-online-storage-[a-f0-9]{32}$') {
        throw 'Refusing to clean an unexpected test directory.'
    }
    Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
}
