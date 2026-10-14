use super::{
    app_err, atomic_write, backup_file, codex_config, now_ms, AppContext, AppResult, BTreeMap,
    McpDiffEntryAction, McpServerSpec, McpSyncDiffEntry, McpSyncEntryKind, McpSyncPreview,
};
use crate::database::{Database, McpServerRecord};
use crate::paths::AppPaths;
use crate::services::plugins::SkillTool;

fn without_blank_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty() && line != "[mcp_servers]"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_live_config(paths: &AppPaths) -> AppResult<String> {
    let path = paths.codex_config();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(app_err!("无法读取 {}: {error}", path.display())),
    }
}

pub(super) fn validate_mcp_connection(spec: &McpServerSpec) -> AppResult<()> {
    let url = spec
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let command = spec
        .command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (url, command) {
        (Some(_), Some(_)) => Err(app_err!("不能同时填写启动命令和服务地址")),
        (None, None) => Err(app_err!(
            "必须填写启动命令（stdio）或服务地址（http）其中之一"
        )),
        (Some(url), None) if !url.starts_with("http://") && !url.starts_with("https://") => {
            Err(app_err!("服务地址必须以 http:// 或 https:// 开头"))
        }
        _ => Ok(()),
    }
}

/// 首次操作从现场读取两端快照，保留另一端的原生配置与状态。
fn mcp_record_from_live(paths: &AppPaths, name: &str) -> AppResult<McpServerRecord> {
    let codex_fragment = read_live_config(paths)
        .ok()
        .and_then(|text| codex_config::parse_document(&text).ok())
        .and_then(|document| {
            codex_config::mcp_server_fragments_from_document(&document)
                .into_iter()
                .find(|(server_name, _)| server_name == name)
                .map(|(_, toml)| toml)
        });
    let claude_entry = super::claude::read_claude_mcp_document(paths)
        .ok()
        .and_then(|document| {
            document
                .get("mcpServers")
                .and_then(|servers| servers.get(name))
                .cloned()
        });
    if codex_fragment.is_none() && claude_entry.is_none() {
        return Err(app_err!("MCP 服务器不存在: {name}"));
    }
    Ok(McpServerRecord {
        name: name.to_string(),
        toml: codex_fragment.clone().unwrap_or_default(),
        codex_enabled: codex_fragment.is_some(),
        claude_enabled: claude_entry.is_some(),
        codex_installed: codex_fragment.is_some(),
        claude_installed: claude_entry.is_some(),
        claude_json: claude_entry.map(|entry| entry.to_string()),
    })
}

/// 卸载只写所选客户端；安装状态防止后续投影重新添加。
pub(super) fn uninstall_mcp_server(
    database: &Database,
    paths: &AppPaths,
    name: &str,
    tool: SkillTool,
) -> AppResult<()> {
    if codex_config::is_managed_mcp_name(name) {
        return Err(app_err!("「{name}」由 Codex 官方应用自动管理，不能卸载"));
    }
    let record = database.mcp_server_record(name)?;
    let installed = record.as_ref().is_some_and(|record| match tool {
        SkillTool::Codex => record.codex_installed,
        SkillTool::Claude => record.claude_installed,
    });
    let (path, text, present, stem) = match tool {
        SkillTool::Codex => {
            let mut document = codex_config::parse_document(&read_live_config(paths)?)?;
            let present = document
                .get("mcp_servers")
                .and_then(|servers| servers.get(name))
                .is_some();
            if present {
                codex_config::remove_mcp_server(&mut document, name)?;
            }
            (
                paths.codex_config(),
                codex_config::normalize_global_section_order(&document.to_string()),
                present,
                "config",
            )
        }
        SkillTool::Claude => {
            let mut document = super::claude::read_claude_mcp_document(paths)?;
            let present = document
                .get_mut("mcpServers")
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|servers| servers.remove(name))
                .is_some();
            let text = serde_json::to_string_pretty(&document)
                .map_err(|error| app_err!("MCP JSON 序列化失败: {error}"))?;
            (
                paths.claude_mcp_config(),
                format!("{text}\n"),
                present,
                "claude-mcp",
            )
        }
    };
    if !present && !installed {
        return Err(app_err!("MCP 服务器不存在: {name}"));
    }
    let mut record = match record {
        Some(record) => record,
        None => mcp_record_from_live(paths, name)?,
    };
    match tool {
        SkillTool::Codex => {
            record.codex_installed = false;
            record.codex_enabled = false;
        }
        SkillTool::Claude => {
            record.claude_installed = false;
            record.claude_enabled = false;
        }
    }
    crate::fsutil::with_file_rollback(std::slice::from_ref(&path), || {
        if present {
            backup_file(&path, &paths.config_backup, stem)?;
            atomic_write(&path, text.as_bytes())?;
        }
        // 保留卸载状态，避免空镜像的首次导入重新分发到已卸载的客户端。
        database.save_mcp_server_record(Some(name), &record, &now_ms().to_string(), tool)
    })
}

impl AppContext {
    /// 读取 live config.toml；文件不存在视作空文档（首个 MCP 服务器创建前允许没有配置文件）。
    pub(super) fn read_live_config(&self) -> AppResult<String> {
        read_live_config(&self.paths)
    }

    fn codex_active_fragments(records: &[McpServerRecord]) -> Vec<(String, String)> {
        records
            .iter()
            .filter(|record| record.codex_installed && record.codex_enabled)
            .map(|record| (record.name.clone(), record.toml.clone()))
            .collect()
    }

