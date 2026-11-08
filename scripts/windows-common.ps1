# Shared Windows SDK helpers; dot-source from packaging and desktop checks.
function Get-WindowsManifestTool {
    $manifestTool = Get-Command mt.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($manifestTool) { return $manifestTool.Source }

    $kitsRoot = (Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue).KitsRoot10
    if ($kitsRoot) {
        $manifestToolPath = Get-ChildItem -LiteralPath (Join-Path $kitsRoot 'bin') -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/mt.exe' } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
            Select-Object -First 1
        if ($manifestToolPath) { return $manifestToolPath }
    }
    throw 'Windows SDK manifest tool (mt.exe) was not found. Install the Windows SDK or add mt.exe to PATH.'
}

function Set-WindowsApplicationManifest {
    param(
        [Parameter(Mandatory = $true)][string]$ExecutablePath,
        [Parameter(Mandatory = $true)][string]$ManifestPath
    )
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        throw "Executable was not found: $ExecutablePath"
    }
    if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
        throw "Application manifest was not found: $ManifestPath"
    }
    $executableFile = (Resolve-Path -LiteralPath $ExecutablePath).ProviderPath
    $manifestFile = (Resolve-Path -LiteralPath $ManifestPath).ProviderPath
    $manifestToolPath = Get-WindowsManifestTool
    # GPUI links its own manifest. Replace it so app and desktop fixtures use
    # the same Windows compatibility declaration, including layered children.
    & $manifestToolPath -nologo -manifest $manifestFile "-outputresource:$executableFile;#1"
    if ($LASTEXITCODE -ne 0) { throw "Application manifest embedding failed: $executableFile" }
}
