# Builds the workspace in release and updates an installed copy of MyTools in place.
# Only changed files are replaced; user data lives in %LOCALAPPDATA%\MyTools, outside the
# install folder, and is never touched. Running executables are renamed to *.old before the copy,
# changed plugins are stopped so MyTools restarts them, and MyTools itself is restarted
# when mytools.exe changed.
param(
    [string]$Target = 'C:\soft\MyTools',
    [switch]$SkipBuild,
    [switch]$NoRestart
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$source = Join-Path $root 'target/release'

# Loads a complete C++ toolset when this shell has none.
. (Join-Path $PSScriptRoot 'dev-environment.ps1')

# Copies one file when its content differs; a running target is renamed out of the way.
# Returns $true when the file was replaced or added.
function Update-File([string]$From, [string]$To) {
    if ((Test-Path -LiteralPath $To) -and
        (Get-FileHash -LiteralPath $From).Hash -eq (Get-FileHash -LiteralPath $To).Hash) {
        return $false
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $To -Parent) | Out-Null
    if (Test-Path -LiteralPath $To) {
        $old = "$To.old"
        try { Remove-Item -LiteralPath $old -Force -ErrorAction Stop } catch [System.Management.Automation.ItemNotFoundException] {} catch {
            # The previous .old is still running; keep it and use a unique name.
            $old = "$To.$([DateTime]::Now.Ticks).old"
        }
        Move-Item -LiteralPath $To -Destination $old
    }
    Copy-Item -LiteralPath $From -Destination $To
    $script:updated++
    Write-Host "  updated $($To.Substring($Target.Length + 1))"
    return $true
}

# Removes leftovers from earlier updates; files still in use stay until next time.
function Remove-OldFiles {
    Get-ChildItem -Path $Target -Recurse -Filter '*.old' -File -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }
}

function Stop-Executable([string]$Path) {
    Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Path } |
        Stop-Process -Force -ErrorAction SilentlyContinue
}

Push-Location $root
try {
    if (-not (Test-Path -LiteralPath (Join-Path $Target 'mytools.exe'))) {
        throw "MyTools is not installed in $Target"
    }
    if (-not $SkipBuild) {
        Enter-DeveloperEnvironment
        & cargo build --workspace --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    }
    Remove-OldFiles
    $script:updated = 0
    Write-Host "Updating $Target"

    $hostChanged = Update-File (Join-Path $source 'mytools.exe') (Join-Path $Target 'mytools.exe')
    $null = Update-File (Join-Path $root 'config.example.json') (Join-Path $Target 'config.example.json')
    $null = Update-File (Join-Path $root 'README.md') (Join-Path $Target 'README.md')
    $null = Update-File (Join-Path $root 'ui/assets/icons/LICENSE') (Join-Path $Target 'licenses/Lucide.txt')
    foreach ($doc in Get-ChildItem -Path (Join-Path $root 'docs') -Recurse -File) {
        $relative = $doc.FullName.Substring((Join-Path $root 'docs').Length + 1)
        $null = Update-File $doc.FullName (Join-Path $Target "docs/$relative")
    }

    $restartPlugins = @()
    foreach ($manifest in Get-ChildItem -Path (Join-Path $root 'plugins/*/plugin.json')) {
        $plugin = $manifest.Directory.Name
        $entry = "$((Get-Content -LiteralPath $manifest.FullName -Raw | ConvertFrom-Json).entry).exe"
        $directory = Join-Path $Target "plugins/$plugin"
        $changed = Update-File (Join-Path $source $entry) (Join-Path $directory $entry)
        $changed = (Update-File $manifest.FullName (Join-Path $directory 'plugin.json')) -or $changed
        foreach ($locale in Get-ChildItem -Path (Join-Path $manifest.Directory.FullName 'locales') -Filter '*.json') {
            $changed = (Update-File $locale.FullName (Join-Path $directory "locales/$($locale.Name)")) -or $changed
        }
        $skills = Join-Path $manifest.Directory.FullName 'skills'
        if (Test-Path -LiteralPath $skills) {
            foreach ($skill in Get-ChildItem -Path (Join-Path $skills '*/SKILL.md') -File) {
                $changed = (Update-File $skill.FullName (Join-Path $directory "skills/$($skill.Directory.Name)/SKILL.md")) -or $changed
            }
        }
        # Examples such as config.example.json and mcp.example.json; the user's own copies live
        # in %LOCALAPPDATA%\MyTools\<plugin>.
        foreach ($example in Get-ChildItem -Path $manifest.Directory.FullName -Filter '*.example.json') {
            $changed = (Update-File $example.FullName (Join-Path $directory $example.Name)) -or $changed
        }
        if ($changed) { $restartPlugins += Join-Path $directory $entry }
    }

    # Remove the retired bundled plugin from upgrades of existing installations.
    $removedPlugin = $false
    $installRoot = (Resolve-Path -LiteralPath $Target).ProviderPath
    $pluginsDirectory = Join-Path $installRoot 'plugins'
    $retiredDirectory = Join-Path $pluginsDirectory 'batch-rename'
    if (Test-Path -LiteralPath $retiredDirectory -PathType Container) {
        $retired = Get-Item -LiteralPath $retiredDirectory -Force
        foreach ($directory in @((Get-Item -LiteralPath $pluginsDirectory -Force), $retired)) {
            if ($directory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw "Cannot remove a retired plugin through a reparse point: $($directory.FullName)"
            }
        }
        Stop-Executable (Join-Path $retired.FullName 'mytools-batch-rename.exe')
        Remove-Item -LiteralPath $retired.FullName -Recurse -Force
        $removedPlugin = $true
        $script:updated++
        Write-Host '  removed plugins/batch-rename'
    }

    $mytools = Join-Path $Target 'mytools.exe'
    $running = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $mytools }
    if ($NoRestart) {
        Write-Host 'Skipping restarts (-NoRestart); changes apply after MyTools restarts.'
    } elseif ($hostChanged -or $removedPlugin) {
        # Restart to load the new host or refresh the installed plugin list.
        if ($running) {
            $running | Stop-Process -Force -ErrorAction SilentlyContinue
            Get-ChildItem -Path (Join-Path $Target 'plugins/*/*.exe') |
                ForEach-Object { Stop-Executable $_.FullName }
            # --restart waits for the stopped MyTools to release its single-instance lock.
            Start-Process -FilePath $mytools -WorkingDirectory $Target -ArgumentList '--restart' -WindowStyle Hidden
            Write-Host 'Restarted MyTools'
        }
    } else {
        # MyTools starts a stopped plugin again on its next request.
        foreach ($executable in $restartPlugins) {
            Stop-Executable $executable
            Write-Host "  restarted $(Split-Path $executable -Leaf)"
        }
    }
    if ($script:updated -eq 0) { Write-Host 'Already up to date' }
} finally {
    Pop-Location
}