    /// 强制替换数据库镜像的 Codex 活跃集（应用自身的管理操作走这里，允许清空到零）。
    pub(super) fn replace_mcp_mirror(&self, fragments: &[(String, String)]) -> AppResult<()> {
        let previous_records = self.database.mcp_server_records()?;
        if Self::codex_active_fragments(&previous_records) != fragments {
            self.database.replace_mcp_server_fragments_with(
                fragments,
                &now_ms().to_string(),
                |records| {
                    for record in records
                        .iter_mut()
                        .filter(|record| record.claude_installed && record.claude_json.is_none())
                    {
                        if let Some(previous) =
                            previous_records.iter().find(|old| old.name == record.name)
                        {
                            // 旧库尚无原生 JSON 时，先冻结旧快照，避免 Codex 更新改掉 Claude 基准。
                            record.claude_json =
                                Some(super::claude::claude_mcp_entry(previous, None)?.to_string());
                        }
                    }
                    Ok(())
                },
            )?;
        }
        Ok(())
    }

    /// 新供应商使用数据库镜像；首次尚无镜像时才读取 live MCP 段（同样滤除托管条目）。
    pub(super) fn mcp_document_for_new_profile(&self) -> AppResult<toml_edit::DocumentMut> {
        let records = self.database.mcp_server_records()?;
        let fragments = Self::codex_active_fragments(&records);
        if records.is_empty() {
            let live = codex_config::parse_document(&self.read_live_config()?)?;
            let live_fragments = codex_config::mcp_server_fragments_from_document(&live);
            let mut document = toml_edit::DocumentMut::new();
            codex_config::replace_mcp_section_from_fragments(&mut document, &live_fragments);
            return Ok(document);
        }
        let mut document = toml_edit::DocumentMut::new();
        codex_config::replace_mcp_section_from_fragments(&mut document, &fragments);
        Ok(document)
    }

    /// 把数据库镜像的 MCP 段写进 live config.toml；写前照常自动备份原文件。
    /// 建模字段与 live 一致的服务器保留 live 原文：注释/未建模键跟随 live，
    /// 恢复动作只覆盖真有差异的条目，不做整段格式回滚。
    ///
    /// live 解析失败时**只做文本层面 MCP 区域的重建**，区域外逐字节保留。
    /// 定位不到 MCP 区域就如实报错——整份重写会连带丢掉区域外的 projects /
    /// plugins / desktop 等全部配置，代价远高于收益，那种情况该走备份恢复。
    pub(super) fn write_mcp_section_to_live(
        &self,
        fragments: &[(String, String)],
    ) -> AppResult<()> {
        let live_text = self.read_live_config()?;
        let text = match codex_config::parse_document(&live_text) {
            Ok(mut document) => {
                let live_fragments = codex_config::mcp_server_fragments_from_document(&document);
                let live_specs: BTreeMap<String, McpServerSpec> =
                    codex_config::mcp_servers_from_document(&document)
                        .into_iter()
                        .map(|spec| (spec.name.clone(), spec))
                        .collect();
                let merged = fragments
                    .iter()
                    .map(|(name, toml)| {
                        let semantically_equal = live_specs.get(name).is_some_and(|live| {
                            codex_config::spec_from_fragment(name, toml)
                                .is_some_and(|db| *live == db)
                        });
                        if semantically_equal {
                            live_fragments
                                .iter()
                                .find(|(live_name, _)| live_name == name)
                                .cloned()
                                .unwrap_or_else(|| (name.clone(), toml.clone()))
                        } else {
                            (name.clone(), toml.clone())
                        }
                    })
                    .collect::<Vec<_>>();
                codex_config::replace_mcp_section_from_fragments(&mut document, &merged);
                codex_config::normalize_global_section_order(&document.to_string())
            }
            Err(_) => {
                let repaired = codex_config::rebuild_mcp_region_text(&live_text, fragments)
                    .ok_or_else(|| {
                        app_err!("config.toml 中定位不到 MCP 段，无法就地重建；请改用备份恢复")
                    })?;
                // 修完必须能解析才允许落盘：改不动就报错，不做"尽力而为地写下去"
                codex_config::parse_document(&repaired)
                    .map_err(|error| app_err!("重建 MCP 段后配置仍无法解析：{error}"))?;
                repaired
            }
        };
        let config_path = self.paths.codex_config();
        backup_file(&config_path, &self.paths.config_backup, "config")?;
        atomic_write(&config_path, text.as_bytes())?;
        Ok(())
    }

    /// 把改好的 live config.toml 文档落盘：备份一次 + 原子写入（含全局段排序归一）。
    fn write_live_config_document(&self, document: &toml_edit::DocumentMut) -> AppResult<()> {
        let config_path = self.paths.codex_config();
        backup_file(&config_path, &self.paths.config_backup, "config")?;
        atomic_write(
            &config_path,
            codex_config::normalize_global_section_order(&document.to_string()).as_bytes(),
        )
    }

    /// 数据库镜像写回 live config.toml（备份恢复后调用；旧备份无 MCP 表则不动 live）。
    pub(super) fn write_mcp_to_live_from_database(
        &self,
        previous: &[McpServerRecord],
    ) -> AppResult<()> {
        let records = self.database.mcp_server_records()?;
        self.write_mcp_section_to_live(&Self::codex_active_fragments(&records))?;
        self.sync_claude_mcp_projection(previous, &records)
    }

