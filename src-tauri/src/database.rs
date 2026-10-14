use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row, Transaction};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};

use crate::error::{app_err, AppResult};
use crate::models::{
    ClaudeProfileInput, CodexProfileKind, CodexProfilePayload, CodexProfileSummary,
};
use crate::paths::AppPaths;
use crate::services::codex_profile_config::{parse_provider_detail, stored_provider_api_key};
use crate::services::SkillTool;

/// MCP 镜像行：Codex TOML 与 Claude JSON 各自独立，卸载不等同于禁用。
pub struct McpServerRecord {
    pub name: String,
    pub toml: String,
    pub codex_enabled: bool,
    pub claude_enabled: bool,
    pub codex_installed: bool,
    pub claude_installed: bool,
    pub claude_json: Option<String>,
}

/// profile id 进程内自增后缀：同一毫秒内创建多个供应商也不会撞唯一约束。
static PROFILE_ID_SEQ: AtomicU64 = AtomicU64::new(0);

const SCHEMA_V1: &str = r#"
CREATE TABLE app_state (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  active_profile_id TEXT,
  default_account_id TEXT
);

CREATE TABLE accounts (
  id TEXT PRIMARY KEY,
  email TEXT,
  id_token TEXT,
  refresh_token TEXT NOT NULL,
  authenticated_at INTEGER NOT NULL
);

CREATE TABLE profiles (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  icon TEXT,
  kind TEXT NOT NULL,
  account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE switch_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  profile_id TEXT,
  action TEXT NOT NULL,
  status TEXT NOT NULL,
  message TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(profile_id) REFERENCES profiles(id) ON DELETE SET NULL
);

CREATE INDEX idx_switch_events_created_at ON switch_events(created_at DESC);
CREATE INDEX idx_switch_events_profile_id ON switch_events(profile_id);
"#;

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(SCHEMA_V1),
        M::up("ALTER TABLE profiles ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0"),
        // 缓存上次生成的 auth.json 原文：切换 ChatGPT 配置时离线复用，不触发 token 刷新
        M::up("ALTER TABLE accounts ADD COLUMN auth_json TEXT"),
        // MCP 服务器片段镜像（live config.toml 为操作事实源，DB 存一份随备份/恢复携带）
        M::up(
            "CREATE TABLE mcp_servers (
               name TEXT PRIMARY KEY,
               toml TEXT NOT NULL,
               sort_order INTEGER NOT NULL DEFAULT 0,
               updated_at TEXT NOT NULL
             )",
        ),
        // ChatGPT 多账号共存：身份三分（本地主键 / workspace / 用户 sub）。
        // 存量行 id 本就是唯一 workspace，保留原 id 只回填两列，避免重建表把 profiles.account_id 外键置空。
        M::up_with_hook(
            "ALTER TABLE accounts ADD COLUMN chatgpt_account_id TEXT;
             ALTER TABLE accounts ADD COLUMN user_identity TEXT",
            |tx| {
                backfill_account_identity(tx)?;
                Ok(())
            },
        ),
        // 同一 workspace 同一用户唯一：并发登录完成时由索引兜底，后到者转更新
        M::up(
            "CREATE UNIQUE INDEX accounts_ws_user_uq
             ON accounts(chatgpt_account_id, user_identity)
             WHERE user_identity IS NOT NULL",
        ),
        // 订阅套餐徽标：从持久化的 id_token 现场回填存量行
        M::up_with_hook("ALTER TABLE accounts ADD COLUMN plan_type TEXT", |tx| {
            backfill_account_plan_type(tx)?;
            Ok(())
        }),
        // Claude Code 支持第一期：独立供应商表与激活位（与 Codex profiles 完全并行，互不触碰）
        M::up(
            "CREATE TABLE claude_profiles (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               base_url TEXT,
               auth_token TEXT,
               model TEXT,
               created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             ALTER TABLE app_state ADD COLUMN active_claude_profile_id TEXT",
        ),
        // 编辑页端点档反查与官网按钮：记录创建时选的预设 kind 与控制台地址（对齐 Codex payload.builtin）
        M::up(
            "ALTER TABLE claude_profiles ADD COLUMN kind TEXT;
             ALTER TABLE claude_profiles ADD COLUMN admin_url TEXT",
        ),
        // 编辑页描述与模型列表持久化（对齐 Codex description / codex_set_profile_fetched_models）
        M::up(
            "ALTER TABLE claude_profiles ADD COLUMN description TEXT;
             ALTER TABLE claude_profiles ADD COLUMN fetched_models TEXT",
        ),
        // 编辑页 settings.json 编辑器：托管键之外的附加 env（JSON 对象文本，应用时并入 env）
        M::up("ALTER TABLE claude_profiles ADD COLUMN extra_env TEXT"),
        // 编辑页可编辑 logo（对齐 Codex profiles.icon）
        M::up("ALTER TABLE claude_profiles ADD COLUMN icon TEXT"),
        // 卡片拖拽排序（对齐 Codex profiles.sort_order）
        M::up("ALTER TABLE claude_profiles ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0"),
        M::up("ALTER TABLE claude_profiles ADD COLUMN raw_settings TEXT"),
        // MCP 开关第一版（开发期中间态，已被下一迁移替换；槽位内容对已升级的存量库已固化，不可改）
        M::up("ALTER TABLE mcp_servers ADD COLUMN global_enabled INTEGER NOT NULL DEFAULT 1"),
        // MCP 开关按引擎独立：Codex/Claude 各自只控制自己用户范围 live 文件里的条目，
        // 互不影响（对齐 Skills 的每工具独立开关）；中间态列随此移除。
        M::up(
            "ALTER TABLE mcp_servers ADD COLUMN codex_enabled INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE mcp_servers ADD COLUMN claude_enabled INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE mcp_servers DROP COLUMN global_enabled",
        ),
        // Claude 供应商用量条开关。迁移只能追加在末尾：插队会推后后续迁移的序号，
        // 让已应用过它们的存量库重放旧迁移（duplicate column 崩溃）。
        M::up("ALTER TABLE claude_profiles ADD COLUMN show_balance INTEGER NOT NULL DEFAULT 0"),
        M::up("ALTER TABLE mcp_servers ADD COLUMN claude_json TEXT"),
        M::up("ALTER TABLE app_state ADD COLUMN claude_common_settings TEXT"),
        M::up(
            "ALTER TABLE mcp_servers ADD COLUMN codex_installed INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE mcp_servers ADD COLUMN claude_installed INTEGER NOT NULL DEFAULT 1",
        ),
        // Claude 卡片终端入口的最近工作目录（全局一份）；历史迁移只能在末尾追加。
        M::up("ALTER TABLE app_state ADD COLUMN claude_terminal_dirs TEXT"),
        M::up(
            "CREATE TABLE chatgpt_model_cache (
               cache_key TEXT PRIMARY KEY,
               models_json TEXT NOT NULL,
               updated_at TEXT NOT NULL
             )",
        ),
    ])
}

