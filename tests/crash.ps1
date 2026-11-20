# Kills `squ4t-cli add 1` at random moments, many times, including while a write is held open just
# before its rename, then requires a ledger that store.py's rules still accept.
# Build first:  cargo build --release -p squ4t --features test-hooks
param([string]$Squats = "$PSScriptRoot\..\target\release\squ4t-cli.exe", [int]$Rounds = 300, [int]$Seed = 7)
$ErrorActionPreference = 'Stop'
$dir = Join-Path ([IO.Path]::GetTempPath()) "squat-crash-$PID"
New-Item -ItemType Directory -Force $dir | Out-Null
$env:SQU4T_DATA_DIR = $dir
$random = New-Object Random $Seed
$completed = 0; $killed = 0
for ($i = 0; $i -lt $Rounds; $i++) {
    $env:SQUAT_TEST_PAUSE_BEFORE_RENAME_MS = [string]$random.Next(0, 60)
    $p = Start-Process -FilePath $Squats -ArgumentList 'add','1' -PassThru -WindowStyle Hidden -RedirectStandardOutput "$dir\..\squat-crash-$PID.out"
    $null = $p.Handle  # keeps the exit code readable after the process ends
    Start-Sleep -Milliseconds $random.Next(15, 140)
    if (-not $p.HasExited) { try { $p.Kill(); $killed++ } catch { } }
    $p.WaitForExit()
    if ($p.ExitCode -eq 0) { $completed++ }
}
Remove-Item Env:SQUAT_TEST_PAUSE_BEFORE_RENAME_MS
$status = & $Squats status
$ok = $LASTEXITCODE -eq 0
$total = if ($ok) { ($status | ConvertFrom-Json).total } else { -1 }
$leftovers = @(Get-ChildItem $dir -Force -Filter '.ledger-*').Count
"rounds $Rounds; killed $killed; completed $completed; ledger readable: $ok; total $total; temporary files left by killed writers: $leftovers"
Remove-Item -Recurse -Force $dir
if (-not $ok) { throw 'the ledger was corrupted' }
# Every completed add is in the ledger; a killed one may have committed just before it died.
if ($total -lt $completed -or $total -gt $Rounds) { throw "total $total outside [$completed, $Rounds]" }
'PASS'