    /// 读取 live config.toml 中的全部 MCP 服务器（只读，不随供应商切换）。
    pub fn codex_list_mcp_servers(&self) -> AppResult<Vec<McpServerSpec>> {
        let document = codex_config::parse_document(&self.read_live_config()?)?;
        let mut servers = codex_config::mcp_servers_from_document(&document);
        for record in self.database.mcp_server_records()? {
            if !record.codex_installed || record.codex_enabled {
                continue;
            }
            let name = record.name.as_str();
            if let Some(server) = servers.iter_mut().find(|server| server.name == name) {
                server.enabled = Some(false);
                continue;
            }
            if let Some(mut server) = codex_config::spec_from_fragment(name, &record.toml) {
                server.enabled = Some(false);
                servers.push(server);
            }
        }
        Ok(servers)
    }

    /// 读取指定 MCP 服务器的原始片段（含未建模键与注释；编辑页初始化编辑器用）。
    pub fn codex_mcp_server_toml(&self, name: &str) -> AppResult<Option<String>> {
        let document = codex_config::parse_document(&self.read_live_config()?)?;
        if let Some(fragment) = codex_config::mcp_server_fragments_from_document(&document)
            .into_iter()
            .find(|(fragment_name, _)| fragment_name == name)
            .map(|(_, toml)| toml)
        {
            return Ok(Some(fragment));
        }
        Ok(self
            .database
            .mcp_server_record(name)?
            .filter(|record| record.codex_installed && !record.codex_enabled)
            .map(|record| record.toml))
    }

    /// 对比 live config.toml 与数据库镜像的 MCP 差异，供同步前人工裁决。
    /// 首次进入（该端在镜像里没有基线）时先把现场静默导入镜像。
    /// live 无法解析时返回错误，前端进入“仅可从数据库恢复”降级模式。
    pub fn codex_mcp_sync_preview(&self) -> AppResult<McpSyncPreview> {
        self.mcp_sync_preview(SkillTool::Codex)
    }