/// 回填账号订阅套餐：从持久化的 id_token 解析 chatgpt_plan_type。
/// 迁移 hook 与旧 schema 备份恢复共用；只补空值，幂等。
pub(super) fn backfill_account_plan_type(tx: &Transaction) -> rusqlite::Result<()> {
    let mut statement = tx.prepare(
        "SELECT id, id_token FROM accounts
         WHERE plan_type IS NULL AND id_token IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let updates: Vec<(String, Option<String>)> = rows
        .map(|row| {
            let (id, id_token) = row?;
            Ok((id, crate::auth::codex_oauth::extract_plan_type(&id_token)))
        })
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for (id, plan_type) in updates {
        if let Some(plan_type) = plan_type {
            tx.execute(
                "UPDATE accounts SET plan_type = ?2 WHERE id = ?1",
                params![id, plan_type],
            )?;
        }
    }
    Ok(())
}

/// 回填账号身份两列：workspace 取存量行 id，用户 sub 从持久化的 id_token 现场重算。
/// 迁移 hook 与旧 schema 备份恢复共用。
pub(super) fn backfill_account_identity(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE accounts SET chatgpt_account_id = id WHERE chatgpt_account_id IS NULL",
        [],
    )?;
    let mut statement = tx.prepare(
        "SELECT id, id_token FROM accounts
         WHERE user_identity IS NULL AND id_token IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let updates: Vec<(String, Option<String>)> = rows
        .map(|row| {
            let (id, id_token) = row?;
            Ok((
                id,
                crate::auth::codex_oauth::extract_user_identity(&id_token),
            ))
        })
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for (id, identity) in updates {
        if let Some(identity) = identity {
            tx.execute(
                "UPDATE accounts SET user_identity = ?2 WHERE id = ?1",
                params![id, identity],
            )?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub struct Database {
    connection: Mutex<Connection>,
}

impl Database {
    pub fn open(paths: &AppPaths) -> AppResult<Self> {
        paths.ensure()?;
        let mut connection = Connection::open(&paths.database)
            .map_err(|error| app_err!("无法打开数据库 {}: {error}", paths.database.display()))?;

        connection
            .pragma_update(None, "journal_mode", "WAL")
            .and_then(|_| connection.pragma_update(None, "foreign_keys", "ON"))
            .and_then(|_| connection.pragma_update(None, "synchronous", "NORMAL"))
            .map_err(|error| app_err!("无法初始化 SQLite: {error}"))?;

        migrations()
            .to_latest(&mut connection)
            .map_err(|error| app_err!("数据库迁移失败: {error}"))?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Codex 供应商表访问方法以 `codex_` 显式归属；SQL 仍读写历史 `profiles` 表与
    /// `app_state.active_profile_id` 列（schema 改名属持久化迁移，不在命名重构范围）。
    /// 这层名称映射只住在这里，页面/服务一律用 `codex_*` 方法名。
    pub fn codex_profiles(&self) -> AppResult<Vec<StoredCodexProfile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, payload_json, icon, kind, account_id, created_at, updated_at
                 FROM profiles ORDER BY sort_order ASC, created_at ASC, id ASC",
            )
            .map_err(|error| app_err!("无法读取供应商配置: {error}"))?;
        let rows = statement
            .query_map([], profile_from_row)
            .map_err(|error| app_err!("无法读取供应商配置: {error}"))?;

        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(row.map_err(|error| app_err!("供应商配置数据无效: {error}"))?);
        }
        Ok(profiles)
    }

    pub fn codex_reorder_profiles(&self, ids: &[String], timestamp: &str) -> AppResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始排序事务: {error}"))?;
        for (index, id) in ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE profiles SET sort_order = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, index as i64, timestamp],
                )
                .map_err(|error| app_err!("保存排序失败: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| app_err!("保存排序失败: {error}"))?;
        Ok(())
    }

    pub fn codex_profile(&self, id: &str) -> AppResult<StoredCodexProfile> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT id, name, payload_json, icon, kind, account_id, created_at, updated_at
                 FROM profiles WHERE id = ?1",
                params![id],
                profile_from_row,
            )
            .optional()
            .map_err(|error| app_err!("无法读取供应商配置: {error}"))?
            .ok_or_else(|| app_err!("供应商配置不存在"))
    }

    pub fn codex_insert_profile(
        &self,
        name: &str,
        payload: &CodexProfilePayload,
        timestamp: &str,
    ) -> AppResult<CodexProfileSummary> {
        let id = format!(
            "profile-{timestamp}-{}",
            PROFILE_ID_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let payload_json =
            serde_json::to_string(payload).map_err(|_| app_err!("供应商配置序列化失败"))?;
        let kind = if payload.provider_id.is_none() {
            CodexProfileKind::Official
        } else {
            CodexProfileKind::ThirdParty
        };
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO profiles(id, name, payload_json, kind, created_at, updated_at, sort_order)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?5,
                        (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM profiles))",
                params![id, name, payload_json, kind.as_db(), timestamp],
            )
            .map_err(|error| app_err!("无法保存供应商配置: {error}"))?;
        Ok(summary(
            &id, name, payload, None, None, timestamp, timestamp,
        ))
    }

    pub fn codex_set_profile_icon(
        &self,
        id: &str,
        icon: Option<&str>,
        timestamp: &str,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE profiles SET icon=?2, updated_at=?3 WHERE id=?1",
                params![id, icon, timestamp],
            )
            .map_err(|error| app_err!("无法更新供应商配置图标: {error}"))?;
        if changed == 0 {
            return Err(app_err!("供应商配置不存在"));
        }
        Ok(())
    }

    pub fn codex_set_profile_account(
        &self,
        id: &str,
        account_id: Option<&str>,
        timestamp: &str,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE profiles SET account_id=?2, updated_at=?3 WHERE id=?1",
                params![id, account_id, timestamp],
            )
            .map_err(|error| app_err!("无法更新供应商配置: {error}"))?;
        if changed == 0 {
            return Err(app_err!("供应商配置不存在"));
        }
        Ok(())
    }

    pub fn codex_update_profile(
        &self,
        id: &str,
        name: &str,
        payload: &CodexProfilePayload,
        timestamp: &str,
    ) -> AppResult<StoredCodexProfile> {
        let payload_json =
            serde_json::to_string(payload).map_err(|_| app_err!("供应商配置序列化失败"))?;
        let connection = self.lock()?;
        connection
            .query_row(
                "UPDATE profiles SET name=?2, payload_json=?3, updated_at=?4 WHERE id=?1
                 RETURNING id, name, payload_json, icon, kind, account_id, created_at, updated_at",
                params![id, name, payload_json, timestamp],
                profile_from_row,
            )
            .optional()
            .map_err(|error| app_err!("无法更新供应商配置: {error}"))?
            .ok_or_else(|| app_err!("供应商配置不存在"))
    }

    pub fn rename_profile(
        &self,
        id: &str,
        name: &str,
        timestamp: &str,
        tool: SkillTool,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        let table = match tool {
            SkillTool::Codex => "profiles",
            SkillTool::Claude => "claude_profiles",
        };
        let changed = connection
            .execute(
                &format!("UPDATE {table} SET name=?2, updated_at=?3 WHERE id=?1"),
                params![id, name, timestamp],
            )
            .map_err(|error| app_err!("无法重命名供应商配置: {error}"))?;
        if changed == 0 {
            return Err(app_err!("供应商配置不存在"));
        }
        Ok(())
    }

    pub fn codex_delete_profile(&self, id: &str) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute("DELETE FROM profiles WHERE id=?1", params![id])
            .map_err(|error| app_err!("无法删除供应商配置: {error}"))?;
        if changed == 0 {
            return Err(app_err!("供应商配置不存在"));
        }
        Ok(())
    }

    pub fn accounts(&self) -> AppResult<Vec<StoredAccount>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, email, id_token, refresh_token, auth_json, authenticated_at,
                        chatgpt_account_id, user_identity, plan_type
                 FROM accounts ORDER BY authenticated_at DESC, id ASC",
            )
            .map_err(|error| app_err!("无法读取订阅账号: {error}"))?;
        let rows = statement
            .query_map([], account_from_row)
            .map_err(|error| app_err!("无法读取订阅账号: {error}"))?;

        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row.map_err(|error| app_err!("订阅账号数据无效: {error}"))?);
        }
        Ok(accounts)
    }

    pub fn chatgpt_models(
        &self,
        key: &str,
    ) -> AppResult<Option<Vec<crate::services::ChatgptModel>>> {
        let text: Option<String> = self
            .lock()?
            .query_row(
                "SELECT models_json FROM chatgpt_model_cache WHERE cache_key=?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| app_err!("读取账号模型缓存失败: {error}"))?;
        text.map(|text| serde_json::from_str(&text).map_err(|_| app_err!("账号模型缓存格式无效")))
            .transpose()
    }

    pub fn set_chatgpt_models(
        &self,
        key: &str,
        models: &[crate::services::ChatgptModel],
    ) -> AppResult<()> {
        if models.is_empty() {
            return Err(app_err!("ChatGPT 未返回可用模型，保留原有缓存"));
        }
        let text = serde_json::to_string(models).map_err(|_| app_err!("序列化账号模型缓存失败"))?;
        self.lock()?.execute(
            "INSERT INTO chatgpt_model_cache(cache_key, models_json, updated_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(cache_key) DO UPDATE SET models_json=excluded.models_json, updated_at=excluded.updated_at",
            params![key, text, crate::paths::now_ms().to_string()],
        ).map_err(|error| app_err!("保存账号模型缓存失败: {error}"))?;
        Ok(())
    }

    pub fn upsert_account(&self, account: &StoredAccount) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO accounts(id, email, id_token, refresh_token, auth_json, authenticated_at,
                                      chatgpt_account_id, user_identity, plan_type)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   email=excluded.email,
                   id_token=excluded.id_token,
                   refresh_token=excluded.refresh_token,
                   auth_json=excluded.auth_json,
                   authenticated_at=excluded.authenticated_at,
                   chatgpt_account_id=excluded.chatgpt_account_id,
                   user_identity=excluded.user_identity,
                   plan_type=COALESCE(excluded.plan_type, accounts.plan_type)",
                params![
                    account.id,
                    account.email,
                    account.id_token,
                    account.refresh_token,
                    account.auth_json,
                    account.authenticated_at,
                    account.chatgpt_account_id,
                    account.user_identity,
                    account.plan_type,
                ],
            )
            .map_err(|error| app_err!("无法保存订阅账号: {error}"))?;
        Ok(())
    }

    /// 仅当账号不存在时插入；命中唯一索引 accounts_ws_user_uq 返回 Ok(false)。
    /// 并发登录完成时以索引为兜底，调用方据 false 重查并转为更新已有行。
    pub fn insert_account_if_absent(&self, account: &StoredAccount) -> AppResult<bool> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO accounts(id, email, id_token, refresh_token, auth_json, authenticated_at,
                                      chatgpt_account_id, user_identity, plan_type)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    account.id,
                    account.email,
                    account.id_token,
                    account.refresh_token,
                    account.auth_json,
                    account.authenticated_at,
                    account.chatgpt_account_id,
                    account.user_identity,
                    account.plan_type,
                ],
            )
            .map(|changed| changed == 1)
            .or_else(|error| {
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                    Ok(false)
                } else {
                    Err(app_err!("无法保存订阅账号: {error}"))
                }
            })
    }

    pub fn delete_account(&self, id: &str) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute("DELETE FROM accounts WHERE id=?1", params![id])
            .map_err(|error| app_err!("无法删除订阅账号: {error}"))?;
        if changed == 0 {
            return Err(app_err!("订阅账号不存在"));
        }
        Ok(())
    }

    /// 读取 MCP 服务器片段（名称, TOML 片段）按 sort_order；表空返回空列表。
    pub fn mcp_server_fragments(&self) -> AppResult<Vec<(String, String)>> {
        Ok(self
            .mcp_server_records()?
            .into_iter()
            .filter(|record| record.codex_installed)
            .map(|record| (record.name, record.toml))
            .collect())
    }

    /// 读取 MCP 片段及每引擎开关，按 sort_order；表空返回空列表。
    pub fn mcp_server_records(&self) -> AppResult<Vec<McpServerRecord>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT name, toml, codex_enabled, claude_enabled, claude_json, codex_installed, claude_installed
                 FROM mcp_servers ORDER BY sort_order ASC, name ASC",
            )
            .map_err(|error| app_err!("无法读取 MCP 服务器: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(McpServerRecord {
                    name: row.get(0)?,
                    toml: row.get(1)?,
                    codex_enabled: row.get::<_, i64>(2)? != 0,
                    claude_enabled: row.get::<_, i64>(3)? != 0,
                    claude_json: row.get(4)?,
                    codex_installed: row.get::<_, i64>(5)? != 0,
                    claude_installed: row.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(|error| app_err!("无法读取 MCP 服务器: {error}"))?;
        rows.map(|row| row.map_err(|error| app_err!("MCP 服务器数据无效: {error}")))
            .collect()
    }

    pub fn mcp_server_record(&self, name: &str) -> AppResult<Option<McpServerRecord>> {
        Ok(self
            .mcp_server_records()?
            .into_iter()
            .find(|record| record.name == name))
    }

    pub fn set_codex_mcp_enabled(&self, name: &str, enabled: bool) -> AppResult<()> {
        self.update_mcp_enabled("codex_enabled", name, enabled)
    }

    pub fn set_claude_mcp_enabled(&self, name: &str, enabled: bool) -> AppResult<()> {
        self.update_mcp_enabled("claude_enabled", name, enabled)
    }

    /// column 只能来自本文件的两个常量调用点，不接收外部输入。
    fn update_mcp_enabled(&self, column: &str, name: &str, enabled: bool) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                &format!("UPDATE mcp_servers SET {column} = ?2 WHERE name = ?1"),
                params![name, enabled as i64],
            )
            .map_err(|error| app_err!("无法更新 MCP 开关: {error}"))?;
        if changed == 0 {
            return Err(app_err!("MCP 服务器不存在: {name}"));
        }
        Ok(())
    }

    pub fn codex_delete_mcp_server(&self, name: &str) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM mcp_servers WHERE name = ?1", params![name])
            .map_err(|error| app_err!("无法删除 MCP 服务器镜像: {error}"))?;
        Ok(())
    }

    /// 单条保存或改名：同一事务保留另一引擎开关与原生配置，不重建其他行。
    pub fn save_mcp_server_record(
        &self,
        original_name: Option<&str>,
        record: &McpServerRecord,
        timestamp: &str,
        tool: SkillTool,
    ) -> AppResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始 MCP 写入事务: {error}"))?;
        let old_name = original_name.unwrap_or(&record.name);
        transaction.execute(
            "INSERT INTO mcp_servers(name, toml, sort_order, updated_at, codex_enabled, claude_enabled, claude_json, codex_installed, claude_installed)
             VALUES(?1, ?2, COALESCE((SELECT sort_order FROM mcp_servers WHERE name=?7), (SELECT COALESCE(MAX(sort_order), -1)+1 FROM mcp_servers)), ?3, ?4, ?5, ?6, ?8, ?9)
             ON CONFLICT(name) DO UPDATE SET toml=excluded.toml, updated_at=excluded.updated_at,
             codex_enabled=excluded.codex_enabled, claude_enabled=excluded.claude_enabled, claude_json=excluded.claude_json,
             codex_installed=excluded.codex_installed, claude_installed=excluded.claude_installed",
            params![record.name, record.toml, timestamp, record.codex_enabled, record.claude_enabled, record.claude_json, old_name, record.codex_installed, record.claude_installed],
        ).map_err(|error| app_err!("无法保存 MCP 服务器: {error}"))?;
        if old_name != record.name {
            let columns = match tool {
                SkillTool::Codex => "codex_installed=0, codex_enabled=0",
                SkillTool::Claude => "claude_installed=0, claude_enabled=0",
            };
            transaction
                .execute(
                    &format!("UPDATE mcp_servers SET {columns} WHERE name=?1"),
                    params![old_name],
                )
                .map_err(|error| app_err!("无法更新旧 MCP 名称: {error}"))?;
            transaction
                .execute("DELETE FROM mcp_servers WHERE name=?1 AND codex_installed=0 AND claude_installed=0", params![old_name])
                .map_err(|error| app_err!("无法删除旧 MCP 名称: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| app_err!("无法提交 MCP 写入事务: {error}"))
    }

    /// 全量替换 Codex 活跃片段集：管理规模小，整表重写最简单。
    /// 已知行的另一端快照和状态原样保留，新增 Codex 条目不安装到 Claude。
    pub fn replace_mcp_server_fragments(
        &self,
        fragments: &[(String, String)],
        timestamp: &str,
    ) -> AppResult<()> {
        self.replace_mcp_server_fragments_with(fragments, timestamp, |_| Ok(()))
    }

    pub fn replace_mcp_server_fragments_with(
        &self,
        fragments: &[(String, String)],
        timestamp: &str,
        before_commit: impl FnOnce(&mut [McpServerRecord]) -> AppResult<()>,
    ) -> AppResult<()> {
        let previous = self.mcp_server_records()?;
        let mut rows: Vec<McpServerRecord> = fragments
            .iter()
            .map(|(name, toml)| {
                // 新集合里仍存在的行沿用旧开关（另一引擎的关闭状态不能被 Codex 侧保存冲掉）
                let existing = previous.iter().find(|record| &record.name == name);
                McpServerRecord {
                    name: name.clone(),
                    toml: toml.clone(),
                    codex_enabled: existing.is_none_or(|record| record.codex_enabled),
                    claude_enabled: existing.is_some_and(|record| record.claude_enabled),
                    claude_json: existing.and_then(|record| record.claude_json.clone()),
                    codex_installed: true,
                    claude_installed: existing.is_some_and(|record| record.claude_installed),
                }
            })
            .collect();
        for mut record in previous {
            if !rows.iter().any(|row| row.name == record.name) {
                if record.codex_enabled {
                    // 接受 Codex 现场删除，只撤销 Codex 安装；Claude 的安装和开关保留。
                    record.codex_installed = false;
                    record.codex_enabled = false;
                }
                rows.push(record);
            }
        }
        self.replace_mcp_server_records_with(rows, timestamp, before_commit)
    }

    pub fn replace_mcp_server_records_with(
        &self,
        mut rows: Vec<McpServerRecord>,
        timestamp: &str,
        before_commit: impl FnOnce(&mut [McpServerRecord]) -> AppResult<()>,
    ) -> AppResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始 MCP 写入事务: {error}"))?;
        transaction
            .execute("DELETE FROM mcp_servers", [])
            .map_err(|error| app_err!("无法清理 MCP 服务器: {error}"))?;
        // 回调不能重新进入数据库连接锁。
        before_commit(&mut rows)?;
        for (index, record) in rows.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO mcp_servers(name, toml, sort_order, updated_at, codex_enabled, claude_enabled, claude_json, codex_installed, claude_installed)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        record.name,
                        record.toml,
                        index as i64,
                        timestamp,
                        record.codex_enabled as i64,
                        record.claude_enabled as i64,
                        record.claude_json,
                        record.codex_installed,
                        record.claude_installed
                    ],
                )
                .map_err(|error| app_err!("无法保存 MCP 服务器: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| app_err!("无法提交 MCP 写入事务: {error}"))?;
        Ok(())
    }

    /// 读取单行应用状态：返回 (active_profile_id, default_account_id)。
    pub fn app_state(&self) -> AppResult<(Option<String>, Option<String>)> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT active_profile_id, default_account_id FROM app_state WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| app_err!("无法读取应用状态: {error}"))
            .map(|state| state.unwrap_or((None, None)))
    }

    pub fn set_active_codex_profile(&self, id: Option<&str>) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_state(singleton, active_profile_id) VALUES(1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET active_profile_id=excluded.active_profile_id",
                params![id],
            )
            .map_err(|error| app_err!("无法保存应用状态: {error}"))?;
        Ok(())
    }

    pub fn set_default_account(&self, id: Option<&str>) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_state(singleton, default_account_id) VALUES(1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET default_account_id=excluded.default_account_id",
                params![id],
            )
            .map_err(|error| app_err!("无法保存应用状态: {error}"))?;
        Ok(())
    }

    pub fn active_claude_profile(&self) -> AppResult<Option<String>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT active_claude_profile_id FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| app_err!("无法读取应用状态: {error}"))
            .map(Option::flatten)
    }

    pub fn claude_common_settings(&self) -> AppResult<Option<String>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT claude_common_settings FROM app_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| app_err!("无法读取 Claude 通用模板: {error}"))
            .map(Option::flatten)
    }

    pub fn set_claude_common_settings(&self, text: Option<&str>) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_state(singleton, claude_common_settings) VALUES(1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET claude_common_settings=excluded.claude_common_settings",
                params![text],
            )
            .map_err(|error| app_err!("无法保存 Claude 通用模板: {error}"))?;
        Ok(())
    }

    pub fn set_active_claude_profile(&self, id: Option<&str>) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_state(singleton, active_claude_profile_id) VALUES(1, ?1)
                 ON CONFLICT(singleton) DO UPDATE SET active_claude_profile_id=excluded.active_claude_profile_id",
                params![id],
            )
            .map_err(|error| app_err!("无法保存应用状态: {error}"))?;
        Ok(())
    }

    pub fn claude_profiles(&self) -> AppResult<Vec<StoredClaudeProfile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {CLAUDE_PROFILE_COLUMNS}
                 FROM claude_profiles ORDER BY sort_order ASC, created_at ASC, id ASC"
            ))
            .map_err(|error| app_err!("无法读取 Claude 供应商配置: {error}"))?;
        let rows = statement
            .query_map([], claude_profile_from_row)
            .map_err(|error| app_err!("无法读取 Claude 供应商配置: {error}"))?;
        let mut profiles = Vec::new();
        for row in rows {
            profiles.push(row.map_err(|error| app_err!("Claude 供应商配置数据无效: {error}"))?);
        }
        Ok(profiles)
    }

    pub fn claude_profile(&self, id: &str) -> AppResult<StoredClaudeProfile> {
        let connection = self.lock()?;
        connection
            .query_row(
                &format!("SELECT {CLAUDE_PROFILE_COLUMNS} FROM claude_profiles WHERE id = ?1"),
                params![id],
                claude_profile_from_row,
            )
            .optional()
            .map_err(|error| app_err!("无法读取 Claude 供应商配置: {error}"))?
            .ok_or_else(|| app_err!("Claude 供应商配置不存在"))
    }

    pub fn insert_claude_profile(
        &self,
        input: &ClaudeProfileInput,
        timestamp: &str,
    ) -> AppResult<StoredClaudeProfile> {
        let id = format!(
            "cla-{timestamp}-{}",
            PROFILE_ID_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let connection = self.lock()?;
        connection
            .query_row(
                &format!(
                    "INSERT INTO claude_profiles(id, name, base_url, auth_token, model, description, fetched_models, kind, admin_url, extra_env, raw_settings, icon, show_balance, created_at, updated_at, sort_order)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14,
                            (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM claude_profiles))
                     RETURNING {CLAUDE_PROFILE_COLUMNS}"
                ),
                params![
                    id,
                    &input.name,
                    &input.base_url,
                    &input.auth_token,
                    &input.model,
                    &input.description,
                    input
                        .fetched_models
                        .as_ref()
                        .map(|models| serde_json::to_string(models).unwrap_or_default()),
                    &input.kind,
                    &input.admin_url,
                    &input.extra_env,
                    &input.raw_settings,
                    &input.icon,
                    input.show_balance,
                    timestamp
                ],
                claude_profile_from_row,
            )
            .map_err(|error| app_err!("无法保存 Claude 供应商配置: {error}"))
    }

    pub fn update_claude_profile(
        &self,
        id: &str,
        input: &ClaudeProfileInput,
        timestamp: &str,
        before_commit: impl FnOnce(&StoredClaudeProfile) -> AppResult<()>,
    ) -> AppResult<StoredClaudeProfile> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始配置更新事务: {error}"))?;
        let updated = transaction
            .query_row(
                &format!(
                    "UPDATE claude_profiles SET name=?2, base_url=?3, auth_token=?4, model=?5, description=?6, fetched_models=?7, kind=?8, admin_url=?9, extra_env=?10, raw_settings=?11, icon=?12, show_balance=?13, updated_at=?14 WHERE id=?1
                     RETURNING {CLAUDE_PROFILE_COLUMNS}"
                ),
                params![
                    id,
                    &input.name,
                    &input.base_url,
                    &input.auth_token,
                    &input.model,
                    &input.description,
                    input
                        .fetched_models
                        .as_ref()
                        .map(|models| serde_json::to_string(models).unwrap_or_default()),
                    &input.kind,
                    &input.admin_url,
                    &input.extra_env,
                    &input.raw_settings,
                    &input.icon,
                    input.show_balance,
                    timestamp
                ],
                claude_profile_from_row,
            )
            .optional()
            .map_err(|error| app_err!("无法更新 Claude 供应商配置: {error}"))?
            .ok_or_else(|| app_err!("Claude 供应商配置不存在"))?;
        before_commit(&updated)?;
        transaction
            .commit()
            .map_err(|error| app_err!("无法提交配置更新事务: {error}"))?;
        Ok(updated)
    }

    pub fn delete_claude_profile(&self, id: &str) -> AppResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始配置删除事务: {error}"))?;
        let changed = transaction
            .execute("DELETE FROM claude_profiles WHERE id=?1", params![id])
            .map_err(|error| app_err!("无法删除 Claude 供应商配置: {error}"))?;
        if changed == 0 {
            return Err(app_err!("Claude 供应商配置不存在"));
        }
        transaction.execute("UPDATE app_state SET active_claude_profile_id=NULL WHERE active_claude_profile_id=?1", params![id])
            .map_err(|error| app_err!("无法清除激活配置: {error}"))?;
        transaction
            .commit()
            .map_err(|error| app_err!("无法提交配置删除事务: {error}"))
    }

    /// 卡片拖拽排序持久化（对齐 Codex codex_reorder_profiles：事务内逐行写 sort_order）。
    pub fn reorder_claude_profiles(&self, ids: &[String], timestamp: &str) -> AppResult<()> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开始排序事务: {error}"))?;
        for (index, id) in ids.iter().enumerate() {
            transaction
                .execute(
                    "UPDATE claude_profiles SET sort_order = ?2, updated_at = ?3 WHERE id = ?1",
                    params![id, index as i64, timestamp],
                )
                .map_err(|error| app_err!("保存排序失败: {error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| app_err!("提交排序事务失败: {error}"))
    }

    pub fn set_claude_profile_icon(
        &self,
        id: &str,
        icon: Option<&str>,
        timestamp: &str,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE claude_profiles SET icon=?2, updated_at=?3 WHERE id=?1",
                params![id, icon, timestamp],
            )
            .map_err(|error| app_err!("无法更新 Claude 供应商配置图标: {error}"))?;
        if changed == 0 {
            return Err(app_err!("Claude 供应商配置不存在"));
        }
        Ok(())
    }

    pub fn set_claude_profile_show_balance(
        &self,
        id: &str,
        enabled: bool,
        timestamp: &str,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE claude_profiles SET show_balance=?2, updated_at=?3 WHERE id=?1",
                params![id, enabled, timestamp],
            )
            .map_err(|error| app_err!("无法保存 Claude 用量开关: {error}"))?;
        if changed == 0 {
            return Err(app_err!("Claude 供应商配置不存在"));
        }
        Ok(())
    }

    pub fn record_event(
        &self,
        profile_id: Option<&str>,
        action: &str,
        status: &str,
        message: Option<&str>,
        timestamp: &str,
    ) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO switch_events(profile_id, action, status, message, created_at)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![profile_id, action, status, message, timestamp],
            )
            .map_err(|error| app_err!("无法记录操作日志: {error}"))?;
        Ok(())
    }

    /// 最近一次成功应用的供应商 id（应用记录被删除时返回 None 由调用方回退匹配）。
    pub fn codex_latest_applied_profile(&self) -> AppResult<Option<String>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT profile_id FROM switch_events
                 WHERE action = 'apply' AND status = 'success' AND profile_id IS NOT NULL
                 ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| app_err!("无法读取最近应用记录: {error}"))
    }

    /// 把当前数据库导出为一致性快照文件（VACUUM INTO）。
    pub fn export_database(&self, target: &Path) -> AppResult<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "VACUUM INTO ?1",
                params![target.to_string_lossy().to_string()],
            )
            .map_err(|error| app_err!("数据库导出失败: {error}"))?;
        Ok(())
    }

    /// 从备份文件把数据恢复进当前数据库（清空现有数据后复制）。
    pub fn restore_from_backup(&self, backup: &Path) -> AppResult<bool> {
        let source = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| app_err!("无法打开备份文件: {error}"))?;
        let has_profiles: i64 = source
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='profiles'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))?;
        if has_profiles == 0 {
            return Err(app_err!("备份文件不是有效的 Agmux 数据库"));
        }

        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(|error| app_err!("无法开启恢复事务: {error}"))?;
        transaction
            .execute("DELETE FROM switch_events", [])
            .and_then(|_| transaction.execute("DELETE FROM profiles", []))
            .and_then(|_| transaction.execute("DELETE FROM accounts", []))
            .and_then(|_| transaction.execute("DELETE FROM app_state", []))
            .and_then(|_| transaction.execute("DELETE FROM mcp_servers", []))
            .and_then(|_| transaction.execute("DELETE FROM claude_profiles", []))
            .and_then(|_| transaction.execute("DELETE FROM chatgpt_model_cache", []))
            .map_err(|error| app_err!("恢复前清理数据失败: {error}"))?;

        // 旧 schema 备份没有身份两列/套餐列：按旧列复制后回填；新备份带列复制
        let source_column = |name: &str| -> AppResult<i64> {
            source
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('accounts') WHERE name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))
        };
        let source_has_identity_columns = source_column("chatgpt_account_id")?;
        let source_has_plan_type = source_column("plan_type")?;
        if source_has_plan_type > 0 {
            copy_table(
                &source,
                &transaction,
                "accounts",
                "SELECT id, email, id_token, refresh_token, auth_json, authenticated_at,
                        chatgpt_account_id, user_identity, plan_type FROM accounts",
                "INSERT INTO accounts(id, email, id_token, refresh_token, auth_json, authenticated_at,
                                      chatgpt_account_id, user_identity, plan_type)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
        } else if source_has_identity_columns > 0 {
            copy_table(
                &source,
                &transaction,
                "accounts",
                "SELECT id, email, id_token, refresh_token, auth_json, authenticated_at,
                        chatgpt_account_id, user_identity FROM accounts",
                "INSERT INTO accounts(id, email, id_token, refresh_token, auth_json, authenticated_at,
                                      chatgpt_account_id, user_identity)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            backfill_account_plan_type(&transaction)
                .map_err(|error| app_err!("回填账号套餐失败: {error}"))?;
        } else {
            copy_table(
                &source,
                &transaction,
                "accounts",
                "SELECT id, email, id_token, refresh_token, auth_json, authenticated_at FROM accounts",
                "INSERT INTO accounts(id, email, id_token, refresh_token, auth_json, authenticated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            backfill_account_identity(&transaction)
                .map_err(|error| app_err!("回填账号身份失败: {error}"))?;
            backfill_account_plan_type(&transaction)
                .map_err(|error| app_err!("回填账号套餐失败: {error}"))?;
        }
        // 旧备份没有账号模型缓存；恢复后保持空缓存，不混入恢复前的账号数据。
        let has_model_cache: i64 = source.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='chatgpt_model_cache'",
            [], |row| row.get(0),
        ).map_err(|error| app_err!("读取备份模型缓存失败: {error}"))?;
        if has_model_cache > 0 {
            copy_intersected_columns(
                &source,
                &transaction,
                "chatgpt_model_cache",
                &["cache_key", "models_json", "updated_at"],
            )?;
        }
        // app_state 按列交集复制：旧 schema 备份缺 active_claude_profile_id 时落列默认 NULL
        copy_intersected_columns(
            &source,
            &transaction,
            "app_state",
            &[
                "singleton",
                "active_profile_id",
                "default_account_id",
                "active_claude_profile_id",
                "claude_common_settings",
            ],
        )?;
        // 备份带 sort_order 则原样复制（保留卡片排序）；旧 schema 备份无此列，落列默认 0（创建顺序）
        let source_has_sort_order: i64 = source
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('profiles') WHERE name = 'sort_order'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))?;
        if source_has_sort_order > 0 {
            copy_table(
                &source,
                &transaction,
                "profiles",
                "SELECT id, name, payload_json, icon, kind, account_id, created_at, updated_at, sort_order
                 FROM profiles",
                "INSERT INTO profiles(id, name, payload_json, icon, kind, account_id, created_at,
                                      updated_at, sort_order)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
        } else {
            copy_table(
                &source,
                &transaction,
                "profiles",
                "SELECT id, name, payload_json, icon, kind, account_id, created_at, updated_at FROM profiles",
                "INSERT INTO profiles(id, name, payload_json, icon, kind, account_id, created_at, updated_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
        }
        copy_table(
            &source,
            &transaction,
            "switch_events",
            "SELECT id, profile_id, action, status, message, created_at FROM switch_events",
            "INSERT INTO switch_events(id, profile_id, action, status, message, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        // 旧备份可能还没有 mcp_servers 表：无表跳过（保持 live 现状，不误清）
        let has_mcp_servers: i64 = source
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='mcp_servers'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))?;
        if has_mcp_servers > 0 {
            copy_intersected_columns(
                &source,
                &transaction,
                "mcp_servers",
                &[
                    "name",
                    "toml",
                    "sort_order",
                    "updated_at",
                    "codex_enabled",
                    "claude_enabled",
                    "claude_json",
                    "codex_installed",
                    "claude_installed",
                ],
            )?;
        }
        // 旧备份可能还没有 claude_profiles 表：无表跳过（Claude 供应商从零开始）
        let has_claude_profiles: i64 = source
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='claude_profiles'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))?;
        if has_claude_profiles > 0 {
            // 列与 CLAUDE_PROFILE_COLUMNS 正典序求交集：备份有哪些列就复制哪些，缺列落默认 NULL
            copy_intersected_columns(
                &source,
                &transaction,
                "claude_profiles",
                &CLAUDE_PROFILE_COLUMNS.split(", ").collect::<Vec<_>>(),
            )?;
        }

        transaction
            .commit()
            .map_err(|error| app_err!("恢复事务提交失败: {error}"))?;
        Ok(has_mcp_servers > 0)
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| app_err!("数据库连接锁已损坏，请重启 Agmux"))
    }
}

