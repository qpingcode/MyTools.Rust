param([switch]$SkipBuild, [ValidateSet('debug', 'release')][string]$Profile = 'release')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $SkipBuild) {
        $arguments = @{ BuildOnly = $true }
        if ($Profile -eq 'release') { $arguments.Release = $true }
        & ./run.ps1 @arguments
    }
    $source = Join-Path $root "target/$Profile"
    $destination = Join-Path $root 'dist'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    # A fresh staging directory prevents old plugins and user data entering the archive.
    $stage = Join-Path $destination ("stage-" + [guid]::NewGuid().ToString('N'))
    $package = Join-Path $stage 'MyTools'
    New-Item -ItemType Directory -Path $package | Out-Null
    Copy-Item -LiteralPath (Join-Path $source 'mytools.exe') -Destination $package
    Copy-Item -LiteralPath (Join-Path $root 'config.example.json') -Destination (Join-Path $package 'config.json')
    Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $package
    Copy-Item -LiteralPath (Join-Path $root 'docs') -Destination $package -Recurse
    $licenses = Join-Path $package 'licenses'
    New-Item -ItemType Directory -Path $licenses | Out-Null
    Copy-Item -LiteralPath (Join-Path $root 'ui/assets/icons/LICENSE') -Destination (Join-Path $licenses 'Lucide.txt')
    foreach ($manifest in Get-ChildItem -Path (Join-Path $root 'plugins/*/plugin.json')) {
        $plugin = $manifest.Directory.Name
        $metadata = Get-Content -LiteralPath $manifest.FullName -Raw | ConvertFrom-Json
        $directory = Join-Path $package "plugins/$plugin"
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
        Copy-Item -LiteralPath $manifest.FullName -Destination $directory
        $executable = "$($metadata.entry).exe"
        Copy-Item -LiteralPath (Join-Path $source $executable) -Destination $directory
        Copy-Item -LiteralPath (Join-Path $manifest.Directory.FullName 'locales') -Destination $directory -Recurse
        $example = Join-Path $manifest.Directory.FullName 'config.example.json'
        if (Test-Path -LiteralPath $example) { Copy-Item -LiteralPath $example -Destination $directory }
    }
    $archive = Join-Path $destination 'MyTools-windows-x64-portable.zip'
    Compress-Archive -LiteralPath $package -DestinationPath $archive -Force
    $checksum = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    "$checksum  $([IO.Path]::GetFileName($archive))" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
    Write-Host "Package: $archive"
} finally {
    Pop-Location
}
