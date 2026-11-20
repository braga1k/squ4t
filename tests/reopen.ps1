# Opens the panel, closes it with Escape and opens it again, twice, and requires each reopened
# panel to match the first one pixel for pixel. Windows drops a window's pixels when it is hidden
# or leaves see-through mode; this catches a panel that comes back blank or half drawn.
# Needs an interactive desktop. Uses its own data folder.
param([string]$Exe = "$PSScriptRoot\..\target\release\squ4t.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if (-not ('ReopenShot' -as [type])) {
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class ReopenShot {
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L,T,Ri,B; }
  [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32")] public static extern bool SetProcessDPIAware();
}
"@ }
[ReopenShot]::SetProcessDPIAware() | Out-Null
$dir = Join-Path ([IO.Path]::GetTempPath()) "squ4t-reopen-$PID"
New-Item -ItemType Directory -Force $dir | Out-Null
$env:SQU4T_DATA_DIR = $dir
function Shot($app) {
    $h = (Get-Process -Id $app.Id).MainWindowHandle
    $r = New-Object ReopenShot+R; [ReopenShot]::GetWindowRect($h, [ref]$r) | Out-Null
    $bmp = New-Object Drawing.Bitmap ($r.Ri - $r.L), ($r.B - $r.T)
    [Drawing.Graphics]::FromImage($bmp).CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $bmp
}
function Difference($a, $b) {
    $diff = 0; $total = 0
    for ($y = 0; $y -lt [Math]::Min($a.Height, $b.Height); $y += 2) { for ($x = 0; $x -lt [Math]::Min($a.Width, $b.Width); $x += 2) {
        $total++; $p = $a.GetPixel($x, $y); $q = $b.GetPixel($x, $y)
        if ([Math]::Abs($p.R - $q.R) + [Math]::Abs($p.G - $q.G) + [Math]::Abs($p.B - $q.B) -gt 24) { $diff++ } } }
    $diff / $total
}
$app = Start-Process $Exe -ArgumentList '--ui-test' -PassThru
try {
    Start-Sleep -Milliseconds 1800
    $first = Shot $app
    $cli = Join-Path (Split-Path $Exe) 'squ4t-cli.exe'
    $ws = New-Object -ComObject wscript.shell
    foreach ($round in 1, 2) {
        $null = $ws.AppActivate($app.Id); Start-Sleep -Milliseconds 300
        $ws.SendKeys('{ESC}'); Start-Sleep -Milliseconds 600
        & $cli open; Start-Sleep -Milliseconds 700
        $again = Shot $app
        $d = Difference $first $again
        '{0}  reopen {1}: {2:P1} of pixels differ from the first open' -f $(if ($d -lt 0.005) { 'ok ' } else { 'BAD' }), $round, $d
        if ($d -ge 0.005) { throw "the reopened panel is not the same picture" }
    }
    'PASS'
} finally {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
