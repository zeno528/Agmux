<#
用户可感知性能探针（Windows）。

只测「用户在界面上能直接感知」的维度，不测内部实现细节：
  1. 冷启动      —— App 完全关闭 → 双击图标 → 主窗口可见且可响应
  2. 托盘恢复    —— 关闭到托盘 → 再点图标 → 主窗口重新可见
  3. 安装体积    —— 用户要下载/占用的磁盘
  4. 常驻资源    —— 托盘挂机时内存/句柄是否单调增长、空闲 CPU 是否吃满（需 -SoakSeconds > 0）

判定「窗口可见」用 EnumWindows + IsWindowVisible + 标题精确匹配：
不能用 .NET 的 Process.MainWindowHandle —— 它会命中 single-instance 插件的窗口
（标题 com.zeno528.agmux-siw），得到几十毫秒的假终点。

用法（路径相对**仓库根**，不是本文件所在目录）：
  pwsh -File scripts/perf-probe.ps1 -Exe "C:\Users\<you>\AppData\Local\Agmux\agmux.exe"
  pwsh -File scripts/perf-probe.ps1 -Exe <exe> -Runs 20 -SoakSeconds 600

⚠️ 测量期间不要手动启动 Agmux：每轮冷启动前需要「完全关闭」，脚本会强杀**与被测 exe 同路径**
的进程来保证这一点。手测时开着的实例会被一并结束（本脚本不碰其他路径的同名进程）。

