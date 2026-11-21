param(
    [Parameter(Mandatory)] [ValidatePattern('^[0-9a-f]{40}$')] [string] $Revision,
    [string] $Stage
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repository = 'F:\CODING\Velune'
$workspace = 'F:\CODING\.worktrees\yyh-win-install'
$checkout = Join-Path $workspace 'velune'

function Invoke-Git([string] $Directory, [string[]] $Arguments) {
    & git -C $Directory @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Git failed: $($Arguments -join ' ')" }
}

function Copy-Tree([string] $From, [string] $To) {
    & robocopy $From $To /E /XJ /R:2 /W:1 /NFL /NDL /NJH /NJS /NP
    if ($LASTEXITCODE -ge 8) { throw "Copy failed: $From -> $To (robocopy $LASTEXITCODE)." }
}

$programs = Join-Path $env:LOCALAPPDATA 'Programs'
$install = Join-Path $programs 'Velune'
$executable = Join-Path $install 'Velune.exe'
$result = Join-Path $PSScriptRoot 'result.json'

function Get-InstalledVelune {
    Get-Process -Name Velune -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $executable -and -not $_.HasExited }
}

function Move-InstallationAside([string] $From, [string] $Backup) {
    $wait = [Diagnostics.Stopwatch]::StartNew()
    $retries = 0
    while ($true) {
        try {
            [IO.Directory]::Move($From, $Backup)
            return $retries
        } catch {
            $failure = $_.Exception.GetBaseException()
            $code = $failure.HResult -band 0xffff
            # Open descendants can surface as access-denied, sharing or lock IO errors.
            if ($failure -isnot [IO.IOException] -or $code -notin @(5, 32, 33) -or
                $wait.Elapsed.TotalSeconds -ge 15 -or
                -not (Test-Path -LiteralPath $From -PathType Container) -or
                (Test-Path -LiteralPath $Backup)) { throw }
            $retries++
            Start-Sleep -Milliseconds 250
        }
    }
}