    pub fn mcp_sync_preview(&self, tool: SkillTool) -> AppResult<McpSyncPreview> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let mut records = self.database.mcp_server_records()?;
        // 首次进入该端：现场静默成为基线，避免"库为空 ⇒ 全是新增"的必然差异。
        if self.establish_mcp_baseline(tool, &records)? {
            records = self.database.mcp_server_records()?;
        }
        // 每侧只保存一份名称 -> (展示片段, 建模字段, Claude JSON)，不另建索引。
        let (live, db) = match tool {
            SkillTool::Codex => {
                let document = codex_config::parse_document(&self.read_live_config()?)?;
                let convert = |fragments: Vec<(String, String)>| -> BTreeMap<_, _> {
                    fragments
                        .into_iter()
                        .filter(|(name, _)| !codex_config::is_managed_mcp_name(name))
                        .map(|(name, raw)| {
                            let spec = codex_config::spec_from_fragment(&name, &raw);
                            (name, (raw, spec, None))
                        })
                        .collect()
                };
                (
                    convert(codex_config::mcp_server_fragments_from_document(&document)),
                    convert(Self::codex_active_fragments(&records)),
                )
            }
            SkillTool::Claude => {
                let document = self.read_claude_mcp_document()?;
                let convert = |name: &str, entry: &serde_json::Value| -> AppResult<_> {
                    let spec = super::claude::claude_entry_to_spec(name, entry)?;
                    // 复用投影规则比较 JSON，键顺序和缺省字段不会产生伪差异。
                    let normalized = super::claude::normalize_claude_mcp_entry(name, entry)?;
                    let raw = serde_json::to_string_pretty(entry)
                        .map_err(|error| app_err!("Claude MCP JSON 序列化失败: {error}"))?;
                    Ok((name.to_owned(), (raw, Some(spec), Some(normalized))))
                };
                let live: BTreeMap<_, _> = document
                    .get("mcpServers")
                    .and_then(serde_json::Value::as_object)
                    .into_iter()
                    .flat_map(|servers| servers.iter())
                    .filter(|(name, _)| !codex_config::is_managed_mcp_name(name))
                    .map(|(name, entry)| convert(name, entry))
                    .collect::<AppResult<_>>()?;
                let db: BTreeMap<_, _> = records
                    .iter()
                    .filter(|record| {
                        record.claude_installed
                            && record.claude_enabled
                            && !codex_config::is_managed_mcp_name(&record.name)
                    })
                    .map(|record| {
                        convert(
                            &record.name,
                            &super::claude::claude_mcp_entry(record, None)?,
                        )
                    })
                    .collect::<AppResult<_>>()?;
                (live, db)
            }
        };
        let mut entries = Vec::new();
        for (name, (live_raw, live_spec, live_json)) in &live {
            let Some((db_raw, db_spec, db_json)) = db.get(name) else {
                entries.push(McpSyncDiffEntry {
                    name: name.clone(),
                    kind: McpSyncEntryKind::LiveOnly,
                    live_spec: live_spec.clone(),
                    db_spec: None,
                    live_toml: Some(live_raw.clone()),
                    db_toml: None,
                });
                continue;
            };
            // Codex 沿用建模字段比较；Claude 比较完整 JSON，包含原生扩展字段。
            if (tool == SkillTool::Claude && live_json == db_json)
                || without_blank_lines(live_raw) == without_blank_lines(db_raw)
                || (tool == SkillTool::Codex && live_spec.is_some() && live_spec == db_spec)
            {
                continue;
            }
            entries.push(McpSyncDiffEntry {
                name: name.clone(),
                kind: McpSyncEntryKind::Changed,
                live_spec: live_spec.clone(),
                db_spec: db_spec.clone(),
                live_toml: Some(live_raw.clone()),
                db_toml: Some(db_raw.clone()),
            });
        }
        for (name, (db_raw, db_spec, _)) in &db {
            if !live.contains_key(name) {
                entries.push(McpSyncDiffEntry {
                    name: name.clone(),
                    kind: McpSyncEntryKind::DbOnly,
                    live_spec: None,
                    db_spec: db_spec.clone(),
                    live_toml: None,
                    db_toml: Some(db_raw.clone()),
                });
            }
        }
        Ok(McpSyncPreview {
            entries,
            live_count: live.len(),
            db_count: db.len(),
        })
    }

    /// 首次进入某端 MCP 页（该端在镜像里没有任何安装行）时，把现场静默导入为基线：
    /// 只写镜像、不碰 live，避免"库为空 ⇒ 全是新增"的必然差异。残缺条目跳过，
    /// 留给随后的正常差异计算暴露。返回是否建立了基线。
    fn establish_mcp_baseline(
        &self,
        tool: SkillTool,
        records: &[McpServerRecord],
    ) -> AppResult<bool> {
        if !records.iter().all(|record| match tool {
            SkillTool::Codex => !record.codex_installed,
            SkillTool::Claude => !record.claude_installed,
        }) {
            return Ok(false);
        }
        let (imported, skipped) = match tool {
            SkillTool::Codex => {
                let document = codex_config::parse_document(&self.read_live_config()?)?;
                let (fragments, invalid): (Vec<_>, Vec<_>) =
                    codex_config::mcp_server_fragments_from_document(&document)
                        .into_iter()
                        .filter(|(name, _)| !codex_config::is_managed_mcp_name(name))
                        .partition(|(name, fragment)| {
                            codex_config::spec_from_fragment(name, fragment)
                                .is_some_and(|spec| validate_mcp_connection(&spec).is_ok())
                        });
                let imported = fragments.len();
                if imported > 0 {
                    self.replace_mcp_mirror(&fragments)?;
                }
                (imported, invalid.len())
            }
            SkillTool::Claude => {
                let document = self.read_claude_mcp_document()?;
                let Some(servers) = document
                    .get("mcpServers")
                    .and_then(serde_json::Value::as_object)
                else {
                    return Ok(false);
                };
                let mut imported = 0;
                let mut skipped = 0;
                for (name, entry) in servers {
                    if codex_config::is_managed_mcp_name(name) {
                        continue;
                    }
                    if super::claude::claude_entry_to_spec(name, entry).is_err() {
                        skipped += 1;
                        continue;
                    }
                    let mut record =
                        self.database
                            .mcp_server_record(name)?
                            .unwrap_or_else(|| McpServerRecord {
                                name: name.clone(),
                                toml: String::new(),
                                codex_enabled: false,
                                claude_enabled: false,
                                codex_installed: false,
                                claude_installed: false,
                                claude_json: None,
                            });
                    record.claude_json = Some(entry.to_string());
                    record.claude_installed = true;
                    record.claude_enabled = true;
                    self.database.save_mcp_server_record(
                        Some(name),
                        &record,
                        &now_ms().to_string(),
                        SkillTool::Claude,
                    )?;
                    imported += 1;
                }
                (imported, skipped)
            }
        };
        if imported == 0 {
            return Ok(false);
        }
        tauri_plugin_log::log::info!(
            "[mcp.baseline] client={} count={imported} skipped={skipped} outcome=success msg=\"首次进入，现场 MCP 已导入为镜像基线\"",
            match tool {
                SkillTool::Codex => "Codex",
                SkillTool::Claude => "Claude Code",
            }
        );
        Ok(true)
    }

    /// 用户显式操作：数据库镜像写回 live config.toml（配置损坏/段丢失后的恢复）。
    /// 返回恢复的服务器数量。
    pub fn restore_mcp_from_database(&self) -> AppResult<usize> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let records = self.database.mcp_server_records()?;
        if !records.iter().any(|record| record.codex_installed) {
            return Err(app_err!("数据库中没有 MCP 镜像可恢复"));
        }
        let fragments = Self::codex_active_fragments(&records);
        let count = fragments.len();
        self.write_mcp_section_to_live(&fragments)?;
        tauri_plugin_log::log::info!(
            "[mcp.config.restore] outcome=success count={count} msg=\"已从数据库恢复 MCP 配置\""
        );
        Ok(count)
    }

    /// 创建表单预填用：优先数据库 MCP 镜像，首次无镜像时回退 live。
    pub fn codex_mcp_section_toml(&self) -> AppResult<String> {
        Ok(
            codex_config::mcp_server_fragments_from_document(&self.mcp_document_for_new_profile()?)
                .into_iter()
                .map(|(_, toml)| toml)
                .collect(),
        )
    }

    /// 新增/编辑/重命名一个 MCP 服务器：就地修改 live config.toml，未建模键与注释原样保留；
    /// 激活供应商的快照在下次 get_state 时自动吸收（与地址/密钥回写 live 同机制）。
    pub fn codex_save_mcp_server(
        &self,
        original_name: Option<&str>,
        spec: McpServerSpec,
    ) -> AppResult<()> {
        self.save_mcp_server_with_fragment(original_name, spec, None)
    }

    /// 编辑页保存：fragment = 编辑器当前片段。有片段时以它整表替换 live 里的该服务器
    /// （未建模键、注释与编辑器所见一致——所见即所得），建模字段先按 spec 补齐兜底；
    /// 无片段（纯表单路径）退回就地 upsert。
    /// 编辑已关闭的 Codex 条目等同重新启用；Claude 的快照、开关与 live 文件不动。
    pub fn save_mcp_server_with_fragment(
        &self,
        original_name: Option<&str>,
        spec: McpServerSpec,
        fragment: Option<&str>,
    ) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;

        let name = spec.name.trim().to_string();
        if name.is_empty() {
            return Err(app_err!("MCP 名称不能为空"));
        }
        if name.len() > 64 {
            return Err(app_err!("MCP 名称过长（最多 64 字符）"));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            // 点号会让 [mcp_servers.a.b] 变成嵌套表，空格/引号等也会破坏键名
            return Err(app_err!("MCP 名称只能包含字母、数字、下划线和连字符"));
        }
        if codex_config::is_managed_mcp_name(&name)
            || original_name.is_some_and(codex_config::is_managed_mcp_name)
        {
            // Codex 官方应用自动写入并维护（路径带版本哈希，每次更新都会变），编辑必被覆盖
            return Err(app_err!(
                "「{name}」由 Codex 官方应用自动管理，不能在本应用中创建或编辑"
            ));
        }
        if spec.startup_timeout_sec.is_some_and(|timeout| timeout <= 0) {
            return Err(app_err!("启动超时必须为正数（秒）"));
        }
        if spec.tool_timeout_sec.is_some_and(|timeout| timeout <= 0) {
            return Err(app_err!("工具调用超时必须为正数（秒）"));
        }
        validate_mcp_connection(&spec)?;

        let mut spec = spec;
        spec.name = name;
        let existing = self
            .database
            .mcp_server_record(original_name.unwrap_or(spec.name.as_str()))?;
        if existing
            .as_ref()
            .is_some_and(|record| !record.codex_enabled)
        {
            // 编辑关闭条目等同重新启用，不把列表开关写成 Codex 原生 enabled 键。
            spec.enabled = None;
        }
        let mut document = codex_config::parse_document(&self.read_live_config()?)?;
        let name_taken = document
            .as_table()
            .get("mcp_servers")
            .and_then(toml_edit::Item::as_table)
            .is_some_and(|servers| servers.contains_key(&spec.name));
        let destination = self.database.mcp_server_record(&spec.name)?;
        let name_conflict = name_taken
            || destination
                .as_ref()
                .is_some_and(|record| record.codex_installed);
        if original_name != Some(spec.name.as_str()) && name_conflict {
            return Err(app_err!("已存在同名 MCP 服务器"));
        }

        if let Some(original) = original_name.filter(|original| original != &spec.name) {
            let servers = document
                .as_table_mut()
                .get_mut("mcp_servers")
                .and_then(toml_edit::Item::as_table_mut);
            if let Some(servers) = servers {
                if let Some(table) = servers.remove(original) {
                    servers.insert(&spec.name, table);
                }
            }
            if existing
                .as_ref()
                .is_none_or(|record| !record.codex_installed)
                && document
                    .get("mcp_servers")
                    .and_then(|servers| servers.get(&spec.name))
                    .is_none()
            {
                return Err(app_err!("MCP 服务器 {original} 不存在"));
            }
        }

        if let Some(fragment) = fragment {
            // 片段路径：把建模字段补进片段后整表搬进 live，编辑器所见 = 保存所写
            let patched = codex_config::patch_mcp_fragment(fragment, &spec)?;
            let mut fragment_doc = codex_config::parse_document(&patched)?;
            let table = fragment_doc
                .as_table_mut()
                .get_mut("mcp_servers")
                .and_then(toml_edit::Item::as_table_mut)
                .and_then(|servers| servers.remove(spec.name.as_str()))
                .ok_or_else(|| app_err!("片段中没有可保存的服务器 {}", spec.name))?;
            document
                .as_table_mut()
                .entry("mcp_servers")
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .ok_or_else(|| app_err!("mcp_servers 不是 TOML table"))?
                .insert(spec.name.as_str(), table);
        } else {
            codex_config::upsert_mcp_server(&mut document, &spec)?;
        }
        let toml = codex_config::mcp_server_fragments_from_document(&document)
            .into_iter()
            .find(|(name, _)| name == &spec.name)
            .map(|(_, toml)| toml)
            .ok_or_else(|| app_err!("MCP 片段不存在"))?;
        let mut destination = destination;
        if let Some(record) = destination
            .as_mut()
            .filter(|record| record.claude_installed && record.claude_json.is_none())
        {
            record.claude_json = Some(super::claude::claude_mcp_entry(record, None)?.to_string());
        }
        let record = McpServerRecord {
            name: spec.name.clone(),
            toml,
            codex_enabled: true,
            claude_enabled: destination
                .as_ref()
                .is_some_and(|record| record.claude_enabled),
            codex_installed: true,
            claude_installed: destination
                .as_ref()
                .is_some_and(|record| record.claude_installed),
            claude_json: destination.and_then(|record| record.claude_json),
        };
        crate::fsutil::with_file_rollback(&[self.paths.codex_config()], || {
            self.write_live_config_document(&document)?;
            self.database.save_mcp_server_record(
                original_name,
                &record,
                &now_ms().to_string(),
                SkillTool::Codex,
            )
        })?;
        tauri_plugin_log::log::info!(
            "[mcp.config.save] server={:?} outcome=success msg=\"MCP 配置已保存\"",
            spec.name
        );
        Ok(())
    }

    /// 切换引擎级 MCP 开关：只影响该引擎的用户范围 live 文件（Codex=config.toml，Claude=.claude.json），
    /// 数据库片段保留；另一引擎的开关与 live 文件不动。项目级配置从不在这里改写。
    pub fn set_mcp_server_enabled(
        &self,
        name: &str,
        tool: SkillTool,
        enabled: bool,
    ) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        if codex_config::is_managed_mcp_name(name) {
            return Err(app_err!("「{name}」由 Codex 官方应用自动管理，不能切换"));
        }

        let existing = self.database.mcp_server_record(name)?;
        let has_record = existing.is_some();
        let mut record = if let Some(record) = existing {
            record
        } else {
            // 首次操作也保留另一端现场已有的开关和原生配置。
            mcp_record_from_live(&self.paths, name)?
        };
        if tool == SkillTool::Claude && record.claude_json.is_none() {
            record.claude_json = self
                .read_claude_mcp_document()?
                .get("mcpServers")
                .and_then(|servers| servers.get(name))
                .map(serde_json::Value::to_string);
        }
        let already = match tool {
            SkillTool::Codex => record.codex_enabled,
            SkillTool::Claude => record.claude_enabled,
        };
        let installed = match tool {
            SkillTool::Codex => record.codex_installed,
            SkillTool::Claude => record.claude_installed,
        };
        if !installed {
            // 用户在客户端外重新添加后，以该端现场为准；过期 UI 不能恢复已卸载条目。
            let live = mcp_record_from_live(&self.paths, name)?;
            match tool {
                SkillTool::Codex if live.codex_installed => {
                    record.codex_installed = true;
                    record.toml = live.toml;
                }
                SkillTool::Claude if live.claude_installed => {
                    record.claude_installed = true;
                    record.claude_json = live.claude_json;
                }
                _ => return Err(app_err!("MCP 服务器已在该客户端卸载: {name}")),
            }
        }
        // 开关已是要的状态时通常直接返回；唯一例外：Codex 开关是开、但片段里残留
        // 原生 enabled=false（导入/采纳带进来的）——UI 开关显示关，必须借这次点击
        // 走启用分支把键剥掉，否则用户点"打开"毫无反应。
        let native_disabled = tool == SkillTool::Codex
            && record.codex_enabled
            && codex_config::spec_from_fragment(name, &record.toml)
                .is_some_and(|spec| spec.enabled == Some(false));
        if has_record && installed && already == enabled && !(enabled && native_disabled) {
            return Ok(());
        }

        let target_path = match tool {
            SkillTool::Codex => self.paths.codex_config(),
            SkillTool::Claude => self.paths.claude_mcp_config(),
        };
        crate::fsutil::with_file_rollback(&[target_path], || {
            match (tool, enabled) {
                (SkillTool::Codex, true) => {
                    // 开关只控制条目是否存在；恢复启用时移除 Codex 原生 enabled 键，
                    // 避免历史遗留的 enabled=false 把刚恢复的条目再次停用。
                    let mut spec = codex_config::spec_from_fragment(name, &record.toml)
                        .ok_or_else(|| app_err!("MCP 服务器 {name} 的共享片段无法解析"))?;
                    spec.enabled = None;
                    let fragment = codex_config::patch_mcp_fragment(&record.toml, &spec)?;
                    let document = codex_config::parse_document(&self.read_live_config()?)?;
                    let mut live = codex_config::mcp_server_fragments_from_document(&document);
                    live.retain(|(server_name, _)| server_name != name);
                    live.push((name.to_string(), fragment.clone()));
                    self.write_mcp_section_to_live(&live)?;
                    record.toml = fragment;
                }
                (SkillTool::Codex, false) => {
                    let mut document = codex_config::parse_document(&self.read_live_config()?)?;
                    let present = codex_config::mcp_server_fragments_from_document(&document)
                        .iter()
                        .any(|(server_name, _)| server_name == name);
                    if present {
                        codex_config::remove_mcp_server(&mut document, name)?;
                        self.write_live_config_document(&document)?;
                    }
                }
                (SkillTool::Claude, true) => {
                    record.claude_enabled = true;
                    self.sync_claude_mcp_projection(&[], std::slice::from_ref(&record))?;
                }
                (SkillTool::Claude, false) => {
                    self.remove_claude_mcp_entry(name)?;
                }
            }
            match tool {
                SkillTool::Codex => record.codex_enabled = enabled,
                SkillTool::Claude => record.claude_enabled = enabled,
            }
            self.database
                .save_mcp_server_record(Some(name), &record, &now_ms().to_string(), tool)
        })?;
        tauri_plugin_log::log::info!(
            "[mcp.config.toggle] server={name:?} tool={:?} enabled={enabled} outcome=success msg=\"MCP 引擎开关已更新\"",
            tool
        );
        Ok(())
    }

    /// 差异处理"同步"原语（单条）：见 set_mcp_mirror_entries。
    pub fn set_mcp_mirror_entry(&self, name: &str, fragment: Option<&str>) -> AppResult<()> {
        self.set_mcp_mirror_entries(&[McpDiffEntryAction {
            name: name.to_string(),
            fragment: fragment.map(str::to_string),
        }])
        .map(|_| ())
    }

    /// 差异处理"同步"原语：把若干条目一次写进数据库镜像——fragment=Some 用 live 片段覆盖该条，
    /// fragment=None 删除该条（"外部已删除"的同步）。整批校验通过才落一次盘，任一条非法整批不写。
    /// 不改任何 live 文件或 Claude 快照。不能用 codex_save_mcp_server / codex_delete_mcp_server
    /// 代替，因为它们会同时修改 live，无法表达仅采纳外部差异。
    pub fn set_mcp_mirror_entries(&self, actions: &[McpDiffEntryAction]) -> AppResult<usize> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        if actions.is_empty() {
            return Ok(0);
        }
        let mut fragments = Self::codex_active_fragments(&self.database.mcp_server_records()?);
        for action in actions {
            let name = action.name.as_str();
            if codex_config::is_managed_mcp_name(name) {
                return Err(app_err!(
                    "「{name}」由 Codex 官方应用自动管理，不能改写数据库镜像"
                ));
            }
            fragments.retain(|(existing, _)| existing != name);
            if let Some(fragment) = &action.fragment {
                if codex_config::spec_from_fragment(name, fragment).is_none() {
                    return Err(app_err!("片段无法解析为 MCP 服务器 {name}"));
                }
                fragments.push((action.name.clone(), fragment.clone()));
            }
        }
        self.replace_mcp_mirror(&fragments)?;
        for action in actions.iter().filter(|action| action.fragment.is_none()) {
            if let Some(mut record) = self.database.mcp_server_record(&action.name)? {
                record.codex_installed = false;
                record.codex_enabled = false;
                self.database.save_mcp_server_record(
                    Some(&action.name),
                    &record,
                    &now_ms().to_string(),
                    SkillTool::Codex,
                )?;
            }
        }
        // 采纳（fragment=Some）外部加回 live 的条目 = 接受它在 Codex 端启用的事实：
        // 不翻开开关的话该行会卡在"live 有条目、开关却是关"的状态，差异也永远消不掉
        for action in actions.iter().filter(|action| action.fragment.is_some()) {
            if self
                .database
                .mcp_server_record(&action.name)?
                .is_some_and(|record| !record.codex_enabled)
            {
                self.database.set_codex_mcp_enabled(&action.name, true)?;
            }
        }
        tauri_plugin_log::log::info!(
            "[mcp.diff.batch] source=mirror count={} outcome=success msg=\"已把外部 MCP 修改写入数据库镜像\"",
            actions.len()
        );
        Ok(actions.len())
    }

    /// 差异处理"撤销"原语（单条）：见 revert_mcp_live_entries。
    pub fn revert_mcp_live_entry(&self, name: &str, fragment: Option<&str>) -> AppResult<()> {
        self.revert_mcp_live_entries(&[McpDiffEntryAction {
            name: name.to_string(),
            fragment: fragment.map(str::to_string),
        }])
        .map(|_| ())
    }

    /// 差异处理"撤销"原语：把若干条目一次写回 live config.toml——fragment=Some 恢复为数据库内容，
    /// fragment=None 从 live 移除。逐条原地改写：只动列出的条目，其余条目、其他配置段、
    /// 段内既有顺序与文件布局全部原样保留。整批改完才备份并写一次盘，任一条失败整批不写
    /// （逐条调用会各备份一次，把保留池里操作前的备份挤掉）。
    /// 不能走 write_mcp_section_to_live——那是整段替换语义，会清掉 live 里其他服务器并重排段内条目。
    pub fn revert_mcp_live_entries(&self, actions: &[McpDiffEntryAction]) -> AppResult<usize> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        if actions.is_empty() {
            return Ok(0);
        }
        let mut document = codex_config::parse_document(&self.read_live_config()?)?;
        for action in actions {
            let name = action.name.as_str();
            if codex_config::is_managed_mcp_name(name) {
                return Err(app_err!("「{name}」由 Codex 官方应用自动管理，不能回退"));
            }
            let Some(fragment) = &action.fragment else {
                codex_config::remove_mcp_server(&mut document, name)?;
                continue;
            };
            // 恢复 = 把数据库片段合并进现有 live 文档：只动这一个条目，其余条目原样保留
            let mut fragment_doc = codex_config::parse_document(fragment)?;
            let table = fragment_doc
                .as_table_mut()
                .get_mut("mcp_servers")
                .and_then(toml_edit::Item::as_table_mut)
                .and_then(|servers| servers.remove(name))
                .ok_or_else(|| app_err!("片段中没有可恢复的服务器 {name}"))?;
            document
                .as_table_mut()
                .entry("mcp_servers")
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .ok_or_else(|| app_err!("mcp_servers 不是 TOML table"))?
                .insert(name, table);
        }
        self.write_live_config_document(&document)?;
        tauri_plugin_log::log::info!(
            "[mcp.diff.batch] source=live count={} outcome=success msg=\"已把数据库 MCP 配置写回 live\"",
            actions.len()
        );
        Ok(actions.len())
    }

    /// Codex 页的卸载入口；Claude 页显式选择自己的客户端。
    pub fn codex_delete_mcp_server(&self, name: &str) -> AppResult<()> {
        self.delete_mcp_server_for_tool(name, SkillTool::Codex)
    }

    pub(super) fn delete_mcp_server_for_tool(&self, name: &str, tool: SkillTool) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        uninstall_mcp_server(&self.database, &self.paths, name, tool)?;
        tauri_plugin_log::log::info!(
            "[mcp.config.delete] server={name:?} tool={tool:?} outcome=success msg=\"MCP 已从当前客户端卸载\""
        );
        Ok(())
    }
}

