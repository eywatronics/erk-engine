<#
.SYNOPSIS
Erk's memory against Electron's and Chrome's (M5.13; the project owner's
decision, 2026-11-23): the same pages, the same window, the GPU on and off.

.DESCRIPTION
Each engine opens each page in an 800x600 window, once with the GPU and once
without: Erk's shell (`--cpu` without), Chrome as an app window with a fresh
profile (`--disable-gpu` without), and Electron with ./electron/main.js
(`app.disableHardwareAcceleration()` without). Once the page is shown and
the memory settles (the private bytes moving less than 1 MB over three
seconds, at most -Settle seconds), the private bytes and the working set of
the whole process tree are summed: Chrome and Electron run a browser, a GPU,
a page and utility processes, Erk one. Then the tree is stopped.

The bar: in the worst case, Erk uses at most half of Electron's memory, with
the GPU and without.

Windows only, by hand, like the Chrome reference captures: it opens windows,
and Electron is not a dependency of Erk. Before the first run:

    cargo build --release -p erk-shell
    cd tests/memory-compare/electron; npm ci

.EXAMPLE
pwsh tests/memory-compare/compare.ps1
#>
param(
    [string]$Erk = "$PSScriptRoot\..\..\target\release\erk.exe",
    [string]$Chrome = "C:\Program Files\Google\Chrome\Application\chrome.exe",
    [string]$Electron = "$PSScriptRoot\electron\node_modules\electron\dist\electron.exe",
    [int]$Settle = 40,
    # Report from the measurements a run saved, without measuring again.
    [string]$From,
    [string]$Save = "$PSScriptRoot\..\..\target\memory-compare.json"
)

$ErrorActionPreference = "Stop"
$pages = @(
    @{ name = "m2-demo.html"; path = (Resolve-Path "$PSScriptRoot\..\..\examples\m2-demo.html").Path },
    @{ name = "erk-10000.html"; path = (Resolve-Path "$PSScriptRoot\..\..\examples\erk-10000.html").Path }
)
foreach ($exe in @($Erk, $Chrome, $Electron)) {
    if (-not (Test-Path $exe)) { throw "not found: $exe (see the notes at the top)" }
}

# The process `root` and every process it started, and they started. Windows
# reuses process ids: a process whose parent is long gone may name, as its
# parent, the id `root` now has. Only those started after `root` count.
function Get-Tree([int]$root) {
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate
    $born = ($all | Where-Object { $_.ProcessId -eq $root }).CreationDate
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($root)
    for ($at = 0; $at -lt $ids.Count; $at++) {
        foreach ($child in $all | Where-Object { $_.ParentProcessId -eq $ids[$at] -and $_.CreationDate -ge $born }) {
            if (-not $ids.Contains([int]$child.ProcessId)) { $ids.Add([int]$child.ProcessId) }
        }
    }
    $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
}

function Measure-Tree([string]$exe, [string[]]$arguments) {
    $started = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    $history = @()
    $tree = @()
    for ($second = 0; $second -lt $Settle; $second++) {
        Start-Sleep -Seconds 1
        $tree = @(Get-Tree $started.Id)
        $private = ($tree | Measure-Object PrivateMemorySize64 -Sum).Sum
        $history += $private
        $n = $history.Count
        if ($n -ge 6 -and ($history[($n - 3)..($n - 1)] | Measure-Object -Maximum -Minimum |
                ForEach-Object { $_.Maximum - $_.Minimum }) -lt 1MB) { break }
    }
    $tree = @(Get-Tree $started.Id)
    $result = [pscustomobject]@{
        processes = $tree.Count
        private = ($tree | Measure-Object PrivateMemorySize64 -Sum).Sum
        working = ($tree | Measure-Object WorkingSet64 -Sum).Sum
        seconds = $history.Count
    }
    $tree | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    $result
}

$profiles = Join-Path ([System.IO.Path]::GetTempPath()) "erk-memory-compare"
$rows = @()
if ($From) {
    $rows = @(Get-Content $From -Raw | ConvertFrom-Json)
    $pages = @()
}
foreach ($page in $pages) {
    $url = ([System.Uri]$page.path).AbsoluteUri
    foreach ($gpu in @($true, $false)) {
        $profile = Join-Path $profiles ([guid]::NewGuid())
        $runs = @(
            @{ engine = "Erk"; exe = $Erk; args = @(if (-not $gpu) { "--cpu" }) + @("`"$($page.path)`"") },
            @{ engine = "Chrome"; exe = $Chrome; args = @("--user-data-dir=`"$profile`"", "--no-first-run",
                    "--no-default-browser-check", "--disable-extensions", "--window-size=800,600",
                    "--app=$url") + @(if (-not $gpu) { "--disable-gpu" }) },
            @{ engine = "Electron"; exe = $Electron; args = @("`"$PSScriptRoot\electron\main.js`"",
                    "`"$($page.path)`"") + @(if (-not $gpu) { "--no-gpu" }) }
        )
        foreach ($run in $runs) {
            Write-Host "$($page.name), $($run.engine), GPU $(if ($gpu) { 'on' } else { 'off' })..."
            $measured = Measure-Tree $run.exe $run.args
            $rows += [pscustomobject]@{
                page = $page.name; engine = $run.engine; gpu = $gpu
                processes = $measured.processes
                private = $measured.private; working = $measured.working
                seconds = $measured.seconds
            }
        }
        Remove-Item -Recurse -Force $profile -ErrorAction SilentlyContinue
    }
}

if (-not $From) {
    $rows | ConvertTo-Json | Set-Content -Encoding utf8 $Save
}
$pages = @($rows | Select-Object -ExpandProperty page -Unique | ForEach-Object { @{ name = $_ } })
$mb = { param($bytes) "{0:N1} MB" -f ($bytes / 1MB) }
"| Page | GPU | Engine | Processes | Private | Working set |"
"|---|---|---|---|---|---|"
foreach ($row in $rows) {
    "| $($row.page) | $(if ($row.gpu) { 'on' } else { 'off' }) | $($row.engine) | $($row.processes) | $(& $mb $row.private) | $(& $mb $row.working) |"
}
""
"| Page | GPU | Erk / Electron, private | Erk / Electron, working set | Erk / Chrome, private |"
"|---|---|---|---|---|"
$worst = 0.0
foreach ($page in $pages) {
    foreach ($gpu in @($true, $false)) {
        $at = { param($engine) $rows | Where-Object { $_.page -eq $page.name -and $_.gpu -eq $gpu -and $_.engine -eq $engine } }
        # Not $erk, $electron, $chrome: PowerShell's names ignore case, and
        # those are the [string] parameters.
        $erkRow = & $at "Erk"; $electronRow = & $at "Electron"; $chromeRow = & $at "Chrome"
        if (-not ($erkRow.private -and $electronRow.private -and $chromeRow.private)) {
            "| $($page.name) | $(if ($gpu) { 'on' } else { 'off' }) | not measured | | |"
            $worst = [double]::PositiveInfinity
            continue
        }
        $private = $erkRow.private / $electronRow.private
        $working = $erkRow.working / $electronRow.working
        $worst = [Math]::Max($worst, [Math]::Max($private, $working))
        "| $($page.name) | $(if ($gpu) { 'on' } else { 'off' }) | {0:P0} | {1:P0} | {2:P0} |" -f $private, $working, ($erkRow.private / $chromeRow.private)
    }
}
""
"Worst case: Erk uses {0:P0} of Electron's memory; the bar is 50%: {1}." -f $worst, $(if ($worst -le 0.5) { "met" } else { "not met" })
