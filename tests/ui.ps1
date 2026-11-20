# Drives the panel through UI Automation, as a screen reader would: every control has a name,
# and Record, Undo, Snooze, the Reminders switch and Save change the ledger as store.py would.
# Needs an interactive desktop. Uses its own data folder; never touches the user's ledger.
param([string]$Exe = "$PSScriptRoot\..\target\release\squ4t.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$dir = Join-Path ([IO.Path]::GetTempPath()) "squat-ui-$PID"
New-Item -ItemType Directory -Force $dir | Out-Null
$env:SQU4T_DATA_DIR = $dir
$app = Start-Process $Exe -ArgumentList '--ui-test' -PassThru
function Ledger { Get-Content (Join-Path $dir 'ledger.json') -Raw | ConvertFrom-Json }
function Wait-For([scriptblock]$Condition, [string]$What) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.ElapsedMilliseconds -lt 5000) { try { if (& $Condition) { return } } catch { }; Start-Sleep -Milliseconds 50 }
    throw "timed out: $What"
}
function Elements {
    $p = Get-Process -Id $app.Id
    $root = [Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
    $root.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
}
function Named([string]$Name, [string]$Type) { Elements | Where-Object { $_.Current.Name -eq $Name -and $_.Current.ControlType.ProgrammaticName -eq "ControlType.$Type" } | Select-Object -First 1 }
function Act([string]$Name, [string]$Type = 'Button') {
    # Controls are off for the moment a ledger operation runs (as in the plugin): wait for them.
    Wait-For { (Named $Name $Type).Current.IsEnabled } "'$Name' is enabled"
    $e = Named $Name $Type; if (-not $e) { throw "no $Type named '$Name'" }
    $o = $null
    if ($e.TryGetCurrentPattern([Windows.Automation.InvokePattern]::Pattern, [ref]$o)) { $o.Invoke() }
    elseif ($e.TryGetCurrentPattern([Windows.Automation.TogglePattern]::Pattern, [ref]$o)) { $o.Toggle() }
    else { throw "'$Name' cannot be activated" }
}
function Check($Condition, [string]$What) { if (-not $Condition) { throw "FAILED: $What" }; "ok  $What" }
try {
    Wait-For { (Get-Process -Id $app.Id).MainWindowHandle -ne 0 -and (Named 'Record 20 squats' 'Button').Current.IsEnabled } 'the panel is ready'
    foreach ($n in 'Record 20 squats', 'Undo last entry', 'Snooze 5 min', 'Save settings') { Check ([bool](Named $n 'Button')) "button '$n' is exposed" }
    foreach ($n in 'Squats completed', 'Interval (min)', 'Squats per break') { Check ([bool](Named $n 'Spinner')) "number field '$n' is exposed" }
    Check ([bool](Named 'Reminders' 'CheckBox')) 'the Reminders switch is a checkbox'
    Check (-not (Named 'Undo last entry' 'Button').Current.IsEnabled) 'Undo is disabled before any entry'
    $rect = (Get-Process -Id $app.Id).MainWindowHandle | ForEach-Object { [Windows.Automation.AutomationElement]::FromHandle($_).Current.BoundingRectangle }
    $last = (Named 'Save settings' 'Button').Current.BoundingRectangle
    $gap = [Math]::Round(($rect.Bottom - $last.Bottom) / ($rect.Height / 378), 1)
    Check ([Math]::Abs($gap - 16) -le 1) "the window ends 16 below the last row (measured $gap)"

    Act 'Record 20 squats'
    Wait-For { ((Ledger).days.PSObject.Properties | Measure-Object Value -Sum).Sum -eq 20 } 'Record adds 20'
    Check ((Ledger).last.count -eq 20) 'the entry can be undone'
    Wait-For { (Named 'Undo last entry' 'Button').Current.IsEnabled } 'Undo becomes enabled'
    Act 'Undo last entry'
    Wait-For { ((Ledger).days.PSObject.Properties | Measure-Object Value -Sum).Sum -eq 0 -and $null -eq (Ledger).last } 'Undo removes it'

    Act 'Snooze 5 min'
    Wait-For { (Ledger).remaining -eq 300 } 'Snooze sets five minutes'

    $toggle = Named 'Reminders' 'CheckBox'
    Act 'Reminders' 'CheckBox'
    Wait-For { (Ledger).paused -eq $true } 'the switch pauses reminders'
    Wait-For { [bool](Elements | Where-Object { $_.Current.Name -eq 'Reminders paused' }) } 'the status says so'
    Act 'Reminders' 'CheckBox'
    Wait-For { (Ledger).paused -eq $false } 'and resumes them'

    Check (-not (Named 'Save settings' 'Button').Current.IsEnabled) 'Save is off while the settings are unchanged'
    $field = Named 'Squats per break' 'Spinner'
    $range = $null
    if ($field.TryGetCurrentPattern([Windows.Automation.RangeValuePattern]::Pattern, [ref]$range)) { $range.SetValue(15) }
    else { $vp = $field.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern); $vp.SetValue('15') }
    Wait-For { (Named 'Save settings' 'Button').Current.IsEnabled } 'a changed field turns Save on'
    Act 'Save settings'
    Wait-For { (Ledger).suggested -eq 15 -and (Ledger).interval -eq 60 } 'Save stores 15 squats per break and keeps the interval'
    Wait-For { -not (Named 'Save settings' 'Button').Current.IsEnabled } 'once saved, Save is off again'
    'PASS'
} finally {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
