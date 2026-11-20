# Compiles 1nstall's src/motion.cs (fetched at a pinned commit, unmodified) with PoseDump.cs and
# writes oracle/golden/motion.json. Needs only Windows: the .NET Framework C# compiler and WPF.
param([string]$Commit = '4e4dc643ac1eadb5bf7d8c2ba17bd8184c66c0c1')
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) "squat-motion-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$motion = Join-Path $work 'motion.cs'
Invoke-WebRequest "https://raw.githubusercontent.com/braga1k/1nstall/$Commit/src/motion.cs" -OutFile $motion -UseBasicParsing
$fx = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319"
$wpf = "$fx\WPF"
$exe = Join-Path $work 'posedump.exe'
& "$fx\csc.exe" /nologo /optimize+ /out:$exe /r:"$wpf\PresentationFramework.dll" /r:"$wpf\PresentationCore.dll" /r:"$wpf\WindowsBase.dll" /r:"$fx\System.Xaml.dll" $motion (Join-Path $here 'PoseDump.cs')
if ($LASTEXITCODE) { throw "csc failed" }
& $exe (Join-Path $here '..\golden\motion.json')
(Get-FileHash $motion -Algorithm SHA256).Hash.ToLower() + '  motion.cs@' + $Commit | Set-Content (Join-Path $here 'SOURCE.sha256') -Encoding ascii
Remove-Item -Recurse -Force $work