function Install-InDesktop([string] $StagedApp) {
    $session = (Get-Process -Id $PID).SessionId
    if ($session -eq 0) { throw 'The install must run in the logged-in Windows desktop.' }
    $destination = Get-Item -LiteralPath $install
    if (-not $destination.PSIsContainer -or ($destination.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        -not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Expected the manual Velune installation at $install."
    }
    if ($env:VELUNE_HOME) {
        $data = [IO.Path]::GetFullPath($env:VELUNE_HOME).TrimEnd('\')
        if ($data -eq $install -or $data.StartsWith($install + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'VELUNE_HOME points inside the installation directory.'
        }
    }
    $running = @(Get-InstalledVelune)
    if ($running.Count -gt 1 -or ($running.Count -eq 1 -and $running[0].SessionId -ne $session)) {
        throw 'Close the existing Velune installation in its own desktop, then retry.'
    }
    if ($running.Count -eq 1) {
        $old = $running[0]
        [void]$old.Handle
        $windows = [Collections.Generic.HashSet[IntPtr]]::new()
        $timeout = [Diagnostics.Stopwatch]::StartNew()
        while (-not $old.HasExited -and $timeout.Elapsed.TotalSeconds -lt 45) {
            $old.Refresh()
            $handle = $old.MainWindowHandle
            if ($handle -ne [IntPtr]::Zero -and $windows.Add($handle)) {
                [void]$old.CloseMainWindow()
            }
            Start-Sleep -Milliseconds 250
        }
        if (-not $old.HasExited) {
            throw 'Velune did not exit. Finish any prompts on YYH-WS or close it normally, then retry.'
        }
        $exitCode = $old.ExitCode
        $old.Dispose()
        if ($exitCode -ne 0) { throw "Velune exited with code $exitCode. Retry after checking its desktop." }
    }
    if (@(Get-InstalledVelune).Count -gt 0) { throw 'Velune reopened before the update. Retry the command.' }
    $backup = $StagedApp + '-previous'
    $moveRetries = Move-InstallationAside $install $backup
    try {
        [IO.Directory]::Move($StagedApp, $install)
    } catch {
        [IO.Directory]::Move($backup, $install)
        throw
    }

    try {
        $app = Start-Process -FilePath $executable -WorkingDirectory $install -PassThru
        $timeout = [Diagnostics.Stopwatch]::StartNew()
        do {
            Start-Sleep -Milliseconds 300
            $app.Refresh()
            if ($app.HasExited) { throw "The new process exited ($($app.ExitCode))." }
        } while ($app.MainWindowHandle -eq [IntPtr]::Zero -and $timeout.Elapsed.TotalSeconds -lt 45)
        if ($app.MainWindowHandle -eq [IntPtr]::Zero -or $app.SessionId -ne $session) {
            throw 'The new application did not open a window in the expected desktop.'
        }
    } catch {
        throw "The new files were installed but startup could not be confirmed. Previous files: $backup. $_"
    }
    Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction Continue
    [ordered]@{ ok = $true; revision = $Revision; path = $executable; pid = $app.Id; session = $session; directoryMoveRetries = $moveRetries }
}

if ($Stage) {
    try {
        $receipt = Install-InDesktop $Stage
    } catch {
        $receipt = @{ ok = $false; error = $_.Exception.Message }
    }
    $receipt | ConvertTo-Json -Compress | Set-Content -LiteralPath "$result.tmp" -Encoding UTF8
    Move-Item -LiteralPath "$result.tmp" -Destination $result
    return
}

New-Item -ItemType Directory -Force -Path $workspace | Out-Null
$lock = [IO.File]::Open((Join-Path $workspace 'update.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
try {
    if (-not (Test-Path -LiteralPath $checkout)) {
        Invoke-Git $repository @('worktree', 'add', '--detach', $checkout, 'HEAD')
    }
    $common = Invoke-Git $checkout @('rev-parse', '--path-format=absolute', '--git-common-dir')
    if ($common -ne (Invoke-Git $repository @('rev-parse', '--path-format=absolute', '--git-common-dir'))) {
        throw 'The build checkout does not belong to the YYH-WS Velune repository.'
    }
    $dirty = Invoke-Git $checkout @('status', '--porcelain')
    if ($dirty -or (Invoke-Git $checkout @('rev-parse', '--abbrev-ref', 'HEAD')) -ne 'HEAD') {
        throw 'The dedicated build checkout must be clean and detached.'
    }
    Invoke-Git $checkout @('fetch', '--no-tags', (Join-Path $PSScriptRoot 'source.bundle'), 'HEAD')
    if ((Invoke-Git $checkout @('rev-parse', 'FETCH_HEAD')) -ne $Revision) {
        throw 'The transferred commit does not match the requested build.'
    }
    Invoke-Git $checkout @('switch', '--detach', $Revision)
    foreach ($tool in 'runtime-support', 'tools') {
        $cached = Join-Path $checkout "target\$tool"
        if (-not (Test-Path -LiteralPath $cached)) {
            Copy-Tree (Join-Path $repository "target\$tool") $cached
        }
    }
    Write-Output "Building Velune from $Revision on $env:COMPUTERNAME."
    $env:CARGO_BUILD_JOBS = '2'
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $checkout 'scripts\build-windows.ps1') -Profile dev
    if ($LASTEXITCODE -ne 0) { throw "Velune build failed ($LASTEXITCODE)." }

    $source = Join-Path $checkout 'target\windows-dev\Velune'
    $stagePath = Join-Path $programs ('.Velune-update-' + [Guid]::NewGuid().ToString('N'))
    $taskName = 'Velune-Update-' + [Guid]::NewGuid().ToString('N')
    $registered = $false
    try {
        Copy-Tree $source $stagePath
        foreach ($file in 'Velune.exe', 'Velune.dll', 'velune_bindings.dll', 'velune_relaykit.dll') {
            if ((Get-FileHash -LiteralPath (Join-Path $source $file)).Hash -ne
                (Get-FileHash -LiteralPath (Join-Path $stagePath $file)).Hash) {
                throw "Staged file does not match the build: $file"
            }
        }
        Set-Content -LiteralPath (Join-Path $stagePath 'velune-source-revision.txt') -Value $Revision -Encoding ASCII
        $arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Revision $Revision -Stage `"$stagePath`""
        $action = New-ScheduledTaskAction -Execute (Join-Path $PSHOME 'powershell.exe') -Argument $arguments
        $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
        $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive
        $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
        Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Settings $settings | Out-Null
        $registered = $true
        Write-Output 'Build and staging verified. Updating the installed application in the Windows desktop.'
        Start-ScheduledTask -TaskName $taskName
        $deadline = (Get-Date).AddSeconds(125)
        while (-not (Test-Path -LiteralPath $result) -and (Get-Date) -lt $deadline) {
            Start-Sleep -Milliseconds 500
        }
        if (-not (Test-Path -LiteralPath $result)) {
            $info = Get-ScheduledTaskInfo -TaskName $taskName
            throw "No desktop update receipt. Ensure $user is logged in (task result $($info.LastTaskResult))."
        }
        $receipt = Get-Content -Raw -LiteralPath $result -Encoding UTF8 | ConvertFrom-Json
        if (-not $receipt.ok) { throw $receipt.error }
        Write-Output ($receipt | ConvertTo-Json -Compress)
    } finally {
        if ($registered) { Unregister-ScheduledTask -TaskName $taskName -Confirm:$false }
        if (Test-Path -LiteralPath $stagePath) { Remove-Item -LiteralPath $stagePath -Recurse -Force }
    }
} finally {
    $lock.Dispose()
}
