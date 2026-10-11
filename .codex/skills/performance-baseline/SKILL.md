---
name: performance-baseline
description: Measure and retain Agofra performance baselines, centred on what a user can actually perceive — cold start, tray restore, resident memory and idle CPU, install size — plus Criterion Rust hot paths and production frontend builds. Use for recurring performance tests, regressions, baseline comparisons, and preserving measured results; do not use for implementing optimizations or general code review.
---

# Agofra 性能基线

用本 Skill 做可重复的测量与对比；不在此过程中修改业务代码。

## 产出标准：用户可感知

**只有用户在界面上能直接感知的，才算本 Skill 的产出。** 内部指标（函数耗时、包体字节）只有在能解释某个可感知现象时才有意义 —— 否则不作结论。

对照下表，**能测的必须测**，测不了的必须写明「缺什么才能测」，不能默默跳过。

| # | 维度 | 用户场景 | 终点定义 | 采集方式 | 状态 |
|---|---|---|---|---|---|
| 1 | **冷启动** | 双击图标从零打开 | 主窗口可见且进程可响应 | `scripts/perf-probe.ps1` | 自动 |
| 2 | **托盘恢复** | 关掉窗口后再点图标 | 主窗口重新可见 | 同上 | 自动 |
| 3 | **安装体积** | 下载、占盘 | 安装目录 / 安装包 MB | 同上 | 自动 |
| 4 | **常驻资源** | 托盘挂机数小时~数日 | 私有内存与句柄是否单调增长；空闲 CPU 占用 | 同上（`-SoakSeconds`） | 自动 |
| 5 | **首屏内容就绪** | 窗口出现后多久能看到供应商列表 | 列表可读（非骨架屏） | **需埋点或 WebView2 自动化** | 缺 |
| 6 | **页面切换** | 点侧边栏 5 个入口 | 目标视图可交互 | 同上 | 缺 |
| 7 | **等待类操作** | 应用配置 / 测试连通 / 获取模型 / 市场加载 | spinner 消失 | 同上 | 缺 |

维度 5–7 属**已知未覆盖**，不得用 `web-mock` 或 dev 构建的数据冒充；每次出报告都要在「尚未覆盖」里重新点名。
**连续缺失要升级**：同一维度连续 3 轮仍为「缺」，在报告里明确写出「需要投入 X（埋点/自动化）才能测」，
不要让它靠「写明缺失」无限期挂账。维度 6（页面切换）目前已连续 5 份基线零数据，属用户核心诉求，优先补。

另有一条观测盲区：`[sync.harvest]` 等逐轮埋点是 `debug!`，而 release 按 `lib.rs` 只出 Info，
**release 探针跑完在日志里查不到逐轮同步记录**。要评估「启动路径上后台工作的开销」（它就花在首屏里），
需先把相关埋点提到 `info!`；在此之前只能靠 `warn!` 级的预算告警间接判断，并在报告里写明这一限制。

## 流程

1. 记录 `git rev-parse --short HEAD`、`git status --short`、`node --version`、`pnpm --version`、`cargo --version` 与 `rustc --version`。
2. **确认没有同实例在跑**（App 注册了 single-instance，已有实例时再启动走「聚焦已有窗口」路径，不是冷启动）。不得自行结束用户的实例。
   ⚠️ **测量全程提醒用户不要手动启动 App**：探针每轮冷启动前需要「完全关闭」，会强杀**与被测 exe 同路径**的进程
   （其他路径的同名进程不受影响，例如用户开着的已安装版不会被牵连）。这一点必须在开跑前告知用户。
3. 确认被测产物是 `tauri build` 的产品（见「可测产物」）。探针开跑前会自动核验产物并在不通过时直接中止。
   脚本路径相对**仓库根**（不是 skill 目录）。`-Runs` 至少给到 **20**：`-Runs 3` 时中位数取到的是第 2 个样本，
   没有统计意义；20 轮还能看出离群轮（本轮实测首轮因 Defender 扫描新二进制而明显偏慢）。

```powershell
pwsh -File scripts/perf-probe.ps1 -Exe "<安装目录>\agofra.exe" -Runs 20 -SoakSeconds 600
```

4. 若本轮涉及 Rust 或前端改动，追加支撑层测量：

```powershell
cargo bench --bench config_bench --manifest-path src-tauri/Cargo.toml -- --save-baseline "perf-$(Get-Date -Format 'yyyy-MM-dd-HHmm')"
```

```powershell
pnpm build
```

5. 新建 `reports/baselines/<stamp>.md` 写本次结果（探针输出 + 支撑层数据 + 环境 + 可比性说明），并在 `reports/history.md` 追加一行索引。**不要覆盖旧记录。**
6. 比较只限相同基准名、同一命令和可比环境。差异写成「回归 / 改善 / 环境噪声」，不同机器、Mock 或调试构建不得混为一谈。**环境无法对齐时，宁可写「不可归因」。**

## 输出

- 一份带环境、命令、原始指标和限制条件的日期报告。
- `reports/history.md` 的可追溯索引与上一次可比记录的结论。
- Criterion HTML 仅作本机细节查看；长期比较以 `reports/` 的已跟踪 Markdown 为准。

## 可测产物（关键）

判据只有一条：**必须由 `tauri build` 产出，且前端资源已嵌入**。安装目录里的 exe、或同一次
`pnpm tauri build`（含 `--no-bundle`）产出的 `src-tauri/target/release/agofra.exe` 都算数。

**不要拿 `cargo build` / `cargo bench` 直接编出的 `target/release/*.exe` 测**：它们不走 tauri 打包流程、
前端资源未嵌入，实测**主窗口永不显示** —— 会被误读成「窗口显示回归」。探针会在开跑前扫 exe 里有没有
`assets/index-<hash>.js|css` 来自动核验，不通过直接中止。

两个别用错的判据：

- ❌ 看 `bundle/` 存不存在 —— `cargo bench` 和 `tauri build --no-bundle` **都不产生** `bundle/`，区分不了。
- ❌ 在二进制里找 `custom-protocol` 字面量 —— 那是 Cargo feature 名不是嵌入字符串，永远查不到。

执行顺序也有讲究：`cargo bench` 会刷新 `target/release/agofra.exe`，**必须排在 `tauri build` 之前**，
否则被测产物会被换成没嵌前端的那个。

## 边界

- 不自动执行 `pnpm tauri build`（耗时且会产出安装包；需要绑定 HEAD 的产物时，先向用户要一次构建产物或取得明确同意）。用户已明确授权构建时不受此限。
- 不自动停止正在运行的 App —— 包括用户的 dev 会话。
- 不因 Vite 的包体警告直接改代码；先拿到可比的真实测量。
- 不把 `web-mock`、`dev-web` 或调试构建的时间称为 Release 原生数据。

详见 [测量约定](references/measurement-contract.md)、[历史索引](reports/history.md)、[输出风险](reports/output-risk-profile.md) 与 [触发用例](evals/trigger_cases.json)。
