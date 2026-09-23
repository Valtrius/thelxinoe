# Keep controller history tied to the server profile, including when a profile
# is reset at the same path. Existing profiles retain their legacy volume names.
function Resolve-OnlineStoragePrefix {
    param(
        [Parameter(Mandatory)][string]$StateRoot,
        [Parameter(Mandatory)][string]$BasePrefix,
        [Parameter(Mandatory)][bool]$FirstLaunch
    )

    $identityPath = Join-Path $StateRoot 'controller-storage-id'
    if ($FirstLaunch) {
        $identity = [guid]::NewGuid().ToString('N')
        Set-Content -LiteralPath $identityPath -Value $identity -Encoding ascii -NoNewline
    }
    elseif (Test-Path -LiteralPath $identityPath -PathType Leaf) {
        $identity = (Get-Content -LiteralPath $identityPath -Raw).Trim()
        if ($identity -ne 'legacy' -and $identity -notmatch '^[a-f0-9]{32}$') {
            throw "Invalid controller storage identity in $identityPath. Restore the identity that belongs to this server profile."
        }
    }
    else {
        $identity = 'legacy'
        Set-Content -LiteralPath $identityPath -Value $identity -Encoding ascii -NoNewline
    }

    if ($identity -eq 'legacy') { return $BasePrefix }
    return "$BasePrefix-$identity"
}
