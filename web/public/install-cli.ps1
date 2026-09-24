# Installs the Lorca CLI on Windows: https://lorca.app/docs/cli
#
#   irm https://lorca.app/install-cli.ps1 | iex
#
# It downloads the Windows build from the latest release of github.com/egoist/lorca, checks it
# against the checksum published beside it, puts lorca.exe in ~\.local\bin, and adds that folder
# to your user PATH. Run it again to update. Settings, as environment variables set before it
# runs:
#
#   $env:LORCA_VERSION = '1.0.0'        a release to install instead of the latest
#   $env:LORCA_INSTALL_DIR = 'C:\...'   where lorca.exe goes
#   $env:LORCA_NO_MODIFY_PATH = '1'     leave PATH alone
#
# It runs in a script block of its own, so iex leaves no variables behind in the session, and it
# never calls exit, which would close the window.

& {
    $ErrorActionPreference = 'Stop'
    # Windows PowerShell redraws its progress bar so often that the download slows down.
    $ProgressPreference = 'SilentlyContinue'
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $releases = 'https://github.com/egoist/lorca/releases'
    if ($env:LORCA_DOWNLOAD_URL) { $releases = $env:LORCA_DOWNLOAD_URL.TrimEnd('/') }

    $cpu = $null
    try { $cpu = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() } catch {}
    if (-not $cpu) { $cpu = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE } }
    if ($cpu -notmatch '^(x64|amd64|arm64)$') { throw "There is no Lorca CLI for Windows on $cpu yet." }
    # One build for both: Windows 11 on Arm runs x64 programs.
    $target = 'windows-x86_64'

    if ($env:LORCA_VERSION) {
        $version = $env:LORCA_VERSION.Trim().TrimStart('v')
        if ($version -notmatch '^[0-9.]+$') { throw "Not a Lorca version: $env:LORCA_VERSION" }
        $url = "$releases/download/cli-v$version"
        $release = "release $version"
    } else {
        $url = "$releases/latest/download"
        $release = 'the latest release'
    }

    $dir = $env:LORCA_INSTALL_DIR
    if (-not $dir) { $dir = Join-Path $HOME '.local\bin' }
    $exe = Join-Path $dir 'lorca.exe'
    $archive = "lorca-cli-$target.zip"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('lorca-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "Downloading lorca for $target from $release"
        $zip = Join-Path $tmp $archive
        try {
            Invoke-WebRequest -Uri "$url/$archive" -OutFile $zip -UseBasicParsing
            Invoke-WebRequest -Uri "$url/$archive.sha256" -OutFile "$zip.sha256" -UseBasicParsing
        } catch {
            throw "Could not download $archive from ${release}: $($_.Exception.Message)"
        }
        $expected = ((Get-Content -Path "$zip.sha256" -TotalCount 1).Trim() -split '\s+')[0]
        if ((Get-FileHash -Algorithm SHA256 -Path $zip).Hash -ne $expected) {
            throw "$archive does not match its checksum: the download was damaged, so run the installer again."
        }

        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $new = Join-Path $tmp 'lorca.exe'
        if (-not (Test-Path $new)) { throw "$archive holds no lorca.exe." }

        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        # A running lorca.exe cannot be replaced, but it can be renamed: move it aside, then delete
        # what is aside unless something still runs it, which a later install cleans up.
        if (Test-Path $exe) { Move-Item -Path $exe -Destination ($exe + '.old-' + [Guid]::NewGuid().ToString('N')) }
        Move-Item -Path $new -Destination $exe
        Get-ChildItem -Path $dir -Filter 'lorca.exe.old-*' | Remove-Item -Force -ErrorAction SilentlyContinue
        Unblock-File -Path $exe
        $installed = & $exe --version
        if ($LASTEXITCODE -ne 0) { throw "$exe does not start on this computer." }
    } finally {
        Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Installed $installed at $exe"
    $inDir = { param($entry) $entry -and [Environment]::ExpandEnvironmentVariables($entry).TrimEnd('\') -eq $dir.TrimEnd('\') }
    if ($env:LORCA_NO_MODIFY_PATH -eq '1') {
        if (-not ($env:Path -split ';' | Where-Object { & $inDir $_ })) { Write-Host "$dir is not on your PATH." }
    } else {
        # The registry value as written: [Environment]::GetEnvironmentVariable expands %VARIABLES%,
        # and SetEnvironmentVariable would write them back expanded.
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        try {
            $userPath = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $entries = @($userPath -split ';' | Where-Object { $_ })
            if (-not ($entries | Where-Object { & $inDir $_ })) {
                $key.SetValue('Path', (($entries + $dir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
                # A user variable set through .NET broadcasts the change, so terminals opened from
                # now on read the new PATH.
                [Environment]::SetEnvironmentVariable('LORCA_INSTALLER', '1', 'User')
                [Environment]::SetEnvironmentVariable('LORCA_INSTALLER', $null, 'User')
                Write-Host "Added $dir to your user PATH."
            }
        } finally {
            $key.Close()
        }
        # This window too.
        if (-not ($env:Path -split ';' | Where-Object { & $inDir $_ })) { $env:Path = "$env:Path;$dir" }
    }

    Write-Host ''
    $lorcaHome = $env:LORCA_HOME
    if (-not $lorcaHome) { $lorcaHome = Join-Path $HOME '.lorca' }
    if (Test-Path (Join-Path $lorcaHome 'machine.json')) {
        Write-Host 'If lorca serve is running, restart it to run the new version.'
    } else {
        Write-Host 'To make this computer a Runner, pair it with your account and start the service:'
        Write-Host "  lorca pair 'lorca://pair?...'   # from Pair a Device in the app"
        Write-Host '  lorca serve'
    }
    Write-Host 'Docs: https://lorca.app/docs/cli'
}
