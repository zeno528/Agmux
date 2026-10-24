---
name: local-build
description: 用户要求本地构建 Agmux 安装包时使用，按 macOS 或 Windows 分支构建并核实产物；不用于普通前端构建。
---

# 本地快速编译

在本机编译**当前工作区状态**（包括未提交改动）的安装包，不触发 GitHub 发行。工具链沿用根 `AGENTS.md`；当前 Mac 的 `pnpm …` 命令通过 `./work/dev-env/run.sh …` 执行。

## Step 0: 环境检测

先按当前系统与工具 shell 选择分支；macOS 可运行下列命令，Windows 不为检测系统而启动 WSL：

```bash
uname -s
```

| 输出 | 系统 | 执行 |
|:---|:---|:---|
| Windows 上下文（或 Git Bash 的 `MINGW*` / `MSYS*` / `CYGWIN*`） | Windows | 继续 Step 1 → Step 2W |
| `Darwin` | macOS | 继续 Step 1 → Step 2M |
| 其他（`Linux` 等） | 不在覆盖范围 | 停下报告"本项目本地构建只覆盖 Windows / macOS"，不构建 |

## Step 1: 快速预检（共用）

运行 `pnpm typecheck`（约 10 秒）。失败就直接停下报告错误，不要进入几分钟的编译；用户修完类型错误再重来。

Rust 侧不用预检——`pnpm tauri build` 本身会编译，有错会当场报。

## Step 2W: Windows 分支

### 图片预检（NSIS 用）

如果 `src-tauri/icons/installer-header.bmp` 或 `installer-sidebar.bmp` 存在，先确认它们是 NSIS 可读的经典 BMP：`BITMAPINFOHEADER`（DIB 大小 40）、24-bit、`BI_RGB` 无压缩。不要把 ImageMagick 默认生成的 BMP V4/V5 直接交给 NSIS：

```powershell
foreach ($bmp in @('src-tauri/icons/installer-header.bmp', 'src-tauri/icons/installer-sidebar.bmp')) {
  if (-not (Test-Path -LiteralPath $bmp)) { continue }
  $bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $bmp))
  $dib = [BitConverter]::ToUInt32($bytes, 14)
  $bpp = [BitConverter]::ToUInt16($bytes, 28)
  $compression = [BitConverter]::ToUInt32($bytes, 30)
  if ($bytes.Length -lt 54 -or $bytes[0] -ne 0x42 -or $bytes[1] -ne 0x4d -or $dib -ne 40 -or $bpp -ne 24 -or $compression -ne 0) {
    throw "NSIS installer image is not classic 24-bit BI_RGB BMP: $bmp"
  }
}
```

NSIS 3.11 Modern UI 的官方推荐尺寸是 header `150×57`、welcome/finish sidebar `164×314`。这些是 96-DPI 下的逻辑控件尺寸，不是高 DPI 的 2x 资源规格；Agmux 为 163-DPI/高 DPI 场景交付 `300×114`、`328×628` 的矢量重采样 BMP，保持相同宽高比。默认 `MUI_*_BITMAP_STRETCH` 为 `FitControl`，高 DPI 或 CJK 字体会让控件变大并触发运行时放大。用户反馈发糊时，先区分"源图低分辨率"和"安装器运行时缩放"，不要仅靠换 BMP 编码判断画质。

### 构建

**产物模式由用户在调用时指定**，未指定就走默认：

- **默认**：NSIS exe + MSI 双产物，`pnpm tauri build`（与 Release 工作流一致）
- **快速模式**：用户明确说"只要一个 exe / 不用 msi"时，`pnpm tauri build --bundles nsis`——CLI 覆盖只打 NSIS 一个包，省去 WiX/MSI 打包（约快 20-60 秒）。不要为此改 `tauri.conf.json` 的 `targets: "all"`，那是发行工作流出双资产用的

用 PowerShell 工具在项目根目录执行，timeout 设 600000（10 分钟）：

```powershell
$buildLog = Join-Path $env:TEMP "agmux-tauri-build-$PID.log"
pnpm tauri build 2>&1 | Tee-Object -FilePath $buildLog   # 快速模式改为: pnpm tauri build --bundles nsis
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
if (Select-String -LiteralPath $buildLog -Pattern 'Unsupported format|warning 5040' -Quiet) {
  throw "NSIS rejected an installer image; inspect $buildLog and fix the BMP format before distributing the package."
}
```

- 成功标志：默认输出 `Finished 2 bundles at:`（快速模式 `Finished 1 bundle at:`）并列出路径
- 该命令自动完成：sync-version（VERSION → 三处版本号同步）→ vite build → cargo release 编译 → NSIS（+ MSI）打包
- `Finished N bundles at:` 不是唯一成功标准；NSIS `Unsupported format`/`warning 5040` 即使不阻断构建，也必须视为失败

### 确认产物并打开目录

