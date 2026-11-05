# Outside a developer shell, Rust and the C build scripts pick the newest Visual Studio,
# even when its C++ toolset is incomplete (no headers or CRT libraries). Load the
# environment of the newest install whose toolset is complete instead.
function Enter-DeveloperEnvironment {
    if ($env:VCINSTALLDIR) { return }
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { return }
    $installs = & $vswhere -all -products * -sort -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    foreach ($vs in $installs) {
        $msvc = Get-ChildItem -Path "$vs\VC\Tools\MSVC" -Directory -ErrorAction SilentlyContinue |
            Sort-Object { [version]$_.Name } | Select-Object -Last 1
        $vcvars = "$vs\VC\Auxiliary\Build\vcvars64.bat"
        if (-not $msvc -or -not (Test-Path -LiteralPath $vcvars) -or
            -not (Test-Path -LiteralPath "$($msvc.FullName)\include\vcruntime.h") -or
            -not (Test-Path -LiteralPath "$($msvc.FullName)\lib\x64\msvcrt.lib")) { continue }
        # vcvars64.bat looks vswhere up on PATH.
        $env:PATH = "$(Split-Path $vswhere -Parent);$env:PATH"
        foreach ($line in & cmd.exe /d /c "`"$vcvars`" >nul && set") {
            if ($line -match '^([^=]+)=(.*)$') { Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2] }
        }
        Write-Host "Using C++ tools from $vs"
        return
    }
}
