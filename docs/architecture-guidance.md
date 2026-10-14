# Agmux 架构与目录演进指导

> 本文用于后续开发决策。当前结论基于 Agmux 现有 Tauri 2 + React 19/Vite + Rust + SQLite 实现，以及未来适配 Codex CLI / WSL2 的规划。

## 当前结论

- SQLite 是当前本地桌面应用的合适方案，不更换 PostgreSQL、MySQL 或其他数据库。
- `rusqlite` 的 `bundled`、WAL 和 `rusqlite_migration` 保持不变。
- 现有源码目录和用户数据目录不需要为了“看起来更规整”而整体重构。
- 当前主要支持 Codex Desktop / Windows，单目标模型暂时可以继续使用。

## 用户数据目录

```text
~/.agmux/
├── settings.json
├── agmux.db
├── agmux.db-wal
├── agmux.db-shm
└── backups/
    ├── config/
    ├── codex-files/
    └── database/
```

### 目录职责

- `settings.json`：应用级设置。
- `agmux.db`：供应商配置数据、账号和切换事件。
- `agmux.db-wal`、`agmux.db-shm`：SQLite WAL 运行文件，应用运行时不得手动删除。
- `backups/config/`：自动备份 `config.toml`。
- `backups/codex-files/`：自动备份 `auth.json`、`models.json`、catalog 等 Codex 配套文件。
- `backups/database/`：用户主动导出的数据库快照，用于列表、恢复、删除和重命名。

这三个备份目录对应不同的备份对象和保留策略，应保持分离，不要为了减少目录数量而合并。

## 源码目录约定

- `src/app/`：React AppShell、主题、窗口生命周期和全局反馈。
- `src/features/profiles/`、`src/features/mcp/`、`src/features/settings/`：按业务边界组织页面与编辑流程。
- `src/components/`：跨 Feature 共享的控件、编辑器和图标。
- `src-tauri/src/auth/`：认证逻辑。
- `src-tauri/src/codex/`：Codex 配置解析和进程操作。
- `src-tauri/src/database.rs`：SQLite schema、迁移和数据访问。
- `src-tauri/src/services/`：当前应用服务编排；在只有一个 Codex 目标时不提前拆分目标适配层。

### 前端数据边界

- `src/api/index.ts` 保持 Tauri Command 与 Web 调试 mock 的兼容接口。
- `AppShell` 持有唯一根 `AppState`；MCP、备份列表和编辑详情只在所属 Feature 内持有局部状态。
- 不为当前应用引入 QueryClient、路由系统、每条 Command 的 Action 或未来插件市场空接口。

以下目录属于开发或构建产物，不属于软件源码结构：

- `src-tauri/target/`
- `dist/`
- `node_modules/`
- `.playwright-mcp/`
- 临时截图和搜索结果图片

这些内容不得进入发布包或 Git。`src-tauri/target/` 已由 Rust 工具链生成并忽略，磁盘紧张时可以清理构建缓存。

## WSL2 适配原则

未来接入 Codex CLI / WSL2 时：

1. Agmux 的 SQLite 数据库仍保存在 Windows 用户目录的 `~/.agmux/`。
2. 不要让 Windows 程序通过 `\\wsl$` 直接高频读写 SQLite 数据库。
3. WSL2 下的 Codex 配置、认证文件和进程操作应通过 WSL 目标适配器执行。
4. 只有在真正开始实现第二个目标时，才新增：

```text
src-tauri/src/targets/
├── codex_desktop.rs
└── codex_wsl2.rs
```

数据库后续只需增加目标维度，例如 `targets`、目标级当前 profile 状态，以及带 `target_id` 的切换事件；现有 `profiles.payload_json` 不需要拆解或废弃。

账号身份和目标凭据应逐步分离。不要默认把 Windows Desktop 的 token 复制到 WSL2；凭据应按目标保存，并在 Windows 侧考虑使用系统级凭据保护。

## 禁止的过度设计

- 不要现在提前创建 WSL2、Linux 或其他 Agent 的空目录。
- 不要为了目录数量减少而合并三类备份。
- 不要因为未来可能接入其他 Agent 就更换 SQLite。
- 不要在只有一个实现时提前创建复杂的工厂、插件系统或多层抽象。
- 新目标出现后，优先增加一个目标适配器和最小数据库迁移；不要重写现有 profile 数据模型。

## 参考实现位置

- `src-tauri/src/paths.rs`：用户数据和备份路径。
- `src-tauri/src/fsutil.rs`：原子写入、备份和清理策略。
- `src-tauri/src/database.rs`：SQLite schema、迁移和备份恢复。
- `src-tauri/src/services/`：配置应用、认证文件和 Codex 进程操作等服务编排。
