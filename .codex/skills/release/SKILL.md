---
name: release
description: Agmux 发版流水线（**默认仅本地 commit 为止**）：AI 读 CHANGELOG 历史与 git 三态（最新 tag / HEAD VERSION / working tree VERSION），CHANGELOG 草稿以**实际代码 diff**（`git diff <上一tag>..HEAD` 逐文件阅读）为唯一依据、commit 信息不可信，**单次弹窗合并确认** bump 级别 + CHANGELOG 草稿（一次 AskUserQuestion 问完）→ 确认后**一次性执行**：跑 `node scripts/bump-version.mjs <level>`（**禁止手写**）→ 写入 CHANGELOG + 本地 commit。**写入本地 commit 即终止**；AI 不主动询问、不主动执行任何 push / 盯构建 / 发布动作。Release 工作流**只手动触发**：push main / push tag 都不自动构建；dispatch 传 `release_mode` 三选一——draft（默认）构建完停在草稿、公开发布仍须用户确认，prerelease=构建完自动预发行，latest=构建完自动正式发行并通知关注者。当用户说"发版"、"发行"、"release"、"发个新版本"、"发布新版本"时使用。
---

# Agmux 发版

分工：本 skill 做需要判断的部分——bump 级别建议、CHANGELOG 整段内容草稿（不是只改版本标题）、本地 commit、跑 bump-version 命令；`.github/workflows/release.yml` 做确定性的部分——校验（含 CHANGELOG 第一段版本号 == VERSION 的 fail-fast 校验）、三平台构建、创建 tag 与发行页（发行说明 = CHANGELOG 第一段原文照抄）、上传资产。**工作流不回写 main**；CHANGELOG 无归档步骤。草稿不会通知关注者；执行发布那一刻 GitHub 才给关注者发通知邮件。

工作流**只手动触发**（`workflow_dispatch`），无任何自动触发器：push main / push tag 都不构建——发版是显式决策。触发时传 `release_mode` 三选一：`draft`（默认）构建完停在**草稿**，等人工发布；`prerelease` 构建完自动预发行（不占 latest 指针）；`latest` 构建完自动正式发行并通知关注者。每次 dispatch 只产生**一条** run（headBranch=main）。工作流用 `GITHUB_TOKEN` 代建 tag，不会递归触发自身。

## 默认流程边界

**Step 0–1 是默认范围**：收集信息 + 单次弹窗合并确认（bump 级别与 CHANGELOG 草稿一次问完） + 一次性执行（bump → 写 CHANGELOG → commit）。**Step 1 写入本地 commit 即终止。**

Step 2–4（push / 盯构建 / 发布）属于扩展流程，**必须用户明确启动**才执行，常见触发词如"继续"、"push"、"推上去"、"触发构建"、"发布"、"发版（确认发布）"。**用户不说就停手**——AI 在 Step 1 之后只汇报 commit 结果，**不主动询问也不主动执行**任何扩展动作。

**角色分工硬约束**：