前置：被测 exe 必须是 tauri build 的产物，且**前端资源已嵌入**（下面 Test-TauriArtifact 会核验）。
cargo build/bench 直接编出的 target/release/*.exe 前端未嵌入，主窗口永不显示——
不拦住的话整轮会得到一列 null，被误读成「窗口显示回归」。
#>
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [string]$Title = "Agmux",
  [int]$Runs = 3,
  [int]$SoakSeconds = 0,
  [int]$SoakIntervalSeconds = 30,
  [string]$InstallDir = ""
)

if (-not (Test-Path $Exe)) { throw "找不到可执行文件：$Exe" }

# 产物核验：唯一的可靠判据是「前端资源有没有嵌进 exe」。
# tauri build 会把 dist/assets/index-<hash>.js 打进二进制，cargo build/bench 直编的不会。
# （`bundle/` 是否存在不能当判据：cargo bench 与 `tauri build --no-bundle` 都不产生它。）
# （也别去二进制里找 `custom-protocol` 字面量——那是 Cargo feature 名，不是嵌入的字符串。）
# 注意：本函数不返回布尔值——消息走输出流、失败走异常。若返回 $true/$false，
# 调用处的 `if` 会把提示语一起捕获吞掉。
function Assert-TauriArtifact([string]$Path) {
  $text = [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($Path))
  $hit = [regex]::Match($text, 'assets/index-[A-Za-z0-9_\-]+\.(js|css)')
  if ($hit.Success) {
    Write-Output "产物核验 ✓：exe 内嵌前端资源（命中 $($hit.Value)）"
    return
  }
  throw "产物核验失败：exe 内找不到 assets/index-*.js|css —— 多半是 cargo build/bench 直编的空壳。`n  请改用 'pnpm tauri build'（可加 --no-bundle）产出后再测；否则主窗口永不显示，会伪装成「窗口显示回归」。"
}
Assert-TauriArtifact $Exe

Add-Type @"
using System;using System.Text;using System.Runtime.InteropServices;
public class PerfWindow {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder t, int m);
  public static bool MainVisible(uint targetPid, string title) {
    bool found = false;
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid == targetPid && IsWindowVisible(h)) {
        var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
        if (sb.ToString() == title) { found = true; return false; }
      }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@

# 每轮冷启动需要「完全关闭」才能测到真正的冷启动。这里只结束**与被测 exe 同路径**的进程：
# 用户可能同时开着另一个构建的 Agmux（例如已安装版），那不是本轮要重启的那个，不该被牵连。
# 读不到 Path 的进程一律跳过（宁可漏杀也不要误杀用户实例）；探针自己 Start-Process 起的实例
# 路径必然可读，所以本轮自己的残留一定能被清掉。
function EnsureNoInstance {
  $target = (Resolve-Path $Exe).Path
  $name = [IO.Path]::GetFileNameWithoutExtension($Exe)
  Get-Process -Name $name -ErrorAction SilentlyContinue | ForEach-Object {
    $path = $null
    try { $path = $_.Path } catch { $path = $null }
    if (-not $path) { return }
    $resolved = Resolve-Path $path -ErrorAction SilentlyContinue
    if ($resolved -and $resolved.Path -eq $target) {
      Write-Output "  （结束同路径实例 PID=$($_.Id)，以保证从零冷启动）"
      Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
  }
}

# 注意不能用 [int]($n/2)：PowerShell 的 [int] 是银行家舍入，3/2 会得到 2，取到最大值而非中位数。
function Get-Median([double[]]$Values) {
  $v = @($Values | Where-Object { $_ -ne $null } | Sort-Object)
  if ($v.Count -eq 0) { return $null }
  $mid = [math]::Floor($v.Count / 2)
  if ($v.Count % 2 -eq 1) { return $v[$mid] }
  return [math]::Round(($v[$mid - 1] + $v[$mid]) / 2)
}

function Wait-Visible([int]$ProcessId, [int]$TimeoutSeconds = 30) {
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  while ($sw.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    if ([PerfWindow]::MainVisible([uint32]$ProcessId, $Title)) { $sw.Stop(); return [math]::Round($sw.Elapsed.TotalMilliseconds) }
    Start-Sleep -Milliseconds 10
  }
  return $null
}

$result = [ordered]@{}

# ── 1. 冷启动：每个样本前彻底关闭实例 ────────────────────────────
$cold = @()
for ($i = 1; $i -le $Runs; $i++) {
  EnsureNoInstance
  Start-Sleep -Milliseconds 2500
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $Exe -PassThru
  $visible = Wait-Visible $p.Id
  $responsive = $null
  if ($visible) {
    while ($sw.Elapsed.TotalSeconds -lt 30) { $p.Refresh(); if ($p.Responding) { $responsive = [math]::Round($sw.Elapsed.TotalMilliseconds); break }; Start-Sleep -Milliseconds 10 }
  }
  $sw.Stop()
  $cold += [pscustomobject]@{ Run = $i; Visible_ms = $visible; Responsive_ms = $responsive }
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
}
$result["冷启动"] = $cold

# ── 2. 托盘恢复：关闭窗口（minimize_to_tray）后再点图标 ──────────
EnsureNoInstance
Start-Sleep -Milliseconds 2500
$p = Start-Process -FilePath $Exe -PassThru
[void](Wait-Visible $p.Id)
$tray = @()
for ($i = 1; $i -le $Runs; $i++) {
  $p.Refresh()
  [void]$p.CloseMainWindow()
  Start-Sleep -Milliseconds 1200
  $hiddenOk = (-not [PerfWindow]::MainVisible([uint32]$p.Id, $Title)) -and (-not $p.HasExited)
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  Start-Process -FilePath $Exe | Out-Null   # 走 single-instance，把已有窗口拉回前台
  $visible = Wait-Visible $p.Id
  $sw.Stop()
  $tray += [pscustomobject]@{ Run = $i; HiddenBefore = $hiddenOk; Restore_ms = $visible }
  Start-Sleep -Milliseconds 1500
}
$result["托盘恢复"] = $tray
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue

# ── 3. 体积：用户下载的安装包 + 安装后占盘 ──────────────────────
# 不能量 exe 所在目录 —— 对 target/release 那类构建目录会把全部构建产物算进去。
$exeDir = Split-Path -Parent $Exe
$bundleDir = Join-Path $exeDir "bundle"
$exeTime = (Get-Item $Exe).LastWriteTime
# bundle/ 会累积历届安装包。只取「最新」一个仍不够：--no-bundle 构建根本不产出新包，
# 于是会回退到几个月前的陈旧安装包，体积数字会被当成本轮数据引用。
# 因此额外要求 installer 的时间戳晚于被测 exe，否则显式标注「本轮未产出」并把体积留空。
$installer = $null
$installerNote = "（本轮未产出安装包）"
if (Test-Path $bundleDir) {
  $newest = Get-ChildItem $bundleDir -Recurse -File -Include *.exe, *.msi -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
  if ($newest -and $newest.LastWriteTime -gt $exeTime) {
    $installer = $newest
    $installerNote = $newest.Name
  } elseif ($newest) {
    $installerNote = "（本轮未产出：bundle/ 里最新的是 $($newest.Name)，早于被测 exe，不作为本轮数据）"
  }
}
$installMb = $null
if ($InstallDir -and (Test-Path $InstallDir)) {
  $installMb = [math]::Round(((Get-ChildItem $InstallDir -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum / 1MB), 1)
}
$result["体积"] = [pscustomobject]@{
  Exe_MB       = [math]::Round((Get-Item $Exe).Length / 1MB, 1)
  Installer_MB = if ($installer) { [math]::Round($installer.Length / 1MB, 1) } else { $null }
  Installer    = if ($installer) { $installer.Name } else { $installerNote }
  Installed_MB = $installMb
}

# ── 4. 常驻资源（托盘挂机）──────────────────────────────────────
if ($SoakSeconds -gt 0) {
  EnsureNoInstance
  Start-Sleep -Milliseconds 2500
  $p = Start-Process -FilePath $Exe -PassThru
  [void](Wait-Visible $p.Id)
  $p.Refresh()
  $cpu0 = $p.TotalProcessorTime.TotalSeconds
  $soak = @()
  $elapsed = 0
  while ($elapsed -lt $SoakSeconds) {
    Start-Sleep -Seconds $SoakIntervalSeconds
    $elapsed += $SoakIntervalSeconds
    $p.Refresh()
    $cpu = $p.TotalProcessorTime.TotalSeconds - $cpu0
    $soak += [pscustomobject]@{
      At_s       = $elapsed
      WS_MB      = [math]::Round($p.WorkingSet64 / 1MB, 1)
      Private_MB = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
      Handles    = $p.HandleCount
      CPU_pct    = [math]::Round(100 * $cpu / $elapsed, 2)
    }
    $cpu0 = $p.TotalProcessorTime.TotalSeconds
  }
  $result["常驻资源"] = $soak
  Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
}

# ── 输出 ────────────────────────────────────────────────────────
""
"## 冷启动（App 全关 → 主窗口可见）"
$result["冷启动"] | Format-Table -AutoSize
$v = @($result["冷启动"] | Where-Object { $_.Visible_ms } | ForEach-Object { $_.Visible_ms })
if ($v.Count) { "中位数：{0} ms（区间 {1}–{2}）" -f (Get-Median $v), ($v | Measure-Object -Minimum).Minimum, ($v | Measure-Object -Maximum).Maximum }

""
"## 托盘恢复（关闭到托盘 → 再点图标 → 窗口可见）"
$result["托盘恢复"] | Format-Table -AutoSize
$t = @($result["托盘恢复"] | Where-Object { $_.Restore_ms } | ForEach-Object { $_.Restore_ms })
if ($t.Count) { "中位数：{0} ms（区间 {1}–{2}）" -f (Get-Median $t), ($t | Measure-Object -Minimum).Minimum, ($t | Measure-Object -Maximum).Maximum }

""
"## 体积"
$result["体积"] | Format-List

if ($SoakSeconds -gt 0) {
  ""
  "## 常驻资源（{0} 秒采样）" -f $SoakSeconds
  $result["常驻资源"] | Format-Table -AutoSize
  $first = $result["常驻资源"] | Select-Object -First 1
  $last = $result["常驻资源"] | Select-Object -Last 1
  "内存变化：{0} → {1} MB（私有）" -f $first.Private_MB, $last.Private_MB
  "句柄变化：{0} → {1}" -f $first.Handles, $last.Handles
}
