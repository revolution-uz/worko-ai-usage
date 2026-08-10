$ErrorActionPreference = "Stop"
$Repository = "revolution-uz/worko-ai-usage"
$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()

switch ($Architecture) {
    "x64" { $Cpu = "x86_64" }
    "arm64" { $Cpu = "aarch64" }
    default { throw "Unsupported architecture: $Architecture" }
}

$Asset = "worko-ai-usage-$Cpu-pc-windows-msvc.zip"
$Url = "https://github.com/$Repository/releases/latest/download/$Asset"
$InstallDir = Join-Path $env:LOCALAPPDATA "WorkoAiUsage"
$Executable = Join-Path $InstallDir "worko-ai-usage.exe"
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("worko-ai-usage-" + [guid]::NewGuid())

New-Item -ItemType Directory -Force -Path $TempDir, $InstallDir | Out-Null
try {
    $Archive = Join-Path $TempDir $Asset
    Write-Host "Downloading $Asset..."
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Archive
    $Checksums = Join-Path $TempDir "SHA256SUMS"
    Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$Repository/releases/latest/download/SHA256SUMS" -OutFile $Checksums
    $ChecksumLine = Get-Content $Checksums | Where-Object { $_ -match "\s$([regex]::Escape($Asset))$" } | Select-Object -First 1
    if (-not $ChecksumLine) { throw "No checksum published for $Asset" }
    $Expected = ($ChecksumLine -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) { throw "Checksum verification failed for $Asset" }
    Write-Host "SHA-256 checksum verified."
    Expand-Archive -Path $Archive -DestinationPath $TempDir -Force
    Copy-Item (Join-Path $TempDir "worko-ai-usage.exe") $Executable -Force
} finally {
    Remove-Item $TempDir -Recurse -Force -ErrorAction SilentlyContinue
}

$CurrentUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($CurrentUserPath -split ";") -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable("Path", (($CurrentUserPath.TrimEnd(";"), $InstallDir) -join ";"), "User")
}

$Action = New-ScheduledTaskAction -Execute $Executable -Argument "sync"
$Trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(5) -RepetitionInterval (New-TimeSpan -Hours 1)
$Settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 5)
Register-ScheduledTask -TaskName "Worko AI Usage" -Action $Action -Trigger $Trigger -Settings $Settings -Description "Sync Claude Code and Codex usage with Worko HR" -Force | Out-Null

Write-Host "Installed $Executable and enabled hourly sync."
& $Executable login @args
