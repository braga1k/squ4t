# The 1nstall Phase 0 ruler, for a tray app: time until the panel is up (--startup-test, median of
# warm runs), then the resident process with its panel closed: memory and CPU over a minute.
param([string]$Exe = "$PSScriptRoot\..\target\release\squ4t.exe", [int]$Warm = 10, [int]$IdleSeconds = 60)
$ErrorActionPreference = 'Stop'
$dir = Join-Path ([IO.Path]::GetTempPath()) "squat-measure-$PID"
New-Item -ItemType Directory -Force $dir | Out-Null
New-Item -ItemType File -Force (Join-Path $dir 'autostart-chosen') | Out-Null  # never touch the Run key
$env:SQU4T_DATA_DIR = $dir
$log = Join-Path (Split-Path $Exe) 'startup-test.log'
$ready = @(); $peak = @()
for ($i = 0; $i -le $Warm; $i++) {
    Remove-Item $log -ErrorAction SilentlyContinue
    $p = Start-Process $Exe -ArgumentList '--startup-test' -PassThru; $null = $p.Handle
    $max = 0L
    while (-not $p.HasExited) { try { $p.Refresh(); $max = [Math]::Max($max, $p.PeakWorkingSet64) } catch { }; Start-Sleep -Milliseconds 5 }
    if ($i -gt 0 -and (Test-Path $log)) { $ready += [int]((Get-Content $log) -replace 'READY ', ''); $peak += [Math]::Round($max / 1MB, 1) }
}
function Median($v) { $s = @($v | Sort-Object); $s[[int][Math]::Floor($s.Count / 2)] }
$p = Start-Process $Exe -PassThru; $null = $p.Handle
Start-Sleep -Seconds 5
$p.Refresh(); $cpu0 = $p.TotalProcessorTime
Start-Sleep -Seconds $IdleSeconds
$p.Refresh(); $cpu1 = $p.TotalProcessorTime
$result = [ordered]@{
    ExeMB = [Math]::Round((Get-Item $Exe).Length / 1MB, 2)
    PanelReadyMsMedian = Median $ready; PanelReadyMsMin = ($ready | Measure-Object -Minimum).Minimum; PanelReadyMsMax = ($ready | Measure-Object -Maximum).Maximum
    StartupPeakWorkingSetMBMedian = Median $peak
    IdleWorkingSetMB = [Math]::Round($p.WorkingSet64 / 1MB, 1); IdlePrivateMB = [Math]::Round($p.PrivateMemorySize64 / 1MB, 1)
    IdleCpuMsPerMinute = [Math]::Round(($cpu1 - $cpu0).TotalMilliseconds * 60 / $IdleSeconds, 1)
    Threads = $p.Threads.Count
}
Stop-Process -Id $p.Id -Force
Start-Sleep -Milliseconds 300
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
$result | ConvertTo-Json