#[cfg(test)]
mod uninstall_tests {
    use super::*;

    #[test]
    fn uninstall_is_client_local_for_live_and_disabled_entries_with_or_without_mirror() {
        for tool in [SkillTool::Codex, SkillTool::Claude] {
            for mirrored in [false, true] {
                for disabled in [false, true] {
                    if disabled && !mirrored {
                        continue;
                    }
                    let home = tempfile::tempdir().unwrap();
                    let paths = crate::paths::from_home(home.path()).unwrap();
                    paths.ensure().unwrap();
                    std::fs::create_dir_all(&paths.codex_home).unwrap();
                    let database = Database::open(&paths).unwrap();
                    let toml = "# keep\nmodel=\"fixture\"\n[mcp_servers.fixture]\ncommand=\"fixture\"\ncustom=\"keep\"\n[mcp_servers.other]\ncommand=\"other\"\n";
                    let json = r#"{"other":"keep","mcpServers":{"fixture":{"type":"stdio","command":"fixture","custom":1},"other":{"type":"stdio","command":"other"}},"projects":{"/fixture":{"mcpServers":{"fixture":{"command":"project"}}}}}"#;
                    std::fs::write(paths.codex_config(), toml).unwrap();
                    std::fs::write(paths.claude_mcp_config(), json).unwrap();
                    if mirrored {
                        database
                            .save_mcp_server_record(
                                None,
                                &mcp_record_from_live(&paths, "fixture").unwrap(),
                                "1",
                                tool,
                            )
                            .unwrap();
                    }
                    let (target, other) = match tool {
                        SkillTool::Codex => (paths.codex_config(), paths.claude_mcp_config()),
                        SkillTool::Claude => (paths.claude_mcp_config(), paths.codex_config()),
                    };
                    if disabled {
                        match tool {
                            SkillTool::Codex => {
                                database.set_codex_mcp_enabled("fixture", false).unwrap();
                                std::fs::write(
                                    &target,
                                    "model=\"fixture\"\n[mcp_servers.other]\ncommand=\"other\"\n",
                                )
                                .unwrap();
                            }
                            SkillTool::Claude => {
                                database.set_claude_mcp_enabled("fixture", false).unwrap();
                                std::fs::write(&target, r#"{"other":"keep","mcpServers":{"other":{"command":"other"}}}"#).unwrap();
                            }
                        }
                    }
                    let other_before = std::fs::read(&other).unwrap();
                    uninstall_mcp_server(&database, &paths, "fixture", tool).unwrap();
                    assert_eq!(std::fs::read(&other).unwrap(), other_before);
                    let record = database.mcp_server_record("fixture").unwrap().unwrap();
                    match tool {
                        SkillTool::Codex => {
                            assert!(!record.codex_installed && !record.codex_enabled);
                            assert!(record.claude_installed && record.claude_enabled);
                            let document =
                                codex_config::parse_document(&read_live_config(&paths).unwrap())
                                    .unwrap();
                            assert!(document["mcp_servers"].get("fixture").is_none());
                            assert!(document["mcp_servers"].get("other").is_some());
                        }
                        SkillTool::Claude => {
                            assert!(!record.claude_installed && !record.claude_enabled);
                            assert!(record.codex_installed && record.codex_enabled);
                            let document =
                                super::super::claude::read_claude_mcp_document(&paths).unwrap();
                            assert!(document["mcpServers"].get("fixture").is_none());
                            assert!(document["mcpServers"].get("other").is_some());
                            if !disabled {
                                assert_eq!(
                                    document["projects"]["/fixture"]["mcpServers"]["fixture"]
                                        ["command"],
                                    "project"
                                );
                            }
                        }
                    }
                    // 全量镜像刷新、备份恢复都保留卸载状态。
                    let fragments = database.mcp_server_fragments().unwrap();
                    database
                        .replace_mcp_server_fragments(&fragments, "2")
                        .unwrap();
                    let backup = home.path().join("backup.db");
                    database.export_database(&backup).unwrap();
                    database.restore_from_backup(&backup).unwrap();
                    let after = database.mcp_server_record("fixture").unwrap().unwrap();
                    assert_eq!(after.codex_installed, record.codex_installed);
                    assert_eq!(after.claude_installed, record.claude_installed);
                    assert!(uninstall_mcp_server(&database, &paths, "fixture", tool).is_err());
                }
            }
        }
    }

    #[test]
    fn uninstall_does_not_depend_on_other_client_parsing_and_rolls_back_failed_database_write() {
        for tool in [SkillTool::Codex, SkillTool::Claude] {
            let home = tempfile::tempdir().unwrap();
            let paths = crate::paths::from_home(home.path()).unwrap();
            paths.ensure().unwrap();
            std::fs::create_dir_all(&paths.codex_home).unwrap();
            let database = Database::open(&paths).unwrap();
            std::fs::write(
                paths.codex_config(),
                "[mcp_servers.fixture]\ncommand=\"fixture\"\n",
            )
            .unwrap();
            std::fs::write(
                paths.claude_mcp_config(),
                r#"{"mcpServers":{"fixture":{"command":"fixture"}}}"#,
            )
            .unwrap();
            database
                .save_mcp_server_record(
                    None,
                    &mcp_record_from_live(&paths, "fixture").unwrap(),
                    "1",
                    tool,
                )
                .unwrap();
            let (target, other, source) = match tool {
                SkillTool::Codex => (
                    paths.codex_config(),
                    paths.claude_mcp_config(),
                    "[mcp_servers.fixture]\ncommand=\"fixture\"\n",
                ),
                SkillTool::Claude => (
                    paths.claude_mcp_config(),
                    paths.codex_config(),
                    r#"{"mcpServers":{"fixture":{"command":"fixture"}}}"#,
                ),
            };
            std::fs::write(&target, source).unwrap();
            std::fs::write(&other, "malformed {").unwrap();
            let connection = rusqlite::Connection::open(&paths.database).unwrap();
            connection.execute_batch("CREATE TRIGGER reject_uninstall BEFORE UPDATE ON mcp_servers BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
            assert!(uninstall_mcp_server(&database, &paths, "fixture", tool).is_err());
            assert_eq!(std::fs::read_to_string(&target).unwrap(), source);
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "malformed {");
            let record = database.mcp_server_record("fixture").unwrap().unwrap();
            assert!(record.codex_installed && record.claude_installed);
            connection
                .execute_batch("DROP TRIGGER reject_uninstall")
                .unwrap();
            uninstall_mcp_server(&database, &paths, "fixture", tool).unwrap();
            assert_eq!(std::fs::read_to_string(&other).unwrap(), "malformed {");
        }
    }
}