- **AI**：给 bump 级别建议 + 起草 CHANGELOG；用户合并确认后跑 `node scripts/bump-version.mjs <level>`、写入 CHANGELOG、commit。**不**直接编辑 `VERSION` / `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`。
- **CHANGELOG 段落标题 = `## [<新版本号>] - <起草日>`**：版本号用 bump 后的 A.B.C（必须与 VERSION 一致——工作流照抄发行说明前会校验，不一致 fail-fast 拒绝构建）；日期写起草当天。若 push/dispatch 推迟且在意日期准确性，dispatch 前顺手改这一行。
- **用户**：在单次弹窗里合并确认 bump 级别（可改 AI 建议）与 CHANGELOG 文案（可改）。
- **只问一次**：bump 级别定了之后 CHANGELOG 标题里的版本号随之确定（两问仍互不阻塞），必须在**同一次** AskUserQuestion 调用里作为两个问题一起确认；确认后一口气执行完毕，中途不再询问。仅当用户对某一项给出修改意见时，才**只对该项**重新弹窗确认（另一项沿用已确认值，不重复问）。
- **确认必须弹窗（硬约束）**：所有需要用户拍板的选择（Step 0 的版本号与 CHANGELOG、Step 4 发布确认），AI 在消息里展示完整详情后，必须用 **AskUserQuestion 弹窗**列出可选项让用户**点选**，禁止只发文本等自由回复。推荐选项放首位并标注（Recommended）；完整详情（状态框、草稿全文、资产清单）仍先在弹窗前的消息里展示，弹窗选项的 description 放关键取舍信息。
- **发版 commit 只许发版文件（硬约束）**：见 Step 1 第 4 步——只 add 6 个发版文件，工作区其他改动一律不进本 commit。
- **递增必须跑脚本命令，禁手写**：`bump-version.mjs` 内已串联 `sync-version.mjs` 同步全部元数据文件，手写会漏。
- **版本号与日志基线 = GitHub 最新已发布 release（非 Draft）**：用 `gh release list --exclude-drafts --limit 1` 取其 tagName 作为唯一基线。分支测试构建的 Draft release 与带后缀的 tag（如 `v0.17.5-codex-3-plugin-optimize.7`）**不算发行**，不作为基线；该基线之后的全部用户可见变更都计入本次日志（即使部分内容曾在分支草稿构建中出现过）。版本号三态（基线 tag 解析出的版本号 + HEAD VERSION（`git show HEAD:VERSION`）+ working tree VERSION（读 `VERSION`））取**最大值**作为 bump 基线——用户在测试场景可能预先 bump，working tree 会领先 index / HEAD。
- **CHANGELOG 只认代码 diff，不认 commit 信息（硬约束）**：commit 标题/信息可能挂错（历史案例：标题写「优化发版流程描述」的提交实际改了 16 个代码文件）。起草草稿与进/不进清单必须以实际代码改动为唯一依据——先 `git diff <上一tag>..HEAD --stat` 总览，再逐个**代码文件**读完整 diff；commit 列表仅用于确定范围与计数，禁止作为内容依据。

本 skill 不限定分支：在任何分支触发都只执行本地流程（bump + CHANGELOG + commit）。分支切换、push、工作流触发、盯构建、发布由用户自行决定，AI 不主动执行也不主动询问。

## Instructions

### Step 0: 收集信息 + 单次合并确认（**等弹窗点选**，不修改任何文件）

1. 取基线 tag = GitHub 最新已发布 release（非 Draft）：`gh release list --exclude-drafts --limit 1` 读其 tagName。分支测试草稿与带后缀的 tag 不算发行，不作基线（见上方硬约束）。
2. **git 三态确认基线**（取 max）：
   - HEAD VERSION：`git show HEAD:VERSION`
   - working tree VERSION：读 `VERSION`
   - 最新 tag 解析出的版本号
   - 三者取 max 作为 bump 基线
3. 拉自上一 tag 起的 commit 列表：`git log <上一tag>..HEAD --oneline --no-merges`（首个版本用全部历史）——仅用于确定范围与计数。**CHANGELOG 内容禁止依据 commit 标题/信息**：必须逐文件阅读 `git diff <上一tag>..HEAD` 的实际代码改动（先 `--stat` 总览，再逐个代码文件读完整 diff，见上方硬约束）。
4. 拉 `CHANGELOG.md` 最近 5–8 段已发布段落作为"项目自有的 minor / patch / major 量级参照"，并检查顶部段落是否为「幽灵版本」（标题版本号无 tag、无 GitHub Release——上次准备好但没发出去的残留，或历史遗留的 `## [Unreleased]`）；若是，其内容并入本次新段落一起发布，不单独建没有 tag 的版本段，并在展示里说明并入方案。
5. **单次展示**（一条消息里同时给出）：

   ┌──────────────────────────────────────────────────────────────┐
   │ 当前状态（git 三态）：                                       │
   │   最新 tag：v0.7.1                                           │
   │   HEAD VERSION：0.7.2                                        │
   │   working tree VERSION：0.7.3（领先 HEAD 是测试残留）        │
   │   bump 基线（取 max）：0.7.3                                 │
   │   自上一 tag 的 commit 数：N                                 │
   │                                                              │
   │ CHANGELOG 草稿（标题 ## [版本] - 起草日，全文）：             │
   │   ……（含归档/并入方案说明）                                  │
   │                                                              │
   │ 进 / 不进清单（无用户可见影响的提交不进）：                  │
   │   ……                                                        │
   │                                                              │
   │ 建议 bump 级别：patch / minor / major                        │
   │ 依据：参照旧段落量级 + 当前 commit 列表的关键特征            │
   └──────────────────────────────────────────────────────────────┘