fn copy_table(
    source: &Connection,
    destination: &rusqlite::Transaction<'_>,
    table: &str,
    select_sql: &str,
    insert_sql: &str,
) -> AppResult<()> {
    let mut statement = source
        .prepare(select_sql)
        .map_err(|error| app_err!("读取备份表 {table} 失败: {error}"))?;
    let column_count = statement.column_count();
    let mut rows = statement
        .query([])
        .map_err(|error| app_err!("读取备份表 {table} 失败: {error}"))?;
    while let Some(row) = rows
        .next()
        .map_err(|error| app_err!("读取备份表 {table} 失败: {error}"))?
    {
        let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(
                row.get::<_, rusqlite::types::Value>(index)
                    .map_err(|error| app_err!("读取备份表 {table} 失败: {error}"))?,
            );
        }
        destination
            .execute(insert_sql, rusqlite::params_from_iter(values))
            .map_err(|error| app_err!("恢复表 {table} 失败: {error}"))?;
    }
    Ok(())
}

/// 备份表列与正典列序求交集：备份有哪些列就按正典序复制哪些，缺列由目标表默认值兜底。
/// （restore_from_backup 的逐档手写分支由此收敛成一条复制语句。）
fn copy_intersected_columns(
    source: &Connection,
    destination: &rusqlite::Transaction<'_>,
    table: &str,
    canonical_columns: &[&str],
) -> AppResult<()> {
    let mut columns: Vec<&str> = Vec::new();
    for name in canonical_columns {
        let present: i64 = source
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                params![name],
                |row| row.get(0),
            )
            .map_err(|error| app_err!("备份文件不是有效的 Agmux 数据库: {error}"))?;
        if present > 0 {
            columns.push(name);
        }
    }
    let column_list = columns.join(", ");
    let placeholders: Vec<String> = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect();
    copy_table(
        source,
        destination,
        table,
        &format!("SELECT {column_list} FROM {table}"),
        &format!(
            "INSERT INTO {table}({column_list}) VALUES({})",
            placeholders.join(", ")
        ),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCodexProfile {
    pub id: String,
    pub name: String,
    pub payload: CodexProfilePayload,
    pub icon: Option<String>,
    pub kind: CodexProfileKind,
    pub account_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Claude Code 供应商行：字段固定即列，不走 payload_json blob。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredClaudeProfile {
    pub id: String,
    pub name: String,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub model: Option<String>,
    pub description: Option<String>,
    /// 已获取的模型 ID 列表（JSON 数组文本，对齐 Codex fetched_models）。
    pub fetched_models: Option<String>,
    /// 创建时选择的预设 kind（自定义/无预设为 NULL）：编辑页反查端点档用。
    pub kind: Option<String>,
    pub admin_url: Option<String>,
    /// 附加 env 键值（JSON 对象文本）：应用时并入 settings.json 的 env，先移除旧值再写入。
    pub extra_env: Option<String>,
    /// 完整 settings.json 原文（旧行为空）。
    pub raw_settings: Option<String>,
    /// 图标 id（icons.ts 收集的 provider 图标；NULL 显示名称首字）。
    pub icon: Option<String>,
    pub show_balance: bool,
    pub created_at: String,
    pub updated_at: String,
    /// 卡片排序（拖拽持久化；同序按 created_at）。
    pub sort_order: i64,
}

const CLAUDE_PROFILE_COLUMNS: &str =
    "id, name, base_url, auth_token, model, description, fetched_models, kind, admin_url, extra_env, raw_settings, icon, show_balance, created_at, updated_at, sort_order";

fn claude_profile_from_row(row: &rusqlite::Row) -> rusqlite::Result<StoredClaudeProfile> {
    Ok(StoredClaudeProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        base_url: row.get(2)?,
        auth_token: row.get(3)?,
        model: row.get(4)?,
        description: row.get(5)?,
        fetched_models: row.get(6)?,
        kind: row.get(7)?,
        admin_url: row.get(8)?,
        extra_env: row.get(9)?,
        raw_settings: row.get(10)?,
        icon: row.get(11)?,
        show_balance: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        sort_order: row.get(15)?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub id: String,
    pub email: Option<String>,
    pub id_token: Option<String>,
    pub refresh_token: String,
    pub auth_json: Option<String>,
    pub authenticated_at: i64,
    /// ChatGPT workspace ID：出站语义（auth.json tokens.account_id、chatgpt-account-id 头）。
    /// 存量行回填为行 id，新行由登录流程写入。
    pub chatgpt_account_id: Option<String>,
    /// id_token 的 sub：跨刷新稳定的用户身份，判重与 live auth.json 所有权校验用。
    pub user_identity: Option<String>,
    /// 订阅套餐（free/plus/pro/team…），随 id_token 刷新而更新
    pub plan_type: Option<String>,
}

fn profile_from_row(row: &Row<'_>) -> rusqlite::Result<StoredCodexProfile> {
    let id = row.get(0)?;
    let name = row.get(1)?;
    let payload_json = row.get::<_, String>(2)?;
    let payload = serde_json::from_str(&payload_json).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let kind_raw: String = row.get(4)?;
    let kind = CodexProfileKind::from_db(&kind_raw).ok_or(rusqlite::Error::InvalidQuery)?;
    Ok(StoredCodexProfile {
        id,
        name,
        payload,
        icon: row.get(3)?,
        kind,
        account_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<StoredAccount> {
    Ok(StoredAccount {
        id: row.get(0)?,
        email: row.get(1)?,
        id_token: row.get(2)?,
        refresh_token: row.get(3)?,
        auth_json: row.get(4)?,
        authenticated_at: row.get(5)?,
        chatgpt_account_id: row.get(6)?,
        user_identity: row.get(7)?,
        plan_type: row.get(8)?,
    })
}

fn summary(
    id: &str,
    name: &str,
    payload: &CodexProfilePayload,
    icon: Option<&str>,
    account_id: Option<&str>,
    created_at: &str,
    updated_at: &str,
) -> CodexProfileSummary {
    let mut model = display_text(payload.model_values.get("model"));
    let mut reasoning_effort = display_text(payload.model_values.get("model_reasoning_effort"));
    if payload.provider_id.is_none() {
        let has_local_catalog = payload.model_values.contains_key("model_catalog_json");
        let local_catalog = if has_local_catalog {
            payload
                .raw_catalog
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok())
                .and_then(|value| crate::services::parse_chatgpt_models(value).ok())
        } else {
            None
        };
        model = model.or_else(|| {
            if has_local_catalog {
                local_catalog
                    .as_ref()?
                    .first()
                    .map(|model| model.slug.clone())
            } else {
                payload.fetched_models.first().cloned()
            }
        });
        reasoning_effort = reasoning_effort.or_else(|| {
            let id = model.as_ref()?;
            if has_local_catalog {
                local_catalog
                    .as_ref()?
                    .iter()
                    .find(|model| &model.slug == id)?
                    .default_reasoning_level
                    .clone()
            } else {
                payload.fetched_model_defaults.get(id).cloned()
            }
        });
    }
    CodexProfileSummary {
        id: id.into(),
        name: name.into(),
        kind: if payload.provider_id.is_some() {
            CodexProfileKind::ThirdParty
        } else {
            CodexProfileKind::Official
        },
        account_id: account_id.map(str::to_string),
        auth_source: payload.effective_auth_source(
            if payload.provider_id.is_some() {
                CodexProfileKind::ThirdParty
            } else {
                CodexProfileKind::Official
            },
            account_id,
        ),
        auth_account_id: None,
        // 套餐由 get_state 聚合时按绑定账号/live 认证填充，Database 层不知道
        plan_type: None,
        model,
        provider: payload.provider_id.clone(),
        reasoning_effort,
        fast_mode: display_text(payload.model_values.get("service_tier")).as_deref()
            == Some("fast"),
        has_base_url: payload
            .provider_body
            .as_deref()
            .and_then(|body| parse_provider_detail(body).ok())
            .and_then(|detail| detail.base_url)
            .is_some_and(|url| !url.trim().is_empty()),
        has_key: stored_provider_api_key(payload).is_some(),
        admin_url: payload.admin_url.clone(),
        show_balance: payload.show_balance,
        icon: icon.map(str::to_string),
        created_at: created_at.into(),
        updated_at: updated_at.into(),
    }
}

fn display_text(value: Option<&String>) -> Option<String> {
    value.map(|raw| raw.trim().trim_matches('"').to_string())
}

pub fn codex_profile_summary(profile: &StoredCodexProfile) -> CodexProfileSummary {
    summary(
        &profile.id,
        &profile.name,
        &profile.payload,
        profile.icon.as_deref(),
        profile.account_id.as_deref(),
        &profile.created_at,
        &profile.updated_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_models_preserve_valid_cache_and_survive_backup_and_legacy_restore() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();
        let models: Vec<crate::services::ChatgptModel> = serde_json::from_value(serde_json::json!([
            {"slug":"fixture", "display_name":"Fixture", "visibility":"list",
                "supported_reasoning_levels":[{"effort":"low"}], "default_reasoning_level":"low"}]
        )).unwrap();
        db.set_chatgpt_models("account-a", &models).unwrap();
        assert!(db.set_chatgpt_models("account-a", &[]).is_err());
        assert_eq!(
            db.chatgpt_models("account-a").unwrap().unwrap()[0].slug,
            "fixture"
        );
        assert!(db.chatgpt_models("account-b").unwrap().is_none());
        let backup = dir.path().join("models.db");
        db.export_database(&backup).unwrap();
        db.set_chatgpt_models("account-b", &models).unwrap();
        db.restore_from_backup(&backup).unwrap();
        let cached = db.chatgpt_models("account-a").unwrap().unwrap();
        assert_eq!(cached[0].supported_reasoning_levels[0].effort, "low");
        assert!(db.chatgpt_models("account-b").unwrap().is_none());
        let legacy = dir.path().join("legacy-models.db");
        let mut old = Connection::open(&legacy).unwrap();
        migrations().to_version(&mut old, 21).unwrap();
        drop(old);
        db.restore_from_backup(&legacy).unwrap();
        assert!(db.chatgpt_models("account-a").unwrap().is_none());
    }

    #[test]
    fn rename_profile_only_changes_target_clients_name_and_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();
        let codex = db
            .codex_insert_profile("codex-fixture", &CodexProfilePayload::default(), "1")
            .unwrap();
        let claude = db
            .insert_claude_profile(
                &ClaudeProfileInput {
                    name: "claude-fixture".into(),
                    raw_settings: Some(
                        r#"{"env":{"CUSTOM":"keep"},"permissions":{"deny":["Write"]}}"#.into(),
                    ),
                    auth_token: Some("fixture-token".into()),
                    description: Some("keep".into()),
                    show_balance: true,
                    ..Default::default()
                },
                "1",
            )
            .unwrap();
        db.set_active_codex_profile(Some(&codex.id)).unwrap();
        db.set_active_claude_profile(Some(&claude.id)).unwrap();
        let mut expected_codex =
            serde_json::to_value(db.codex_profile(&codex.id).unwrap()).unwrap();
        let mut expected_claude =
            serde_json::to_value(db.claude_profile(&claude.id).unwrap()).unwrap();

        db.rename_profile(&claude.id, "renamed-claude", "2", SkillTool::Claude)
            .unwrap();
        expected_claude["name"] = "renamed-claude".into();
        expected_claude["updated_at"] = "2".into();
        assert_eq!(
            serde_json::to_value(db.claude_profile(&claude.id).unwrap()).unwrap(),
            expected_claude
        );
        assert_eq!(
            serde_json::to_value(db.codex_profile(&codex.id).unwrap()).unwrap(),
            expected_codex
        );

        db.rename_profile(&codex.id, "renamed-codex", "3", SkillTool::Codex)
            .unwrap();
        expected_codex["name"] = "renamed-codex".into();
        expected_codex["updated_at"] = "3".into();
        assert_eq!(
            serde_json::to_value(db.codex_profile(&codex.id).unwrap()).unwrap(),
            expected_codex
        );
        assert_eq!(
            serde_json::to_value(db.claude_profile(&claude.id).unwrap()).unwrap(),
            expected_claude
        );
        assert_eq!(
            db.app_state().unwrap().0.as_deref(),
            Some(codex.id.as_str())
        );
        assert_eq!(
            db.active_claude_profile().unwrap().as_deref(),
            Some(claude.id.as_str())
        );
        assert!(db
            .rename_profile(&claude.id, "wrong-client", "4", SkillTool::Codex)
            .is_err());
        assert!(db
            .rename_profile(&codex.id, "wrong-client", "4", SkillTool::Claude)
            .is_err());
    }

    #[test]
    fn common_template_migrates_and_round_trips_without_changing_active_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        paths.ensure().unwrap();
        {
            let mut connection = Connection::open(&paths.database).unwrap();
            migrations().to_version(&mut connection, 18).unwrap();
            connection.execute("INSERT INTO app_state(singleton, active_profile_id, active_claude_profile_id) VALUES(1, 'codex-fixture', 'claude-fixture')", []).unwrap();
        }
        let legacy = dir.path().join("legacy.db");
        std::fs::copy(&paths.database, &legacy).unwrap();
        let db = Database::open(&paths).unwrap();
        assert_eq!(db.claude_common_settings().unwrap(), None);
        assert_eq!(db.app_state().unwrap().0.as_deref(), Some("codex-fixture"));
        assert_eq!(
            db.active_claude_profile().unwrap().as_deref(),
            Some("claude-fixture")
        );

        let raw = r#"{"hooks":{"Stop":[]},"env":{"CUSTOM":"keep"},"unknown":{"nested":[1,null]}}"#;
        db.set_claude_common_settings(Some(raw)).unwrap();
        let backup = dir.path().join("template.db");
        db.export_database(&backup).unwrap();
        db.set_claude_common_settings(Some("{}")).unwrap();
        db.restore_from_backup(&backup).unwrap();
        assert_eq!(db.claude_common_settings().unwrap().as_deref(), Some(raw));
        let invalid = dir.path().join("invalid.db");
        Connection::open(&invalid)
            .unwrap()
            .execute_batch("CREATE TABLE profiles(id TEXT)")
            .unwrap();
        assert!(db.restore_from_backup(&invalid).is_err());
        assert_eq!(db.claude_common_settings().unwrap().as_deref(), Some(raw));
        assert_eq!(db.app_state().unwrap().0.as_deref(), Some("codex-fixture"));
        assert_eq!(
            db.active_claude_profile().unwrap().as_deref(),
            Some("claude-fixture")
        );

        // 旧备份没有模板列：恢复后清空当前模板，不能留下本次恢复之外的数据。
        db.restore_from_backup(&legacy).unwrap();
        assert_eq!(db.claude_common_settings().unwrap(), None);
        db.set_claude_common_settings(Some(raw)).unwrap();
        db.set_claude_common_settings(None).unwrap();
        assert_eq!(db.claude_common_settings().unwrap(), None);
        assert_eq!(
            db.active_claude_profile().unwrap().as_deref(),
            Some("claude-fixture")
        );
    }

    #[test]
    fn migration_creates_schema() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();

        let names: Vec<String> = db
            .lock()
            .unwrap()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(names.contains(&"profiles".into()));
        assert!(names.contains(&"app_state".into()));
        assert!(names.contains(&"switch_events".into()));
        assert!(names.contains(&"accounts".into()));
        assert!(names.contains(&"mcp_servers".into()));
        assert!(names.contains(&"claude_profiles".into()));
    }

    #[test]
    fn migration_from_v20_preserves_claude_profile_and_opens_v21() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        paths.ensure().unwrap();
        {
            let mut connection = Connection::open(&paths.database).unwrap();
            migrations().to_version(&mut connection, 20).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO claude_profiles(id, name, created_at, updated_at)
                 VALUES('claude-existing', 'Existing', '1', '1');
                 INSERT INTO app_state(singleton, active_claude_profile_id)
                 VALUES(1, 'claude-existing')",
                )
                .unwrap();
        }

        let db = Database::open(&paths).unwrap();
        assert_eq!(
            db.claude_profile("claude-existing").unwrap().name,
            "Existing"
        );
        assert_eq!(
            db.active_claude_profile().unwrap().as_deref(),
            Some("claude-existing")
        );
        let connection = db.lock().unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            22
        );
    }

    /// Claude 支持引入前（user_version = 7）的存量库升级：既有数据完好，
    /// Claude 表全列与激活位就位且默认空，global_enabled 给存量 MCP 行落默认 1。
    #[test]
    fn migration_from_pre_claude_database_preserves_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        paths.ensure().unwrap();
        {
            let mut connection = Connection::open(&paths.database).unwrap();
            migrations().to_version(&mut connection, 7).unwrap();
            connection
                .execute_batch(
                    r#"INSERT INTO profiles(id, name, payload_json, kind, created_at, updated_at, sort_order)
                       VALUES ('p-1', 'GLM', '{}', 'third_party', '1', '1', 0);
                       INSERT INTO accounts(id, email, refresh_token, authenticated_at, chatgpt_account_id)
                       VALUES ('acc-1', 'a@example.com', 'rt-1', 1, 'acc-1');
                       INSERT INTO mcp_servers(name, toml, sort_order, updated_at)
                       VALUES ('github', '[mcp_servers.github]', 0, '1');
                       INSERT INTO app_state(singleton, active_profile_id, default_account_id)
                       VALUES (1, 'p-1', 'acc-1');"#,
                )
                .unwrap();
        }

        let db = Database::open(&paths).unwrap();

        // 既有 Codex 数据完好
        let profiles = db.codex_profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "GLM");
        assert_eq!(db.accounts().unwrap()[0].refresh_token, "rt-1");
        assert_eq!(
            db.mcp_server_fragments().unwrap(),
            vec![("github".to_string(), "[mcp_servers.github]".to_string())]
        );
        let (active, default) = db.app_state().unwrap();
        assert_eq!(active.as_deref(), Some("p-1"));
        assert_eq!(default.as_deref(), Some("acc-1"));

        // Claude 表全列就位、激活位默认空；存量 MCP 行落 global_enabled 默认 1
        assert_eq!(db.active_claude_profile().unwrap(), None);
        assert!(db.claude_profiles().unwrap().is_empty());
        let connection = db.lock().unwrap();
        let columns: Vec<String> = connection
            .prepare("SELECT name FROM pragma_table_info('claude_profiles') ORDER BY cid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for column in [
            "id",
            "name",
            "base_url",
            "auth_token",
            "model",
            "created_at",
            "updated_at",
            "kind",
            "admin_url",
            "description",
            "fetched_models",
            "extra_env",
            "icon",
            "sort_order",
            "raw_settings",
            "show_balance",
        ] {
            assert!(
                columns.iter().any(|name| name == column),
                "claude_profiles 缺列 {column}"
            );
        }
        let (codex_enabled, claude_enabled): (i64, i64) = connection
            .query_row(
                "SELECT codex_enabled, claude_enabled FROM mcp_servers WHERE name = 'github'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((codex_enabled, claude_enabled), (1, 1));
        // 开发期中间态列已随迁移移除
        let stale: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('mcp_servers') WHERE name = 'global_enabled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0);
    }

    /// 迁移只能追加在列表末尾：新迁移插队到已发布槽位之前，会推后后续迁移的序号，
    /// 让已应用过它们的存量库（user_version 停留在插队前）按新序号重放旧迁移而崩溃。
    /// 本测试钉住 MCP 拆分迁移边界：v16 库升级到最新只允许追加新列，不得重放旧迁移。
    #[test]
    fn migration_from_mcp_split_database_only_appends_new_columns() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        paths.ensure().unwrap();
        {
            let mut connection = Connection::open(&paths.database).unwrap();
            migrations().to_version(&mut connection, 16).unwrap();
            let column_exists = |table: &str, column: &str| -> bool {
                connection
                    .query_row(
                        &format!(
                            "SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name = '{column}'"
                        ),
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|count| count > 0)
                    .unwrap()
            };
            // v16 = MCP 拆分迁移刚应用完：codex/claude 开关在、global_enabled 已移除
            assert!(column_exists("mcp_servers", "codex_enabled"));
            assert!(!column_exists("mcp_servers", "global_enabled"));
            assert!(!column_exists("claude_profiles", "show_balance"));
            connection.execute("INSERT INTO mcp_servers(name, toml, sort_order, updated_at, codex_enabled, claude_enabled) VALUES('fixture', '[mcp_servers.fixture]\ncommand=\"echo\"', 0, '1', 0, 1)", []).unwrap();
        }

        let db = Database::open(&paths).unwrap();
        let record = db.mcp_server_record("fixture").unwrap().unwrap();
        assert!(!record.codex_enabled);
        assert!(record.claude_enabled);
        assert!(record.codex_installed);
        assert!(record.claude_installed);
        assert!(record.claude_json.is_none());
        let connection = db.lock().unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 22);
        let show_balance: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('claude_profiles') WHERE name = 'show_balance'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(show_balance, 1);
    }

    #[test]
    fn mcp_fragments_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();

        assert!(db.mcp_server_fragments().unwrap().is_empty());

        let fragments = vec![
            (
                "github".to_string(),
                "[mcp_servers.github]\ncommand = \"node\"\n".to_string(),
            ),
            (
                "tavily".to_string(),
                "[mcp_servers.tavily]\nurl = \"https://x/mcp\"\n".to_string(),
            ),
        ];
        db.replace_mcp_server_fragments(&fragments, "1").unwrap();
        assert_eq!(db.mcp_server_fragments().unwrap(), fragments);

        // 全量替换：旧条目清空，顺序按传入顺序
        let replaced = vec![(
            "fresh".to_string(),
            "[mcp_servers.fresh]\nurl = \"https://y\"\n".to_string(),
        )];
        db.replace_mcp_server_fragments(&replaced, "2").unwrap();
        assert_eq!(db.mcp_server_fragments().unwrap(), replaced);
    }

    #[test]
    fn set_profile_icon_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();

        let payload = CodexProfilePayload::default();
        let summary = db.codex_insert_profile("GLM High", &payload, "1").unwrap();
        assert_eq!(summary.icon, None);

        db.codex_set_profile_icon(&summary.id, Some("zhipu"), "2")
            .unwrap();
        assert_eq!(
            db.codex_profiles().unwrap()[0].icon.as_deref(),
            Some("zhipu")
        );

        db.codex_set_profile_icon(&summary.id, None, "3").unwrap();
        assert_eq!(db.codex_profiles().unwrap()[0].icon, None);

        assert!(db
            .codex_set_profile_icon("missing", Some("zhipu"), "4")
            .is_err());
    }

    #[test]
    fn profile_kind_and_accounts_state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        let db = Database::open(&paths).unwrap();

        // 无供应商 → 官方；有供应商 → 第三方
        let mut official = CodexProfilePayload::default();
        official
            .model_values
            .insert("model".into(), "\"gpt-5.6\"".into());
        let official_id = db.codex_insert_profile("官方", &official, "1").unwrap().id;
        let third = CodexProfilePayload {
            provider_id: Some("ZAI".into()),
            provider_body: Some("name = \"ZAI\"".into()),
            ..Default::default()
        };
        let third_id = db.codex_insert_profile("第三方", &third, "2").unwrap().id;
        assert_eq!(
            db.codex_profile(&official_id).unwrap().kind,
            CodexProfileKind::Official
        );
        assert_eq!(
            db.codex_profile(&third_id).unwrap().kind,
            CodexProfileKind::ThirdParty
        );
        assert!(!codex_profile_summary(&db.codex_profile(&third_id).unwrap()).has_base_url);

        let account = StoredAccount {
            id: "acc-1".into(),
            email: Some("a@example.com".into()),
            id_token: Some("id-jwt".into()),
            refresh_token: "rt-1".into(),
            auth_json: None,
            authenticated_at: 100,
            chatgpt_account_id: Some("acc-1".into()),
            user_identity: None,
            plan_type: None,
        };
        db.upsert_account(&account).unwrap();
        assert_eq!(db.accounts().unwrap()[0].refresh_token, "rt-1");
        assert_eq!(
            db.accounts().unwrap()[0].chatgpt_account_id,
            Some("acc-1".into())
        );
        db.delete_account("acc-1").unwrap();
        assert!(db.accounts().unwrap().is_empty());

        assert_eq!(db.app_state().unwrap(), (None, None));
        db.set_active_codex_profile(Some(&official_id)).unwrap();
        db.set_default_account(Some("acc-1")).unwrap();
        let (active, default) = db.app_state().unwrap();
        assert_eq!(active.as_deref(), Some(official_id.as_str()));
        assert_eq!(default.as_deref(), Some("acc-1"));
        db.set_active_codex_profile(None).unwrap();
        db.set_default_account(None).unwrap();
        assert_eq!(db.app_state().unwrap(), (None, None));
    }

    #[test]
    fn profile_summary_reports_non_empty_provider_endpoint() {
        let payload = CodexProfilePayload {
            provider_id: Some("ZAI".into()),
            provider_body: Some("base_url = \"https://api.example\"".into()),
            ..Default::default()
        };
        let summary_with_url = summary("id", "name", &payload, None, None, "1", "1");
        assert!(summary_with_url.has_base_url);

        let empty_payload = CodexProfilePayload {
            provider_id: Some("ZAI".into()),
            provider_body: Some("base_url = \"  \"".into()),
            ..Default::default()
        };
        assert!(!summary("id", "name", &empty_payload, None, None, "1", "1").has_base_url);
    }

    /// 构造带 sub 的 id_token（迁移回填的数据源）
    fn legacy_id_token(sub: &str) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "sub": sub,
                "https://api.openai.com/auth": { "chatgpt_plan_type": "plus" }
            })
            .to_string(),
        );
        format!("{header}.{payload}.")
    }

    /// 手工搭建 v5 旧库（无身份两列），打开后应自动回填
    #[test]
    fn migration_backfills_account_identity_columns() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::paths::from_home(dir.path()).unwrap();
        paths.ensure().unwrap();
        {
            let connection = Connection::open(&paths.database).unwrap();
            // 旧版应用迁移列表共 4 条（SCHEMA/sort_order/auth_json/mcp_servers），真实存量 user_version = 4
            connection.pragma_update(None, "user_version", 4).unwrap();
            connection
                .execute_batch(
                    r#"CREATE TABLE app_state (
                         singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                         active_profile_id TEXT,
                         default_account_id TEXT
                       );
                       CREATE TABLE accounts (
                         id TEXT PRIMARY KEY,
                         email TEXT,
                         id_token TEXT,
                         refresh_token TEXT NOT NULL,
                         authenticated_at INTEGER NOT NULL,
                         auth_json TEXT
                       );
                       CREATE TABLE mcp_servers (
                         name TEXT PRIMARY KEY,
                         toml TEXT NOT NULL,
                         sort_order INTEGER NOT NULL DEFAULT 0,
                         updated_at TEXT NOT NULL
                       );
                       INSERT INTO accounts VALUES
                         ('ws-1', 'a@example.com', NULL, 'rt-1', 1, NULL),
                         ('ws-2', 'b@example.com', NULL, 'rt-2', 2, NULL);
                       UPDATE accounts SET id_token = NULL WHERE id = 'ws-2';"#,
                )
                .unwrap();
        }
        let id_token = legacy_id_token("user-a");
        {
            let connection = Connection::open(&paths.database).unwrap();
            connection
                .execute(
                    "UPDATE accounts SET id_token = ?1 WHERE id = 'ws-1'",
                    params![id_token],
                )
                .unwrap();
        }

        let db = Database::open(&paths).unwrap();
        let accounts = db.accounts().unwrap();
        let row_a = accounts.iter().find(|a| a.id == "ws-1").unwrap();
        let row_b = accounts.iter().find(|a| a.id == "ws-2").unwrap();
        assert_eq!(row_a.chatgpt_account_id.as_deref(), Some("ws-1"));
        assert_eq!(row_a.user_identity.as_deref(), Some("user-a"));
        assert_eq!(row_a.plan_type.as_deref(), Some("plus"));
        // 无 id_token 的行回填 workspace、身份留空（后续登录按 email 认领）
        assert_eq!(row_b.chatgpt_account_id.as_deref(), Some("ws-2"));
        assert_eq!(row_b.user_identity, None);
        assert_eq!(row_b.plan_type, None);
    }

    /// 旧版本导出的备份（accounts 无身份两列）恢复后应回填并保留引用
    #[test]
    fn restore_from_legacy_backup_backfills_identity_columns() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("legacy-backup.db");
        {
            let connection = Connection::open(&source_path).unwrap();
            connection
                .execute_batch(&format!(
                    r#"CREATE TABLE app_state (
                         singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                         active_profile_id TEXT,
                         default_account_id TEXT
                       );
                       CREATE TABLE switch_events (
                         id INTEGER PRIMARY KEY AUTOINCREMENT,
                         profile_id TEXT,
                         action TEXT NOT NULL,
                         status TEXT NOT NULL,
                         message TEXT,
                         created_at TEXT NOT NULL
                       );
                       CREATE TABLE profiles (
                         id TEXT PRIMARY KEY,
                         name TEXT NOT NULL,
                         payload_json TEXT NOT NULL,
                         icon TEXT,
                         kind TEXT NOT NULL,
                         account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
                         created_at TEXT NOT NULL,
                         updated_at TEXT NOT NULL
                       );
                       CREATE TABLE accounts (
                         id TEXT PRIMARY KEY,
                         email TEXT,
                         id_token TEXT,
                         refresh_token TEXT NOT NULL,
                         authenticated_at INTEGER NOT NULL,
                         auth_json TEXT
                       );
                       INSERT INTO accounts VALUES
                         ('ws-1', 'a@example.com', '{id_token}', 'rt-1', 1, NULL);
                       INSERT INTO profiles VALUES
                         ('p1', '官方', '{{}}', NULL, 'official', 'ws-1', '1', '1');"#,
                    id_token = legacy_id_token("user-a")
                ))
                .unwrap();
        }

        let paths = crate::paths::from_home(&dir.path().join("home")).unwrap();
        let db = Database::open(&paths).unwrap();
        db.restore_from_backup(&source_path).unwrap();

        let accounts = db.accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].chatgpt_account_id.as_deref(), Some("ws-1"));
        assert_eq!(accounts[0].user_identity.as_deref(), Some("user-a"));
        assert_eq!(accounts[0].plan_type.as_deref(), Some("plus"));
        // 恢复未重映射 id：供应商绑定原样有效
        assert_eq!(
            db.codex_profiles().unwrap()[0].account_id.as_deref(),
            Some("ws-1")
        );
    }
}
