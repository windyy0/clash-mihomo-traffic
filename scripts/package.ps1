param([switch]$SkipBuild, [switch]$ArchiveOnly, [string]$InnoCompiler)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-common.ps1')
$projectRoot = Split-Path -Parent $PSScriptRoot
$buildTarget = Join-Path $projectRoot 'target/package-build'
$buildExecutable = Join-Path $buildTarget 'release/mihomo-traffic.exe'
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
$localPaths = @()
$remapFlags = @()
foreach ($mapping in @(
    @($env:USERPROFILE, '/home'),
    @($cargoHome, '/cargo'),
    @($rustupHome, '/rustup'),
    @($projectRoot, '/workspace')
)) {
    $from, $to = $mapping
    if (-not $from) { continue }
    if (-not [IO.Path]::IsPathRooted($from)) { $from = Join-Path $projectRoot $from }
    $from = [IO.Path]::GetFullPath($from)
    foreach ($path in @($from, $from.Replace('\', '/')) | Select-Object -Unique) {
        $localPaths += $path
        $remapFlags += "--remap-path-prefix=$path=$to"
    }
}

Push-Location $projectRoot
$temporaryDestination = $null
try {
    if (-not $ArchiveOnly) {
        if (-not $InnoCompiler) {
            $InnoCompiler = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
        }
        if (-not $InnoCompiler) {
            $InnoCompiler = @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs')) |
                Where-Object { $_ } | ForEach-Object {
                    Join-Path $_ 'Inno Setup 6/ISCC.exe'
                    Join-Path $_ 'Inno Setup 7/ISCC.exe'
                } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        }
        if (-not $InnoCompiler -or -not (Test-Path -LiteralPath $InnoCompiler -PathType Leaf)) {
            throw 'Inno Setup compiler not found. Install Inno Setup or pass -InnoCompiler <path to ISCC.exe>. Use -ArchiveOnly to build just the portable ZIP.'
        }
    }
    $metadata = cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Failed to read package version' }
    $version = ($metadata | ConvertFrom-Json).packages | Where-Object name -eq 'mihomo-traffic' | Select-Object -ExpandProperty version
    $baseName = "MihomoTraffic-$version-windows-x64"
    if (-not $SkipBuild) {
        $oldFlags = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
        $oldCl = [Environment]::GetEnvironmentVariable('CL', 'Process')
        $oldCFlags = [Environment]::GetEnvironmentVariable('CFLAGS', 'Process')
        $separator = [char]31
        $flags = if ($null -ne $oldFlags) { $oldFlags -split [string]$separator } else { $env:RUSTFLAGS -split '\s+' }
        # MSVC native dependencies also embed paths through __FILE__ in assertions.
        $trimFlags = $localPaths | ForEach-Object { '"/d1trimfile:{0}"' -f ($_ -replace '(\\+)$', '$1$1') }
        try {
            $env:CARGO_ENCODED_RUSTFLAGS = (@($flags | Where-Object { $_ }) + $remapFlags) -join $separator
            $env:CL = ((@($oldCl) + $trimFlags) -join ' ').Trim()
            $env:CFLAGS = "$oldCFlags /Brepro".Trim()
            cargo build --release --locked --target-dir $buildTarget
            if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
        } finally {
            [Environment]::SetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', $oldFlags, 'Process')
            [Environment]::SetEnvironmentVariable('CL', $oldCl, 'Process')
            [Environment]::SetEnvironmentVariable('CFLAGS', $oldCFlags, 'Process')
        }
    }
    if (-not (Test-Path -LiteralPath $buildExecutable -PathType Leaf)) {
        throw 'No packaging build found. Run package.ps1 without -SkipBuild first.'
    }
    $dist = Join-Path $projectRoot 'dist'
    $temporaryDestination = Join-Path $projectRoot ('target/package-' + [guid]::NewGuid().ToString('N'))
    $destination = $temporaryDestination
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    $executable = Join-Path $destination 'mihomo-traffic.exe'
    Copy-Item -LiteralPath $buildExecutable -Destination $executable
    Set-WindowsApplicationManifest -ExecutablePath $executable -ManifestPath (Join-Path $projectRoot 'assets/app.manifest')
    # Use a byte-preserving encoding to check both UTF-8 and UTF-16 path strings.
    $binaryEncoding = [Text.Encoding]::GetEncoding(28591)
    $binary = $binaryEncoding.GetString([IO.File]::ReadAllBytes($executable))
    foreach ($path in $localPaths) {
        foreach ($encoding in @([Text.Encoding]::UTF8, [Text.Encoding]::Unicode)) {
            $needle = $binaryEncoding.GetString($encoding.GetBytes($path))
            if ($binary.IndexOf($needle, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw 'The release executable contains a local build path. Rebuild without -SkipBuild and check the compiler settings.'
            }
        }
    }
    Copy-Item -LiteralPath 'LICENSE' -Destination $destination
    & (Join-Path $PSScriptRoot 'third-party-notices.ps1') -Destination (Join-Path $destination 'THIRD-PARTY-NOTICES.txt')
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $artifacts = @()
    if (-not $ArchiveOnly) {
        & $InnoCompiler "/DAppVersion=$version" "/DPayloadDir=$destination" "/O$dist" (Join-Path $PSScriptRoot 'installer.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
        $artifacts += Join-Path $dist "$baseName-setup.exe"
    }
    # The marker belongs only in the portable archive. A fresh launch creates data/.
    New-Item -ItemType File -Path (Join-Path $destination 'portable.flag') | Out-Null
    $archive = Join-Path $dist "$baseName-portable.zip"
    Compress-Archive -Path "$destination/*" -DestinationPath $archive -Force
    $artifacts += $archive
    foreach ($artifact in $artifacts) {
        $hash = Get-FileHash -LiteralPath $artifact -Algorithm SHA256
        "$($hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $artifact)" | Set-Content -LiteralPath "$artifact.sha256" -Encoding ascii
        $hash
    }
} finally {
    try {
        if ($temporaryDestination -and (Test-Path -LiteralPath $temporaryDestination)) {
            $resolvedDestination = [IO.Path]::GetFullPath($temporaryDestination)
            $stagingRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target')) + [IO.Path]::DirectorySeparatorChar
            if (-not $resolvedDestination.StartsWith($stagingRoot, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Package staging directory is outside the build directory'
            }
            Remove-Item -LiteralPath $resolvedDestination -Recurse -Force
        }
    } finally { Pop-Location }
}