6. **单次弹窗合并确认**：一次 AskUserQuestion 调用带**两个问题**，用户一次点选完：
   - 问题 1「版本号」：bump 级别选项（如「沿用基线 X.Y.Z（Recommended）」「patch → X.Y.Z+1」「minor」「major」），推荐项放首位标注（Recommended），每项 description 写关键取舍
   - 问题 2「CHANGELOG」：「确认写入（Recommended）」/「需要修改」（修改意见走 Other 自由输入或下一轮消息）
7. 两项都点选后才进入 Step 1；若某项需要修改，按意见修订后**只对该项**重新弹窗确认，另一项沿用已确认值，不重复问。
8. **硬约束**：本步骤**禁止**修改任何文件、**禁止**跑 `bump-version.mjs`。

CHANGELOG 写作规则：

- 用用户视角描述变更（"新增 xxx 功能"），不要照抄 commit 标题。
- 按实际代码差异重新组织本次版本的完整正文；替换未发布段落时，旧正文只作为待核对清单，不原样沿用，不只替换标题。用户明确要求仅改标题时除外。
- 先按模块列出用户可见变更，再核对新增、修复和交互细节，避免用“完善／优化”概括掉独立变化；大 diff 分文件分批读取，输出截断必须补读，未读完不得声称已完整核对。
- 默认按以下顺序分区，空分区整节省略，不为凑标题拆分同一变更：
  - **更名与迁移**：产品名、仓库入口、数据目录或备份命名的变化，写清旧数据如何处理。
  - **新增**：新增的用户功能与使用入口。
  - **修复**：原先的具体问题，以及修复后的行为。
  - **界面与交互**：布局、操作方式与反馈变化。
  - **性能优化**：仅记录有实际测量依据的改善，不从代码变化推断提速。
  - **移除**：用户需要知道的功能移除；涉及安全的用户可见变化可单列 **安全**。
- **不进入 CHANGELOG 的改动**（从 diff 判断，非 commit 标题；无用户可见影响）：
  - 纯版本号 bump（`chore(release): vX.Y.Z`）
  - 纯 CI / 工作流变更
  - 纯文档类提交
  - 纯内部重构与普通依赖升级；影响用户功能、兼容性或安全时，归入对应分区说明实际影响
  - 纯文案 / 按钮 / 标签 / 提示语等措辞小改（不影响功能）
  - 纯图标 / 品牌资源替换（不影响功能）；涉及产品名称、入口或数据迁移时必须记录，不适用此排除项
- 沿用历史段落的 Markdown 格式，不沿用其过时分区或未经代码核对的结论。

CHANGELOG 段落模板（分区按实际变更从可用分区里取，有几段写几段，不是固定三段）：

```markdown
## [A.B.C] - YYYY-MM-DD

### <分区名>
- …
```

（A.B.C = 本次 bump 后的版本号，必须与 VERSION 一致；日期 = 起草当天）

> **不要**在 CHANGELOG 里写「如何选择安装包」安装指南（含 macOS 首次打开提示）：这段由 `.github/workflows/release.yml` 的 release job 自动追加到发行页（安装指南兜底，必带），写进 CHANGELOG 会与工作流产出重复。旧版本段落里出现的这段是历史遗留，新版本不沿用。Full Changelog 对比链接由工作流自动生成（即使没写日志，发行页也有变更入口）；发行说明就是本段落原文照抄，标题行即版本标题。

### Step 1: 一次性执行（**确认后 AI 连续完成，中途不再询问**）

1. AI 按 Step 0 用户点选的级别跑：

   ```bash
   node scripts/bump-version.mjs patch   # 或 minor / major（若用户选沿用基线则跳过）
   ```