```powershell
Get-ChildItem "src-tauri/target/release/bundle/nsis/*.exe" | Select-Object Name, @{n='SizeMB';e={[math]::Round($_.Length/1MB,1)}}, LastWriteTime
if (Test-Path "src-tauri/target/release/bundle/msi") { Get-ChildItem "src-tauri/target/release/bundle/msi/*.msi" | Select-Object Name, @{n='SizeMB';e={[math]::Round($_.Length/1MB,1)}}, LastWriteTime }
Invoke-Item "src-tauri/target/release/bundle/nsis"
```

### 按此格式报告

| 产物 | 路径 | 说明 |
|:---|:---|:---|
| NSIS 安装包（推荐） | `src-tauri\target\release\bundle\nsis\Agmux_<版本>_x64-setup.exe` | 标准安装体验 |
| MSI 安装包（仅默认构建） | `src-tauri\target\release\bundle\msi\Agmux_<版本>_x64_en-US.msi` | 备选 |
| 绿色版 | `src-tauri\target\release\agmux.exe` | 免安装直接运行 |

## Step 2M: macOS 分支

### 构建

**产物模式由用户在调用时指定**，未指定就走默认：

- **默认**：`.app` + DMG 双产物，`pnpm tauri build`
- **指定模式**：用户明确说"只要 DMG"时，`pnpm tauri build --bundles dmg`——CLI 覆盖只出 DMG。不要为此改 `tauri.conf.json` 的 `targets: "all"`，那是发行工作流用的

用 Bash 工具在项目根目录执行：

```bash
pnpm tauri build   # 只要 DMG 时改为: pnpm tauri build --bundles dmg
```

- timeout 设 600000（10 分钟）；首次冷编译 10-20 分钟属正常
- 成功标志：默认输出 `Finished 2 bundles at:`（只要 DMG 时 `Finished 1 bundle at:`）并列出路径
- 自动完成：sync-version → vite build → cargo release 编译 → `.app`（+ DMG）打包
- 只产当前机器架构的包（Apple Silicon 出 `aarch64`，Intel 出 `x86_64`），不产 universal 包

### 确认产物并打开目录

```bash
ls -lh src-tauri/target/release/bundle/dmg/*.dmg
open "src-tauri/target/release/bundle/dmg"
```

### 按此格式报告

| 产物 | 路径 | 说明 |
|:---|:---|:---|
| DMG 安装包（推荐） | `src-tauri/target/release/bundle/dmg/Agmux_<版本>_<架构>.dmg` | 拖入 Applications |
| APP 直接运行 | `src-tauri/target/release/bundle/macos/Agmux.app` | 免安装直接运行 |
| 裸二进制 | `src-tauri/target/release/agmux` | 排查用 |

## 注意事项

共用：

- 版本号取自根目录 `VERSION` 文件；要测新版本号就先改 `VERSION` 再跑本 skill
- 产物文件名固定带版本号，重复构建会覆盖同名旧文件；安装新版本直接覆盖旧版本，无需先卸载
- 产物均未签名

Windows 专属：

- 未签名包触发 SmartScreen 提示——点"仍要运行"
- 本次改动含 `src-tauri/icons` 时默认直接增量构建，不得自动运行 `cargo clean` 或删除旧产物；如 EXE 仍嵌入旧图标，先保留/备份现有安装包，再向用户说明并获确认后做定向清理
- 图标回归检查：`256x256.png` 的白色容器应接近 `254×254`（约 `+1+1`），且 32/48 层 `icon.ico[0]`、`icon.ico[3]` 应分别为完整 `32×32`、`48×48`；不能只检查透明边界而漏掉桌面/资源管理器使用的实际容器尺寸
- 画质排查依据：[NSIS Modern UI 2 文档](https://nsis.sourceforge.io/Docs/Modern%20UI%202/Readme.html)；如需改变高 DPI 缩放行为，必须用 Tauri 的自定义 NSIS 模板，不要把未验证的 `NoStretch` 选项直接写进生成脚本

macOS 专属：

- Gatekeeper 拦截未签名 app——首次打开右键"打开"，或 `xattr -cr "/Applications/Agmux.app"`
- DMG 架构须与装机架构一致（`uname -m` 核对），跨架构包能装但跑不起来或走 Rosetta

## 常见问题

**构建失败，报 Rust 编译错误**
看错误第一行的文件和行号，多为当前代码问题；修完重新执行构建步骤即可，增量编译很快。

**`pnpm` / `cargo` 命令找不到**
先核对根 `AGENTS.md` 的项目环境与启动器，不擅自全局安装或改 PATH；确认依赖缺失后再按项目环境安装。

**第一次构建很慢（10-20 分钟）**
正常——冷编译约 600 个 crate。之后 `src-tauri/target` 是热的，增量构建通常 1 分钟内。

**typecheck 过了但 tauri build 在 vite 阶段失败**
看 vite 报错的具体文件；`pnpm build` 可以单独复现该阶段。

**macOS 构建在 bundling 阶段报 icon.icns 相关错误**
确认 `src-tauri/icons/icon.icns` 存在且有效（`bundle.icon` 列表引用了它）；缺失时用 `pnpm tauri icon` 从源图重新生成。
