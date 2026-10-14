# 测量约定

## 可比性

- Rust 微基准固定使用 `src-tauri/benches/config_bench.rs`，其临时目录不会触碰真实 `~/.agmux` 或 `~/.codex`。
- 每次保存新的 Criterion 标签，读取本次 `new/estimates.json`；旧标签只用于 Criterion 图表内部比较。
- 记录 Git 提交、工作区是否干净、Node/pnpm/Cargo/Rust 版本。硬件、供电模式或防病毒负载不同则标为环境变化。

## 体验测量

- `release-native` 冷启动：先确认没有同实例运行（App 注册了 single-instance，已有实例时再启动会走「聚焦已有窗口」路径，测到的不是冷启动，也不能由 AI 自行结束用户的实例）；计时终点必须是主窗口可见且进程可响应；记录至少三次。仅进程存在或端口监听不算启动完成。
- `release-native` 托盘恢复：关闭主窗口（`minimize_to_tray` 打开时应隐藏而非退出）→ 再次启动 exe 走 single-instance 聚焦 → 计到主窗口重新可见。这是托盘常驻应用最高频的用户动作，且比冷启动快得多（不重建 WebView2），两者必须分开记。
- 常驻资源：托盘挂机场景采样私有内存、句柄数与空闲 CPU。判读看**单调增长趋势**，不看单点绝对值；单次几十秒的采样不足以判断泄漏，至少十分钟起步。
- `dev-web`：可用于页面加载和交互趋势，但不能代替 Release 桌面数据。直接双击 `target/debug/agmux.exe` 不可测：dev 构建从 `localhost:5173` 取前端，Vite dev server 未运行时窗口永不显示。
- `web-mock`：可验证前端状态流和相对变化，不代表 IPC、文件系统扫描或外部 Codex CLI 的耗时。

### 判定「窗口可见」的正确方式

用 Win32 `EnumWindows` 遍历该 PID 的顶层窗口，取 `IsWindowVisible` 为真且标题等于 `Agmux` 的那个。

**不要用 .NET 的 `Process.MainWindowHandle`**：它返回进程第一个可见顶层窗口，在本项目里会命中 single-instance 插件的窗口（标题 `com.zeno528.agmux-siw`），得到 45–87 ms 的假终点（真值约 650–760 ms）。`MainWindowTitle` 同时为空是它的典型症状。

### 如何正确读取启动分段

启动埋点有**两条原点不同的时钟**，混用会稳定算出错误的数：

| 时钟 | 字段 | 原点 |
|---|---|---|
| Rust | `elapsed_ms`（native 侧）、`rust_elapsed_ms`（前端上报时回传） | 进程起点 |
| 前端 | `frontend_elapsed_ms` | **WebView 文档起点** |

实测前端钟比 Rust 钟晚约 290 ms 起算，所以：

- ✅ 「文档导航完成 → 首屏就绪」的真实耗时用 **Rust 钟**：`state_ready.rust_elapsed_ms − page_load_finished.elapsed_ms`
- ❌ 不要写 `frontend_elapsed_ms − page_load_finished`：会得到**负数**（两个原点不同，相减无意义）
- 报分段时同一里程碑的**两条钟读数要成对给出**，否则读者无法判断原点

其他读法约定：

- **筛完整冷启动**：single-instance 的短命二进程（托盘恢复时新起的那个）只打 `native_ready`
  就在插件 init 处退出、**没有 `setup_end`**。按「含 `setup_end`」筛，否则会把托盘段的
  短命进程混进冷启动统计（`native_ready > 400 ms` 可作辅助过滤）。
- **`window_shown` 常被探针的强杀吃掉**：窗口可见 → 判可响应 → `Stop-Process` 的竞态会让 show 之后的
  上报来不及落盘。`window_pre_show` 存活率才是可靠的；出窗时间可用 `state_ready` + rAF 等待推算。
- **release 下 `debug!` 不落盘**：按 `lib.rs` 的 dev=Debug / release=Info 配置，
  `debug!` 级的逐轮埋点在 release 构建里查不到。涉及 `debug!` 埋点的结论，要么把该埋点提到
  `info!`，要么在报告里写明「release 下不可观测」——不要用 dev 会话的日志冒充 release 数据。

### 可测的 Release 产物

判据只有一条：**必须由 `tauri build` 产出，且前端资源已嵌入**。

两种路径都算数：

- 安装目录里的 exe（`%LOCALAPPDATA%\Agmux\agmux.exe`）
- 同一次 `pnpm tauri build`（含 `--no-bundle`）产出的 `src-tauri/target/release/agmux.exe`

**不要拿 `cargo bench` / `cargo build --release` 编出来的 `target/release/*.exe` 测**：它们会顺带编译包内
bin target，但不走 tauri 打包流程，前端资源未嵌入，主窗口永不显示 —— 这会被误读成「窗口显示回归」。

**怎么判**：唯一可靠的判据是扫 exe 二进制里有没有嵌入的 `assets/index-<hash>.js|css`。
`scripts/perf-probe.ps1` 开跑前会自动核验并在不通过时**直接中止**（而不是让你拿着一列 null 去解读）。

两个常见的错误判据：

- ❌ 看 `bundle/` 存不存在 —— `cargo bench` 和 `tauri build --no-bundle` **都不产生** `bundle/`，无法区分。
- ❌ 在二进制里找 `custom-protocol` 字面量 —— 那是 Cargo feature 名，不是嵌入的字符串，永远查不到。

## 判读

- Criterion 的 95% 区间跨过零或仅出现极小差异时，写「噪声范围内」。
- `settings_write` 方差较大时，按 Criterion 提示增加测量时间后再判定回归。
- Vite 的 `>500 kB` 只提示拆包机会；需要结合真实首屏或交互数据后才能优化。