2. 检查脚本输出"版本号已从 X.Y.Z 更新为 A.B.C"，记下 A.B.C 作为本版本号；lockfile 未跟上的话 AI 跑：`cargo update -p agmux --manifest-path src-tauri/Cargo.toml`（lockfile 已对齐可跳过）。
3. 写入 CHANGELOG：
   - **幽灵版本并入**：若顶部已有段落且是「幽灵版本」（标题版本号无 tag、无 GitHub Release——上次准备好但没发出去的残留）或历史遗留的 `## [Unreleased]`，其内容并入本次新段落一起发布，**不**保留没有 tag 的版本段；顶部段落已正常发行则直接进行下一步。
   - 在既有段落上方插入 Step 0 已确认的新版本段落，标题写 `## [<A.B.C>] - <今天日期>`（A.B.C = bump 后版本号，必须与 VERSION 一致，工作流会校验）。
4. 提交发版文件（**硬约束：只许这 6 个文件，多一个都不行**）：
   `git add VERSION package.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/tauri.conf.json CHANGELOG.md`
   提交信息：`chore(release): v<版本>`
   - **禁止** `git add -A` / `git add .` / 任何路径通配或目录级 add；工作区其他一切改动（代码、测试、i18n、样式、新文件等）一律不得进入本 commit，保持原样留在工作区。
   - add 后先 `git status --short` 核对暂存区只含上述 6 个文件；发现多余文件必须 `git restore --staged <文件>` 摘掉后再提交。
5. **到此停下**：汇报版本号、commit hash、CHANGELOG 段落摘要，**会话停在此处**。**不询问用户是否继续**（避免被读成对扩展动作的暗示），**不主动执行**任何 push / 触发工作流 / 发布操作。Step 2–4 需用户用明确指令单独启动。

### Step 2: 推送并手动触发构建 — 需用户明确启动才执行

> 默认流程到 Step 1 为止。本节起必须用户明确指示（如"继续"、"push"、"推上去"、"发出去"）才执行，不要自行越界。

1. 推送代码：`git push origin main`。Release 工作流**不会**因 push 自动触发，无自动构建。
2. 按用户意图手动 dispatch（三选一，发版模式就是这里的显式决策）：
   - **默认（无干预，先看草稿）**：`gh workflow run release.yml --ref main`（默认 `release_mode=draft`）→ 构建完停在草稿，走 Step 4 人工发布（asset 清单 → AskUserQuestion 点选正式/预发行/暂不）；
   - **干预直接正式发行**：`gh workflow run release.yml --ref main -f release_mode=latest` → 构建完自动正式发行并通知关注者，无 Step 4；
   - **干预直接预发行**：`gh workflow run release.yml --ref main -f release_mode=prerelease` → 构建完自动预发行（不占 latest 指针），无 Step 4。
3. dispatch 前确认工作区已推齐（`git status` 干净、`git log origin/main..HEAD` 为空）——构建跑的是远端 main 最新 commit，本地未推的 commit 不参与。
4. 等 10 秒后取 run：`gh run list --workflow=Release --limit 1 --json databaseId,status,headBranch`——只有一条 run（headBranch=main），盯它即可。
5. 推送前可选本地预检 `pnpm check`（与工作流 verify job 同一条链），失败就地修复并补充提交；⚠️ 项目 node_modules 是 Windows 平台构建的，必须在 **Windows 侧**执行（WSL 里跑会触发 corepack 重建依赖、破坏 Windows 开发环境）；跳过也可，工作流 verify 会兜底。

### Step 3: 盯 Release 工作流 — 需用户明确启动才执行

1. 只有一条 run（Step 2 的 dispatch，headBranch=main）：`gh run watch <run-id> --exit-status --interval 30` 放后台执行（约 30-40 分钟），完成时会收到通知。
2. 构建失败：`gh run view <run-id> --log-failed` 提取报错摘要，报告用户并停止（草稿若已创建则留在草稿态，不影响关注者）。
3. 构建成功后工作流已自动完成：创建 tag 与发行页（版本标题 + Full Changelog 链接 + 发行日志 + 安装指南；发行说明 = CHANGELOG 第一段原文照抄）、上传三平台资产。draft 模式停在草稿等人工发布（Step 4）；prerelease / latest 模式此刻已自动发布，无 Step 4。

### Step 4: 确认与发布 — 需用户明确启动才执行

> 本步仅适用于**默认草稿路径**（`release_mode=draft`）。prerelease / latest 模式构建完成即自动公开，跳过本步。

1. 展示给用户（这一步必须等用户明确确认，不得自动发布）：
   - `gh release view v<版本> --json name,isDraft,assets` 的资产清单（文件名 + 大小）
   - 发行日志全文预览
2. 必须用 AskUserQuestion 弹窗让用户点选：「正式发布（--latest）」/「预发布（--prerelease）」/「暂不发布」；点选后才执行对应命令（正式发布 `gh release edit v<版本> --draft=false --latest`）。
3. 变体处理：
   - 用户点选「预发布」：加 `--prerelease`，去掉 `--latest`
   - 用户要改日志：改 `CHANGELOG.md` 对应版本段落，提交推送后工作流自动重跑，用新段落更新既有草稿后再发布
4. 发布后告知用户：关注者通知已发出，附 release 页面链接 `https://github.com/zeno528/Agmux/releases/tag/v<版本>`

## 示例

**场景 A（默认流程）**：用户说"发版"

1. **Step 0**：AI 看 git 三态（最新 tag `v0.7.1` / HEAD VERSION `0.7.2` / working tree VERSION `0.7.3`，取 max = `0.7.3`） + `git log v0.7.1..HEAD` + `CHANGELOG.md` 最近段落 → 单条消息展示状态框 + "建议 patch（依据：参照 v0.7.1 类似量级，单 `fix:` commit）" + CHANGELOG 草稿 + 进/不进 → **一次 AskUserQuestion 弹窗**（问题 1 版本号级别、问题 2 CHANGELOG 确认）→ 用户两项点选确认
2. **Step 1**：AI 一次跑完：`node scripts/bump-version.mjs patch` → 写入 `CHANGELOG.md` → `git commit -m "chore(release): v0.7.4"` → 汇报版本号、commit hash、CHANGELOG 段落摘要，**会话终止**。不询问、不执行 push / 触发工作流 / 发布；这些动作必须等用户在下一轮显式启动（例如"推上去"、"继续"）。

**场景 B（扩展流程 · 干预直接发行，需用户明确指示）**：用户在场景 A 之后说"直接发出去"

1. **Step 2**：`git push origin main` 后 `gh workflow run release.yml --ref main -f release_mode=latest` → 一条 run
2. **Step 3**：盯该 run 至全绿——构建完成即自动正式发行并通知关注者，无 Step 4
3. 想先出预发行：`-f release_mode=prerelease`，不占 latest 指针

**场景 B2（扩展流程 · 默认草稿路径，需用户明确指示）**：用户说"推上去，先看草稿"

1. **Step 2**：`git push origin main` 后 `gh workflow run release.yml --ref main`（默认 draft）→ 一条 run，构建完停在草稿
2. **Step 3**：后台 `gh run watch` 盯至全绿（工作流自动建 tag、草稿并上传资产）
3. **Step 4**：展示资产（Windows setup/msi、macOS x64/arm64 dmg）+ 日志全文 → AskUserQuestion 弹窗点选「正式发布」 → `gh release edit v0.7.4 --draft=false --latest`，报告链接

## Troubleshooting

**push 后没有构建**：设计如此——工作流只手动触发，push 永不自动构建。要出包必须 dispatch：`gh workflow run release.yml --ref main`（默认草稿）或带 `-f release_mode=<mode>`；队列用 `gh run list --workflow=Release` 查。

**verify 绿色跳过（重复 dispatch，`::notice` 提示）**：属正常——该版本（VERSION 对应 tag）已公开发布，重复 dispatch 不重发（改 CHANGELOG 文案不会重出发行页）。

**verify 失败（VERSION 为空）**：说明版本号没提交。跑 `node scripts/bump-version.mjs <level>` 后把 VERSION 等发版文件一起提交推送，再重新 dispatch。

**工作流报「CHANGELOG.md 第一段版本号与 VERSION 不一致」**：起草后 VERSION 又被改动（如补 bump），或段落标题手滑。把 CHANGELOG 顶部段落标题改成与 VERSION 一致（或按需重跑 bump），commit push 后重新 dispatch。

**草稿已存在（重跑场景）**：工作流会检测到草稿并更新（`gh release edit`），不会重复建。

**发行已公开后工作流又跑**：工作流会拒绝修改已发布内容（"发行 vX 已经公开，拒绝修改已发布内容"），属正常保护，不是 bug；需要发新版本时递增 VERSION 重来。
