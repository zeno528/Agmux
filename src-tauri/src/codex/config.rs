use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use toml_edit::{Array, Decor, DocumentMut, Item, Table, Value};

use crate::error::{app_err, AppResult};
use crate::models::{CodexProfilePayload, McpServerSpec};

pub fn parse_document(text: &str) -> AppResult<DocumentMut> {
    text.parse::<DocumentMut>()
        .map_err(|error| app_err!("Codex 配置不是有效 TOML: {error}"))
}

/// 从 Agmux 快照中移除 Codex 自己管理的 MCP，避免重启时把插件状态写回 live。
pub fn without_managed_mcp_servers(text: &str) -> AppResult<String> {
    let mut document = parse_document(text)?;
    remove_managed_mcp_servers(&mut document);
    Ok(document.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct TomlDiagnostic {
    pub from: usize,
    pub to: usize,
    pub message: String,
}

fn utf16_offset(text: &str, byte_offset: usize) -> usize {
    text.get(..byte_offset)
        .unwrap_or(text)
        .encode_utf16()
        .count()
}

/// 校验 TOML 文本，返回全部语法错误的 UTF-16 偏移区间（CodeMirror 以 UTF-16 定位文档）。
/// taplo 解析器自带错误恢复，一次解析即可拿全所有错误；上限 100 条防御异常输入刷屏。
pub fn validate_document(text: &str) -> Vec<TomlDiagnostic> {
    let errors: Vec<_> = taplo::parser::parse(text)
        .errors
        .into_iter()
        .take(100)
        .map(|error| {
            (
                usize::from(error.range.start()).min(text.len()),
                usize::from(error.range.end()).min(text.len()),
                error.message,
            )
        })
        .collect();

    // 相接/重叠的错误链合并为一条：单点错误（如字符串缺闭合引号）会让恢复式解析
    // 在后续文本上报一串连锁错误；独立错误之间必有间隙，不受影响。
    let mut merged: Vec<(usize, usize, String)> = Vec::new();
    for (start, end, message) in errors {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 + 1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end, message));
    }

    merged
        .into_iter()
        .map(|(from, to, message)| TomlDiagnostic {
            from: utf16_offset(text, from),
            to: utf16_offset(text, to),
            message,
        })
        .collect()
}

/// 格式化 TOML 文本；taplo 会跳过含语法错误的区间，坏文档也能尽量保持可格式化。
pub fn format_document(text: &str) -> String {
    taplo::formatter::format(text, taplo::formatter::Options::default())
}

pub fn patch_context_override(
    text: &str,
    enabled: bool,
    compact_token_limit: i64,
) -> AppResult<String> {
    let mut document = parse_document(text)?;
    if enabled {
        if !(1..=1_000_000).contains(&compact_token_limit) {
            return Err(app_err!("压缩阈值必须在 1 到 1000000 Token 之间"));
        }
        document.as_table_mut().insert(
            "model_context_window",
            Item::Value(Value::from(1_000_000_i64)),
        );
        document.as_table_mut().insert(
            "model_auto_compact_token_limit",
            Item::Value(Value::from(compact_token_limit)),
        );
    } else {
        document.as_table_mut().remove("model_context_window");
        document
            .as_table_mut()
            .remove("model_auto_compact_token_limit");
    }
    Ok(document.to_string())
}

/// features 表：不存在则创建；"features 不是 TOML table" 的报错两个 patch 共用。
fn features_table_mut(document: &mut DocumentMut) -> AppResult<&mut Table> {
    document
        .as_table_mut()
        .entry("features")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| app_err!("features 不是 TOML table"))
}

pub fn patch_system_proxy(text: &str, enabled: bool) -> AppResult<String> {
    let mut document = parse_document(text)?;
    if enabled {
        features_table_mut(&mut document)?
            .insert("respect_system_proxy", Item::Value(Value::from(true)));
    } else if let Some(features) = document
        .as_table_mut()
        .get_mut("features")
        .and_then(Item::as_table_mut)
    {
        features.remove("respect_system_proxy");
    }
    Ok(document.to_string())
}

pub fn patch_context_management(text: &str, enabled: bool) -> AppResult<String> {
    let mut document = parse_document(text)?;
    if enabled {
        let context_management = features_table_mut(&mut document)?
            .entry("context_management")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| app_err!("features.context_management 不是 TOML table"))?;
        context_management.insert("experimental_mode", Item::Value(Value::from(true)));
    } else if let Some(features) = document
        .as_table_mut()
        .get_mut("features")
        .and_then(Item::as_table_mut)
    {
        features.remove("context_management");
    }
    Ok(document.to_string())
}

pub fn read_profile(path: &Path) -> AppResult<CodexProfilePayload> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| app_err!("无法读取 {}: {error}", path.display()))?;
    capture_from_document(&parse_document(&text)?)
}

pub fn capture_from_document(document: &DocumentMut) -> AppResult<CodexProfilePayload> {
    let mut model_values = BTreeMap::new();
    for (key, item) in document.as_table().iter() {
        if is_model_key(key) && item.is_value() {
            // 清掉源码装饰（前导空格、行内注释等）只留纯 "value" 形式：
            // apply 侧 parse_value 依赖它 re-parse，前端显示依赖 stripTomlQuotes 剥引号
            let mut value = item.as_value().expect("is_value 已检查").clone();
            *value.decor_mut() = Decor::default();
            model_values.insert(key.to_string(), value.to_string());
        }
    }

    let provider_id = document
        .as_table()
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::to_string);
    let provider_body = provider_id.as_deref().and_then(|id| {
        document
            .as_table()
            .get("model_providers")
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(id))
            .and_then(Item::as_table)
            .map(Table::to_string)
    });

    Ok(CodexProfilePayload {
        model_values,
        provider_id,
        provider_body,
        builtin: None,
        ..Default::default()
    })
}

pub fn apply_to_document(
    document: &mut DocumentMut,
    payload: &CodexProfilePayload,
) -> AppResult<()> {
    let stale_keys: Vec<String> = document
        .as_table()
        .iter()
        .filter(|(key, item)| is_model_key(key) && item.is_value())
        .map(|(key, _)| key.to_string())
        .collect();
    for key in stale_keys {
        document.as_table_mut().remove(&key);
    }

    for (key, raw) in &payload.model_values {
        let value = parse_value(raw)?;
        document.as_table_mut().insert(key, Item::Value(value));
    }

    if let (Some(provider_id), Some(provider_body)) = (&payload.provider_id, &payload.provider_body)
    {
        let providers = document
            .as_table_mut()
            .entry("model_providers")
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| app_err!("model_providers 不是 TOML table"))?;
        providers.remove(provider_id);

        let parsed_body: DocumentMut = provider_body
            .parse()
            .map_err(|_| app_err!("供应商配置中的 provider 数据无效"))?;
        let mut provider = Table::new();
        for (key, item) in parsed_body.as_table() {
            let item = item.clone();
            provider.insert(key, item);
        }
        providers.insert(provider_id, Item::Table(provider));
    }

    Ok(())
}

fn is_model_key(key: &str) -> bool {
    key == "model"
        || key == "service_tier"
        || (key.starts_with("model_") && key != "model_providers")
}

fn parse_value(raw: &str) -> AppResult<Value> {
    raw.trim()
        .parse::<Value>()
        .map_err(|_| app_err!("供应商配置中的模型值无效"))
}

pub fn update_provider_body(
    body: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> AppResult<String> {
    let mut document: DocumentMut = body
        .parse()
        .map_err(|_| app_err!("供应商配置中的 provider 数据无效"))?;
    if let Some(value) = base_url {
        set_table_value(&mut document, "base_url", value);
    }
    if let Some(value) = api_key {
        set_table_value(&mut document, "experimental_bearer_token", value);
    }
    Ok(document.to_string())
}

/// 在已解析的 live 配置文档中就地更新 provider 表的 base_url / 密钥字段。
pub fn update_provider_in_document(
    document: &mut DocumentMut,
    provider_id: &str,
    base_url: Option<&str>,
    api_key: Option<&str>,
) -> AppResult<()> {
    let providers = document
        .as_table_mut()
        .entry("model_providers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| app_err!("model_providers 不是 TOML table"))?;
    let provider = providers
        .entry(provider_id)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| app_err!("model_providers.{provider_id} 不是 TOML table"))?;

    let set = |table: &mut Table, key: &str, value: Option<&str>| {
        if let Some(value) = value {
            if value.trim().is_empty() {
                table.remove(key);
            } else {
                table.insert(key, Item::Value(Value::from(value.trim().to_string())));
            }
        }
    };
    set(provider, "base_url", base_url);
    set(provider, "experimental_bearer_token", api_key);
    Ok(())
}

fn set_table_value(document: &mut DocumentMut, key: &str, value: &str) {
    if value.trim().is_empty() {
        document.remove(key);
    } else {
        let parsed = Value::from(value.trim().to_string());
        document.insert(key, Item::Value(parsed));
    }
}

// ===== MCP 服务器段（[mcp_servers.*]）=====
// live config.toml 是 MCP 的唯一事实源：读取只取建模字段子集，
// 写入就地编辑，未建模键（tools.*、cwd、注释等）原样保留。

/// Codex/ChatGPT 桌面应用自动管理的 MCP 条目（内置 Computer-Use 与 REPL 运行时）：
/// 应用自行写入 config.toml、删除后自动重建，且应用更新会改写其中的路径。这类
/// 条目不属于用户配置——列表/差异对比/数据库镜像全部跳过，重建 MCP 段时原样保留。
pub const MANAGED_MCP_SERVERS: &[&str] = &["node_repl", "computer-use", "cua_repl"];

/// 名称是否为 Codex 官方应用托管的 MCP 条目。
pub fn is_managed_mcp_name(name: &str) -> bool {
    MANAGED_MCP_SERVERS.contains(&name)
}

pub fn remove_managed_mcp_servers(document: &mut DocumentMut) {
    let remove_section = document
        .as_table_mut()
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .map(|servers| {
            for name in MANAGED_MCP_SERVERS {
                servers.remove(name);
            }
            servers.is_empty()
        })
        .unwrap_or(false);
    if remove_section {
        document.as_table_mut().remove("mcp_servers");
    }
}

/// 读取 [mcp_servers.*] 下全部服务器（按文件顺序，跳过 Codex 托管条目）；段缺失返回空列表。
pub fn mcp_servers_from_document(document: &DocumentMut) -> Vec<McpServerSpec> {
    let Some(servers) = document
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table)
    else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|(name, item)| item.as_table().map(|table| (name, table)))
        .filter(|(name, _)| !is_managed_mcp_name(name))
        .map(|(name, table)| McpServerSpec {
            name: name.to_string(),
            enabled: boolean_of(table, "enabled"),
            startup_timeout_sec: integer_of(table, "startup_timeout_sec"),
            tool_timeout_sec: integer_of(table, "tool_timeout_sec"),
            command: string_of(table, "command"),
            args: table
                .get("args")
                .and_then(Item::as_value)
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            env: string_map(table.get("env")),
            url: string_of(table, "url"),
            bearer_token_env_var: string_of(table, "bearer_token_env_var"),
            http_headers: string_map(table.get("http_headers")),
            env_http_headers: string_map(table.get("env_http_headers")),
        })
        .collect()
}

/// 就地写入一个服务器：建模字段按 spec 设置（None/空 = 移除键），未建模字段原样保留；
/// env / http_headers / env_http_headers 三个子表按 map 全量重建。
pub fn upsert_mcp_server(document: &mut DocumentMut, spec: &McpServerSpec) -> AppResult<()> {
    let servers = document
        .as_table_mut()
        .entry("mcp_servers")
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| app_err!("mcp_servers 不是 TOML table"))?;
    let server = servers
        .entry(&spec.name)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| app_err!("mcp_servers.{} 不是 TOML table", spec.name))?;

    let set_string = |table: &mut Table, key: &str, value: &Option<String>| match value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => {
            replace_value(table, key, Value::from(value.to_string()));
        }
        None => {
            table.remove(key);
        }
    };
    let set_map = |table: &mut Table, key: &str, map: &BTreeMap<String, String>| {
        if map.is_empty() {
            table.remove(key);
        } else {
            let mut sub = Table::new();
            for (name, value) in map {
                sub.insert(name, Item::Value(Value::from(value.clone())));
            }
            replace_table(table, key, sub);
        }
    };

    set_typed_value(server, "enabled", spec.enabled);
    set_typed_value(server, "startup_timeout_sec", spec.startup_timeout_sec);
    set_typed_value(server, "tool_timeout_sec", spec.tool_timeout_sec);
    // 标量在前、子表在后：新建条目按 [mcp_servers.x] + [mcp_servers.x.env] 的常规顺序渲染
    set_string(server, "command", &spec.command);
    if spec.args.is_empty() {
        server.remove("args");
    } else {
        let mut args = Array::new();
        for arg in &spec.args {
            args.push(arg.clone());
        }
        replace_value(server, "args", Value::Array(args));
    }
    set_string(server, "url", &spec.url);
    set_string(server, "bearer_token_env_var", &spec.bearer_token_env_var);
    set_map(server, "env", &spec.env);
    set_map(server, "http_headers", &spec.http_headers);
    set_map(server, "env_http_headers", &spec.env_http_headers);
    Ok(())
}

/// 删除一个服务器（连同其全部子表）；不存在时报错。
pub fn remove_mcp_server(document: &mut DocumentMut, name: &str) -> AppResult<()> {
    let removed = document
        .as_table_mut()
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .and_then(|servers| servers.remove(name));
    if removed.is_none() {
        return Err(app_err!("MCP 服务器 {name} 不存在"));
    }
    Ok(())
}

/// 把 source（live）的 [mcp_servers] 段整体搬到 target：live 有则覆盖（indexmap 语义下
/// 同键原位替换，段落位置不动；Item::clone 保留注释等装饰），live 无则移除 target 既有段
/// （全局模型：切换供应商不增删 MCP，只跟随 live）。
pub fn replace_mcp_section(target: &mut DocumentMut, source: &DocumentMut) {
    match source
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table)
        .filter(|servers| !servers.is_empty())
    {
        Some(servers) => {
            target
                .as_table_mut()
                .insert("mcp_servers", Item::Table(servers.clone()));
        }
        None => {
            target.as_table_mut().remove("mcp_servers");
        }
    }
}

/// 把 source（live）的插件运行时段整体搬到 target。
/// 插件市场、已安装插件和插件 hooks 都是全局 Codex 状态，不属于供应商配置，
/// 切换供应商时必须跟随 live 配置；live 没有时移除 target 的陈旧状态。
pub fn replace_plugin_sections(target: &mut DocumentMut, source: &DocumentMut) {
    for section in ["marketplaces", "plugins", "hooks"] {
        match source
            .as_table()
            .get(section)
            .and_then(Item::as_table)
            .filter(|table| !table.is_empty())
        {
            Some(table) => {
                target
                    .as_table_mut()
                    .insert(section, Item::Table(table.clone()));
            }
            None => {
                target.as_table_mut().remove(section);
            }
        }
    }
}

/// 应用供应商时使用：把 live 的 MCP 和插件运行时段合并进即将写回的原文；
/// 原文解析失败时原样返回（保持旧的整文件回退行为）。
pub fn merge_mcp_section(raw: &str, live: &DocumentMut) -> String {
    match parse_document(raw) {
        Ok(mut incoming) => {
            replace_mcp_section(&mut incoming, live);
            replace_plugin_sections(&mut incoming, live);
            normalize_global_section_order(&incoming.to_string())
        }
        Err(_) => raw.to_string(),
    }
}

/// 统一收拢 config.toml 中的全局运行时段，供所有 Agmux 写入路径复用。
pub fn normalize_global_section_order(text: &str) -> String {
    consolidate_plugin_blocks(&consolidate_mcp_blocks(text))
}

fn is_toml_header(line: &str) -> bool {
    line.starts_with('[')
        && line[1..]
            .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '"' || c == '\'')
}

fn toml_blocks(text: &str) -> Vec<Vec<&str>> {
    let mut blocks = Vec::new();
    for line in text.lines() {
        if is_toml_header(line) {
            blocks.push(Vec::new());
        }
        match blocks.last_mut() {
            Some(lines) => lines.push(line),
            None => blocks.push(vec![line]),
        }
    }
    blocks
}

/// 把渲染后散落的插件全局段收拢为连续块，并固定为市场、插件、hooks 的顺序。
/// 与 MCP 相同，锚定在原本最后一个插件全局块的位置，其他配置表的相对顺序不变。
fn consolidate_plugin_blocks(text: &str) -> String {
    let blocks: Vec<_> = toml_blocks(text)
        .into_iter()
        .map(|lines| {
            let section = match lines.first().copied() {
                Some(line) if line.starts_with("[marketplaces.") || line == "[marketplaces]" => {
                    Some(0)
                }
                Some(line) if line.starts_with("[plugins.") || line == "[plugins]" => Some(1),
                Some(line) if line.starts_with("[hooks.") || line == "[hooks]" => Some(2),
                _ => None,
            };
            (section, lines)
        })
        .collect();
    if blocks
        .iter()
        .filter(|(section, _)| section.is_some())
        .count()
        <= 1
    {
        return text.to_string();
    }

    let anchor = blocks
        .iter()
        .rposition(|(section, _)| section.is_some())
        .expect("插件块数量大于一时必有锚点");
    let mut sections = [Vec::new(), Vec::new(), Vec::new()];
    for (section, lines) in &blocks {
        if let Some(section) = section {
            sections[*section].extend(lines.iter().copied());
        }
    }

    let mut out = Vec::new();
    for (index, (section, lines)) in blocks.iter().enumerate() {
        if index == anchor {
            for section in &sections {
                out.extend(section.iter().copied());
            }
        }
        if section.is_none() {
            out.extend(lines.iter().copied());
        }
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// 把渲染后的 TOML 文本中散落各处的 [mcp_servers.*] 块收拢成一个连续块，
/// 锚定在原本最后一个 mcp 块的位置。Codex 官方应用自己的写入器会把各段
/// 散插在 plugins / features 等表之间；toml_edit 按原始字节位置渲染、忠实
/// 保留这种散落，所以我们每次落盘 config.toml 前都做一次文本级整理：
/// 块内容（注释、空行、键值）逐字保留，只搬动块的位置，让 mcp 段模块化。
pub fn consolidate_mcp_blocks(text: &str) -> String {
    // 按表头行切块：(\[\[ 开头的数组表也算)。首个表头之前的根级键值归入无名首块
    let blocks: Vec<_> = toml_blocks(text)
        .into_iter()
        .map(|lines| {
            let header = lines.first().copied().filter(|line| is_toml_header(line));
            (
                header.is_some_and(|line| {
                    line.starts_with("[mcp_servers.") || line == "[mcp_servers]"
                }),
                header.and_then(mcp_server_name_from_header),
                lines,
            )
        })
        .collect();
    let mcp_count = blocks.iter().filter(|(is_mcp, _, _)| *is_mcp).count();
    // 没有 mcp，或只有一块（已连续）：无需整理
    if mcp_count <= 1 {
        return text.to_string();
    }
    // 锚点 = 最后一个 mcp 块的位置：收拢后 mcp 连续块落在这里，
    // 之前的非 mcp 表（如全部 plugins）自然连成一组
    let anchor = blocks
        .iter()
        .rposition(|(is_mcp, _, _)| *is_mcp)
        .expect("mcp_count > 1 必有锚点");
    let mut mcp_lines: Vec<&str> = Vec::new();
    let mut groups: Vec<(Option<&str>, Vec<&str>)> = Vec::new();
    for (is_mcp, server_name, lines) in &blocks {
        if *is_mcp {
            let server_name = server_name.as_deref();
            if let Some((_, grouped_lines)) =
                groups.iter_mut().find(|(name, _)| *name == server_name)
            {
                grouped_lines.extend(lines.iter().copied());
            } else {
                groups.push((server_name, lines.clone()));
            }
        }
    }
    for (_, lines) in groups {
        mcp_lines.extend(lines);
    }
    let mut out: Vec<&str> = Vec::new();
    for (index, (is_mcp, _, lines)) in blocks.iter().enumerate() {
        if index == anchor {
            out.extend(mcp_lines.iter().copied());
        }
        if !*is_mcp {
            out.extend(lines.iter().copied());
        }
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn mcp_server_name_from_header(line: &str) -> Option<String> {
    let name = line.strip_prefix("[mcp_servers.")?.strip_suffix(']')?;
    if let Some(quote) = name
        .chars()
        .next()
        .filter(|quote| *quote == '"' || *quote == '\'')
    {
        let end = name[1..].find(quote)? + 1;
        return Some(name[..=end].to_string());
    }
    Some(name.split('.').next()?.to_string())
}

/// 提取 [mcp_servers.*] 每个服务器的独立片段（含 [mcp_servers.<名称>] 表头、注释、子表，
/// 往返无损；跳过 Codex 托管条目——镜像里不该有它们），(名称, 片段) 按文件顺序。
/// 数据库镜像与创建表单预填共用。
pub fn mcp_server_fragments_from_document(document: &DocumentMut) -> Vec<(String, String)> {
    let Some(servers) = document
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table)
    else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|(name, item)| {
            let table = item.as_table()?;
            let mut piece = DocumentMut::new();
            let mut section = Table::new();
            section.set_implicit(true);
            section.insert(name, Item::Table(table.clone()));
            piece
                .as_table_mut()
                .insert("mcp_servers", Item::Table(section));
            Some((name.to_string(), piece.to_string()))
        })
        .filter(|(name, _)| !is_managed_mcp_name(name))
        .collect()
}

/// 把单个镜像片段回解成建模配置（片段由本模块生成，正常必然可解析；
/// 解析失败返回 None，调用方按整段文本差异展示）。
pub fn spec_from_fragment(name: &str, fragment: &str) -> Option<McpServerSpec> {
    let document = parse_document(fragment).ok()?;
    mcp_servers_from_document(&document)
        .into_iter()
        .find(|spec| spec.name == name)
}

/// 用片段按序重建 [mcp_servers] 段（数据库备份恢复写回 live 用）；
/// 解析失败的片段跳过。Codex 托管条目（node_repl 等）不进片段、也不随重建删除：
/// live 已有的托管条目原样保留——删了 Codex 下次启动也会自动写回（openai/codex#28556）。
pub fn replace_mcp_section_from_fragments(
    document: &mut DocumentMut,
    fragments: &[(String, String)],
) {
    let mut section = Table::new();
    section.set_implicit(true);
    if let Some(existing) = document
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table)
    {
        for (name, item) in existing.iter() {
            if is_managed_mcp_name(name) {
                section.insert(name, item.clone());
            }
        }
    }
    for (name, fragment) in fragments {
        if is_managed_mcp_name(name) {
            continue;
        }
        let parsed = parse_document(fragment).ok().and_then(|doc| {
            doc.as_table()
                .get("mcp_servers")
                .and_then(Item::as_table)
                .and_then(|servers| servers.get(name))
                .and_then(Item::as_table)
                .cloned()
        });
        if let Some(table) = parsed {
            section.insert(name, Item::Table(table));
        }
    }
    document.as_table_mut().remove("mcp_servers");
    if !section.is_empty() {
        document
            .as_table_mut()
            .insert("mcp_servers", Item::Table(section));
    }
}

/// 文本级重建 `[mcp_servers.*]` 区域：整份文件解析不了时的专用路径。
///
/// 与 `replace_mcp_section_from_fragments` 的关键区别是**它不依赖文档能解析**——
/// 按行认出 MCP 区域，只替换区域内的行，区域外（projects / plugins / desktop /
/// model …）逐字节原样保留。托管条目（node_repl 等）不在镜像里，按 live 原文
/// 留在原位。区域边界一律以「行首 `[`」为准，绝不跨过任何表头吞并相邻段落。
///
/// 定位不到 MCP 区域、或镜像片段一条都不可用时返回 `None`：那时只能走备份恢复，
/// 整份重写会连带丢掉区域外的全部配置，代价远高于收益。
pub fn rebuild_mcp_region_text(live_text: &str, fragments: &[(String, String)]) -> Option<String> {
    // 镜像片段渲染成可直接拼接的文本，解析不了的跳过——一条坏片段不该让整次修复失败
    let replacement: Vec<String> = fragments
        .iter()
        .filter(|(name, fragment)| !is_managed_mcp_name(name) && parse_document(fragment).is_ok())
        .map(|(_, fragment)| format!("{}\n\n", fragment.trim()))
        .collect();
    if replacement.is_empty() {
        return None;
    }
    let lines: Vec<&str> = live_text.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|line| mcp_region_name(line).is_some())?;
    let end = lines[start..]
        .iter()
        .position(|line| is_table_header(line) && mcp_region_name(line).is_none())
        .map_or(lines.len(), |offset| start + offset);

    let mut rebuilt = Vec::new();
    let mut placed = false;
    let mut index = start;
    while index < end {
        // 块边界 = 下一个表头行：托管子表（`[mcp_servers.x.env]`）自成一块但同名同待遇
        let block_end = (index + 1..end)
            .find(|&line| is_table_header(lines[line]))
            .unwrap_or(end);
        if is_managed_mcp_name(mcp_region_name(lines[index]).unwrap_or_default()) {
            rebuilt.extend(
                lines[index..block_end]
                    .iter()
                    .map(|line| (*line).to_string()),
            );
        } else if !placed {
            // 镜像片段落在第一个非托管块的位置，托管块留在它们原来的位置
            placed = true;
            rebuilt.extend(replacement.iter().cloned());
        }
        index = block_end;
    }
    if !placed {
        rebuilt.extend(replacement);
    }

    Some(lines[..start].concat() + &rebuilt.concat() + &lines[end..].concat())
}

/// 行首表头里 `mcp_servers` 之后的名字（`[mcp_servers]` 自身返回空串）；
/// 不是 mcp_servers 表头返回 `None`。只看行首，不做完整 TOML 解析——
/// 调用它的场景就是"整份文档已经解析不了"。
fn mcp_region_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('[')?.trim_start_matches('[');
    let rest = rest.strip_prefix("mcp_servers")?;
    match rest.as_bytes().first()? {
        b'.' => {
            let name = &rest[1..];
            let end = name
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
                .unwrap_or(name.len());
            Some(&name[..end])
        }
        b']' => Some(""),
        _ => None,
    }
}

fn is_table_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// 把表单建模字段写进单服务器片段文本（编辑页"表单 → 编辑器"实时同步用）。
/// 复用保存路径的 upsert：未建模键、注释、格式原样保留；
/// 段内现有服务器名与 spec.name 不同时按重命名处理（删旧建新）。
pub fn patch_mcp_fragment(toml: &str, spec: &McpServerSpec) -> AppResult<String> {
    let mut document = parse_document(toml)?;
    let existing = document
        .as_table()
        .get("mcp_servers")
        .and_then(Item::as_table)
        .and_then(|servers| servers.iter().next())
        .map(|(name, _)| name.to_string());
    if let Some(old) = existing {
        if old != spec.name {
            // 原位重命名：整表搬到新键名（未建模键、注释随表保留），不能删了重建
            if let Some(servers) = document
                .as_table_mut()
                .get_mut("mcp_servers")
                .and_then(Item::as_table_mut)
            {
                if let Some(item) = servers.remove(&old) {
                    servers.insert(spec.name.as_str(), item);
                }
            }
        }
    }
    upsert_mcp_server(&mut document, spec)?;
    Ok(document.to_string())
}

/// 解析单服务器片段为建模字段（编辑页"编辑器 → 表单"实时回填用）。
/// 片段里没有服务器或语法不完整时返回 Err，由前端忽略（等待输入完成）。
pub fn parse_mcp_fragment(toml: &str) -> AppResult<McpServerSpec> {
    let document = parse_document(toml)?;
    mcp_servers_from_document(&document)
        .into_iter()
        .next()
        .ok_or_else(|| app_err!("片段中没有 MCP 服务器"))
}

fn string_of(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(Item::as_str).map(str::to_string)
}

/// 就地替换已有键的值：不经过 insert 换键，键上的装饰（前缀注释、行内注释）原样保留。
fn replace_value(table: &mut Table, key: &str, value: Value) {
    if let Some(Item::Value(old)) = table.get_mut(key) {
        *old = value;
    } else {
        table.insert(key, Item::Value(value));
    }
}

/// 同 replace_value，用于子表整体重建（env / http_headers 等）；
/// 被替换子表头部的前缀注释等装饰随旧表带回。
fn replace_table(table: &mut Table, key: &str, sub: Table) {
    if let Some(Item::Table(old)) = table.get_mut(key) {
        let decor = old.decor().clone();
        *old = sub;
        *old.decor_mut() = decor;
    } else {
        table.insert(key, Item::Table(sub));
    }
}

/// Some 写入、None 移除（bool/i64 等 JSON 直接映射的 TOML 标量共用）。
fn set_typed_value<T: Into<Value>>(table: &mut Table, key: &str, value: Option<T>) {
    match value {
        Some(value) => replace_value(table, key, value.into()),
        None => {
            table.remove(key);
        }
    }
}

fn boolean_of(table: &Table, key: &str) -> Option<bool> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(Value::as_bool)
}

fn integer_of(table: &Table, key: &str) -> Option<i64> {
    table
        .get(key)
        .and_then(Item::as_value)
        .and_then(Value::as_integer)
}

/// 子表两种写法都读：[mcp_servers.x.env] 段形式与 env = { .. } 内联形式；非字符串值跳过。
fn string_map(item: Option<&Item>) -> BTreeMap<String, String> {
    let pairs: Vec<(String, String)> = match item {
        Some(Item::Table(table)) => table
            .iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.to_string(), text.to_string()))
            })
            .collect(),
        Some(Item::Value(Value::InlineTable(inline))) => inline
            .iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.to_string(), text.to_string()))
            })
            .collect(),
        _ => Vec::new(),
    };
    pairs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_provider_body_sets_and_removes_fields() {
        let body = r#"
name = "ZAI"
base_url = "https://old.example"
experimental_bearer_token = "old"
"#;
        let updated =
            update_provider_body(body, Some("https://api.z.ai"), Some("new-key")).unwrap();
        assert!(updated.contains(r#"base_url = "https://api.z.ai""#));
        assert!(updated.contains(r#"experimental_bearer_token = "new-key""#));

        let cleared = update_provider_body(&updated, None, Some("")).unwrap();
        assert!(!cleared.contains("experimental_bearer_token"));
        assert!(cleared.contains(r#"base_url = "https://api.z.ai""#));
    }

    #[test]
    fn patch_context_override_updates_only_the_two_root_keys() {
        let source = r#"
model = "gpt-5.6"
model_context_window = 272000
model_auto_compact_token_limit = 200000

[features]
# keep this comment
goals = true
"#;

        let enabled = patch_context_override(source, true, 900_000).unwrap();
        assert!(enabled.contains("model_context_window = 1000000"));
        assert!(enabled.contains("model_auto_compact_token_limit = 900000"));
        assert!(enabled.contains("# keep this comment"));
        assert_eq!(enabled.matches("model_context_window").count(), 1);
        assert_eq!(enabled.matches("model_auto_compact_token_limit").count(), 1);

        let disabled = patch_context_override(&enabled, false, 900_000).unwrap();
        assert!(!disabled.contains("model_context_window"));
        assert!(!disabled.contains("model_auto_compact_token_limit"));
        assert!(disabled.contains("# keep this comment"));
    }

    #[test]
    fn patch_context_override_uses_custom_compact_limit_and_rejects_out_of_range_values() {
        let source = "model = \"gpt-5.6\"\n";

        let enabled = patch_context_override(source, true, 750_000).unwrap();
        assert!(enabled.contains("model_context_window = 1000000"));
        assert!(enabled.contains("model_auto_compact_token_limit = 750000"));

        assert!(patch_context_override(source, true, 0).is_err());
        assert!(patch_context_override(source, true, 1_000_001).is_err());
    }

    #[test]
    fn context_management_can_be_toggled_without_losing_features() {
        let source = "[features]\ngoals = true\n";
        let enabled = patch_context_management(source, true).unwrap();
        assert!(enabled.contains("goals = true"));
        assert!(enabled.contains("experimental_mode = true"));

        let disabled = patch_context_management(&enabled, false).unwrap();
        assert!(disabled.contains("goals = true"));
        assert!(!disabled.contains("experimental_mode"));
    }

    #[test]
    fn system_proxy_can_be_toggled_without_losing_features() {
        let source = "[features]\ngoals = true\n";
        let enabled = patch_system_proxy(source, true).unwrap();
        assert!(enabled.contains("goals = true"));
        assert!(enabled.contains("respect_system_proxy = true"));

        let disabled = patch_system_proxy(&enabled, false).unwrap();
        assert!(disabled.contains("goals = true"));
        assert!(!disabled.contains("respect_system_proxy"));
    }

    const SOURCE: &str = r#"
model = "glm-5.3"
model_provider = "ZAI"
model_reasoning_effort = "high"
model_catalog_json = "zai.json"

[features]
# user comment stays
goals = true

[mcp_servers.test]
command = "node"

[model_providers.ZAI]
name = "ZAI"
base_url = "https://api.z.ai"
wire_api = "responses"
experimental_bearer_token = "secret"

[model_providers.Old]
name = "Old"
"#;

    #[test]
    fn capture_and_apply_preserves_unrelated_configuration() {
        let mut document = parse_document(SOURCE).unwrap();
        let payload = capture_from_document(&document).unwrap();
        document.as_table_mut().remove("model_catalog_json");

        apply_to_document(&mut document, &payload).unwrap();
        let text = document.to_string();

        assert!(text.contains("# user comment stays"));
        assert!(text.contains("glm-5.3"));
        assert!(text.contains("model_catalog_json"));
        assert!(text.contains("[mcp_servers.test]"));
        assert!(text.contains("[model_providers.Old]"));
        assert_eq!(text.matches("experimental_bearer_token").count(), 1);
    }

    #[test]
    fn apply_removes_stale_model_keys() {
        let mut document = parse_document(SOURCE).unwrap();
        document.as_table_mut().remove("model_catalog_json");
        let payload = capture_from_document(&document).unwrap();
        document
            .as_table_mut()
            .insert("model_stale", Item::Value("yes".into()));

        apply_to_document(&mut document, &payload).unwrap();

        assert!(!document.to_string().contains("model_stale"));
    }

    #[test]
    fn validate_document_reports_invalid_toml_range() {
        let source = "name = \"ZAI\"\n[features]\ngoals =\n";
        let diagnostics = validate_document(source);

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].from <= diagnostics[0].to);
        assert!(diagnostics[0].to <= source.len());
        assert!(!diagnostics[0].message.is_empty());
    }

    #[test]
    fn validate_document_uses_utf16_offsets_for_editor() {
        let source = "# 🦄\nname = @\n";
        let diagnostics = validate_document(source);
        let prefix = "# 🦄\nname = ";

        assert_eq!(diagnostics[0].from, prefix.encode_utf16().count());
    }

    #[test]
    fn validate_document_reports_multiple_invalid_toml_ranges() {
        let source = "first =\nsecond = @\n";
        let diagnostics = validate_document(source);

        // taplo 错误恢复式解析：两处错误都被定位（`first =` 行尾 7、`@` 字符 17）
        assert!(diagnostics.len() >= 2, "{diagnostics:?}");
        assert!(
            diagnostics.iter().any(|d| d.from == 7),
            "应定位到第一个错误：{diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.from == 17),
            "应定位到 @ 字符：{diagnostics:?}"
        );
    }

    #[test]
    fn validate_document_does_not_repeat_one_toml_error() {
        let source = "items = [\"a\" \"b\"]\n";
        let diagnostics = validate_document(source);

        let mut ranges = diagnostics
            .iter()
            .map(|d| (d.from, d.to))
            .collect::<Vec<_>>();
        ranges.sort_unstable();
        ranges.dedup();
        assert_eq!(ranges.len(), diagnostics.len(), "同一区间不应重复报告");
        assert!(diagnostics.len() <= 10, "单点错误不应刷屏：{diagnostics:?}");
    }

    #[test]
    fn validate_document_missing_quote_does_not_cascade() {
        // 真实样本：仅第一个 trusted_hash 缺结尾引号，其余两个 section 完整
        let source = concat!(
            r#"[hooks.state."ponytail@ponytail:hooks/claude-codex-hooks.json:session_start:0:0"]"#,
            "\n",
            r#"trusted_hash = "sha256:5f81d38f47448a1581c08ec877e044d9e04dd6f814dce3f2671f7a8edadd719b"#,
            "\n\n",
            r#"[hooks.state."ponytail@ponytail:hooks/claude-codex-hooks.json:user_prompt_submit:0:0"]"#,
            "\n",
            r#"trusted_hash = "sha256:6a6f42bc3b58d6262db38bfd74d7f340fcca2b09cdb134aad365063f0bfefca4""#,
            "\n\n",
            r#"[hooks.state."ponytail@ponytail:hooks/claude-codex-hooks.json:subagent_start:0:0"]"#,
            "\n",
            r#"trusted_hash = "sha256:1423b56c1322f96c8f74c51c1e7ae9a047b904c1fa43ee9165d462fd7a6e70ef""#,
            "\n",
        );
        let diagnostics = validate_document(source);

        // 单点标点缺失只报本地错误：恢复式解析不应让后续合法 section 级联报错
        assert!(
            diagnostics.len() <= 3,
            "缺一个引号不应级联刷屏：{diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|d| d.from < source.len() && d.to <= source.len()),
            "位置应落在文档内：{diagnostics:?}"
        );
    }

    #[test]
    fn capture_model_values_strips_decor_and_roundtrips() {
        // 行内注释/前导空格属于源码装饰，不能进入 model_values（apply re-parse 与前端显示都会被污染）
        let source = r#"model = "glm-5.3" # 主模型
model_catalog_json = "~/.codex/a.json" # 目录
model_reasoning_effort = "high""#;
        let payload = capture_from_document(&parse_document(source).unwrap()).unwrap();
        assert_eq!(
            payload.model_values.get("model_catalog_json").unwrap(),
            "\"~/.codex/a.json\""
        );
        assert_eq!(payload.model_values.get("model").unwrap(), "\"glm-5.3\"");

        // round-trip：capture 的值必须能被 parse_value 重新解析（apply 回写路径）
        for raw in payload.model_values.values() {
            parse_value(raw).unwrap_or_else(|error| panic!("{} 无法 re-parse: {error}", raw));
        }
    }

    #[test]
    fn format_document_normalizes_spacing_and_survives_errors() {
        let formatted = format_document("a =1\n[table]\nb= 2\n");
        assert!(formatted.contains("a = 1"), "{formatted}");
        assert!(formatted.contains("b = 2"), "{formatted}");

        // 含语法错误时不 panic：taplo 跳过错误区间仍产出文本
        assert!(!format_document("a = @\n").is_empty());
    }

    #[test]
    fn mcp_servers_parses_stdio_http_and_inline_subtables() {
        let source = r#"
[mcp_servers.local_repl]
command = 'C:\bin\node_repl.exe'
args = ["--a", "--b"]
startup_timeout_sec = 120

[mcp_servers.local_repl.env]
CODEX_HOME = 'C:\.codex'

[mcp_servers.tavily]
url = "https://mcp.tavily.com/mcp"
bearer_token_env_var = "TAVILY_API_KEY"

[mcp_servers.tavily.http_headers]
X-Region = "us"

[mcp_servers.exa]
url = "https://mcp.exa.ai/mcp"
env_http_headers = { "x-api-key" = "EXA_API_KEY" }
"#;
        let servers = mcp_servers_from_document(&parse_document(source).unwrap());

        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].name, "local_repl");
        assert_eq!(servers[0].command.as_deref(), Some(r"C:\bin\node_repl.exe"));
        assert_eq!(servers[0].args, ["--a", "--b"]);
        assert_eq!(servers[0].startup_timeout_sec, Some(120));
        assert_eq!(
            servers[0].env.get("CODEX_HOME").map(String::as_str),
            Some(r"C:\.codex")
        );

        assert_eq!(servers[1].name, "tavily");
        assert_eq!(
            servers[1].url.as_deref(),
            Some("https://mcp.tavily.com/mcp")
        );
        assert_eq!(
            servers[1].bearer_token_env_var.as_deref(),
            Some("TAVILY_API_KEY")
        );
        assert_eq!(
            servers[1].http_headers.get("X-Region").map(String::as_str),
            Some("us")
        );

        assert_eq!(servers[2].name, "exa");
        assert_eq!(
            servers[2]
                .env_http_headers
                .get("x-api-key")
                .map(String::as_str),
            Some("EXA_API_KEY")
        );
    }

    #[test]
    fn upsert_new_server_renders_expected_section() {
        let mut document = DocumentMut::new();
        upsert_mcp_server(
            &mut document,
            &McpServerSpec {
                name: "context7".into(),
                url: Some("https://mcp.context7.com/mcp".into()),
                bearer_token_env_var: Some("CONTEXT7_API_KEY".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let text = document.to_string();
        assert!(text.contains("[mcp_servers.context7]"), "{text}");
        assert!(
            text.contains(r#"url = "https://mcp.context7.com/mcp""#),
            "{text}"
        );
        assert!(
            text.contains(r#"bearer_token_env_var = "CONTEXT7_API_KEY""#),
            "{text}"
        );
        assert!(!text.contains("command"), "{text}");
    }

    #[test]
    fn upsert_edit_preserves_unmodeled_keys_and_comments() {
        let source = r#"
[mcp_servers.github]
# 手动维护的服务器
command = "github-mcp-server"
args = ["stdio"]
cwd = "/srv"

[mcp_servers.github.env]
GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_x"
EXTRA = "1"

[mcp_servers.github.tools.read]
approval_mode = "approve"
"#;
        let mut document = parse_document(source).unwrap();
        upsert_mcp_server(
            &mut document,
            &McpServerSpec {
                name: "github".into(),
                command: Some("github-mcp-server".into()),
                args: vec!["stdio".into(), "--verbose".into()],
                env: BTreeMap::from([(
                    "GITHUB_PERSONAL_ACCESS_TOKEN".to_string(),
                    "ghp_y".to_string(),
                )]),
                ..Default::default()
            },
        )
        .unwrap();

        let text = document.to_string();
        assert!(text.contains("# 手动维护的服务器"), "{text}");
        assert!(text.contains("cwd = \"/srv\""), "{text}");
        assert!(text.contains("[mcp_servers.github.tools.read]"), "{text}");
        assert!(text.contains("approval_mode = \"approve\""), "{text}");
        assert!(text.contains("\"--verbose\""), "{text}");
        // env 子表按 map 全量重建：旧 token 不留、未列入的 EXTRA 也被清掉
        assert!(!text.contains("ghp_x"), "{text}");
        assert!(!text.contains("EXTRA"), "{text}");
        assert!(text.contains("ghp_y"), "{text}");
    }

    #[test]
    fn upsert_removes_emptied_modeled_keys() {
        let source = r#"
[mcp_servers.old]
command = "node"
url = "https://old.example"
startup_timeout_sec = 20
enabled = false

[mcp_servers.old.env]
A = "1"
"#;
        let mut document = parse_document(source).unwrap();
        upsert_mcp_server(
            &mut document,
            &McpServerSpec {
                name: "old".into(),
                command: Some("node".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let text = document.to_string();
        assert!(text.contains("command = \"node\""), "{text}");
        assert!(!text.contains("url"), "{text}");
        assert!(!text.contains("startup_timeout_sec"), "{text}");
        assert!(!text.contains("enabled"), "{text}");
        assert!(!text.contains("A = \"1\""), "{text}");
        assert!(!text.contains("[mcp_servers.old.env]"), "{text}");
    }

    #[test]
    fn upsert_scalar_after_subtable_reparses_correctly() {
        // 已有 env 子表的服务器再补一个标量键：依赖 toml_edit 先值后子表的渲染顺序，
        // 标量若落到 [x.env] 头之后就会归属错误——重解析钉住该假设。
        let source = r#"
[mcp_servers.exa]
url = "https://mcp.exa.ai/mcp"

[mcp_servers.exa.env]
A = "1"
"#;
        let mut document = parse_document(source).unwrap();
        upsert_mcp_server(
            &mut document,
            &McpServerSpec {
                name: "exa".into(),
                url: Some("https://mcp.exa.ai/mcp".into()),
                bearer_token_env_var: Some("EXA_API_KEY".into()),
                env: BTreeMap::from([("A".to_string(), "1".to_string())]),
                ..Default::default()
            },
        )
        .unwrap();

        let text = document.to_string();
        let servers = mcp_servers_from_document(&parse_document(&text).unwrap());
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].bearer_token_env_var.as_deref(),
            Some("EXA_API_KEY")
        );
        assert_eq!(servers[0].env.get("A").map(String::as_str), Some("1"));
    }

    #[test]
    fn replace_mcp_section_carries_decor_and_drops_stale() {
        let live = parse_document(
            "# live 注释\n[mcp_servers.tavily]\nurl = \"https://mcp.tavily.com/mcp\"\n",
        )
        .unwrap();

        let mut target =
            parse_document("model = \"gpt-5.6\"\n\n[mcp_servers.stale]\ncommand = \"old\"\n")
                .unwrap();
        replace_mcp_section(&mut target, &live);
        let text = target.to_string();
        assert!(text.contains("model = \"gpt-5.6\""), "{text}");
        assert!(text.contains("mcp_servers.tavily"), "{text}");
        assert!(text.contains("live 注释"), "{text}");
        assert!(!text.contains("stale"), "{text}");

        // live 无 MCP 段：target 的陈旧段被清除（全局模型：删除即全局删除）
        let empty_live = parse_document("model = \"gpt-5.6\"\n").unwrap();
        let mut target = parse_document("[mcp_servers.stale]\ncommand = \"old\"\n").unwrap();
        replace_mcp_section(&mut target, &empty_live);
        assert!(!target.to_string().contains("stale"));
    }

    #[test]
    fn replace_plugin_sections_carries_global_plugin_state() {
        let live = parse_document(
            "[marketplaces.ponytail]\nsource = \"https://github.com/DietrichGebert/ponytail.git\"\n\n[plugins.\"ponytail@ponytail\"]\nenabled = true\n\n[hooks.state]\n\n",
        )
        .unwrap();
        let mut target = parse_document(
            "[marketplaces.stale]\nsource = \"old\"\n\n[plugins.\"stale@stale\"]\nenabled = true\n\n[hooks.state]\nold = true\n",
        )
        .unwrap();

        replace_plugin_sections(&mut target, &live);
        let text = target.to_string();
        assert!(text.contains("marketplaces.ponytail"), "{text}");
        assert!(text.contains("ponytail@ponytail"), "{text}");
        assert!(!text.contains("marketplaces.stale"), "{text}");
        assert!(!text.contains("stale@stale"), "{text}");
        assert!(!text.contains("old = true"), "{text}");
    }

    #[test]
    fn merge_mcp_section_falls_back_to_verbatim_on_invalid_raw() {
        let live = parse_document("[mcp_servers.tavily]\nurl = \"x\"\n").unwrap();
        let raw = "not [ valid";
        assert_eq!(merge_mcp_section(raw, &live), raw);

        let merged = merge_mcp_section("model = \"gpt-5.6\"\n", &live);
        assert!(merged.contains("mcp_servers.tavily"), "{merged}");
        assert!(merged.contains("model = \"gpt-5.6\""), "{merged}");
        assert!(
            !merged.contains("[mcp_servers]\n"),
            "合并隐式 MCP 父表时不能额外写出显式根表：\n{merged}"
        );

        let live = parse_document(
            "[marketplaces.ponytail]\nsource = \"https://github.com/DietrichGebert/ponytail.git\"\n\n[plugins.\"ponytail@ponytail\"]\nenabled = true\n",
        )
        .unwrap();
        let merged = merge_mcp_section("model = \"gpt-5.6\"\n", &live);
        assert!(merged.contains("marketplaces.ponytail"), "{merged}");
        assert!(merged.contains("ponytail@ponytail"), "{merged}");
    }

    #[test]
    fn merge_mcp_section_groups_plugin_sections_for_existing_and_new_config() {
        let live = parse_document(
            "[marketplaces.live]\nsource = \"https://example.com/market.git\"\n\n[plugins.\"live@market\"]\nenabled = true\n\n[hooks.state]\nversion = 1\n",
        )
        .unwrap();
        let assert_grouped = |text: &str| {
            let pos = |needle: &str| {
                text.find(needle)
                    .unwrap_or_else(|| panic!("缺少 {needle}：\n{text}"))
            };
            let marketplaces = pos("[marketplaces.live]");
            let plugins = pos("[plugins.\"live@market\"]");
            let hooks = pos("[hooks.state]");
            assert!(marketplaces < plugins && plugins < hooks, "{text}");
            assert!(
                !text[marketplaces..hooks].contains("[desktop]"),
                "插件全局段之间不能混入其他表：\n{text}"
            );
        };

        // 旧配置中的三个全局段已被 Codex 散插：合并后仍必须连续且按固定顺序。
        assert_grouped(&merge_mcp_section(
            "[marketplaces.stale]\nsource = \"old\"\n\n[desktop]\nlocale = \"zh-CN\"\n\n[plugins.\"stale@market\"]\nenabled = false\n\n[features]\nflag = true\n\n[hooks.state]\nversion = 0\n",
            &live,
        ));

        // 新建快照没有上述全局段时，也应以同一顺序追加。
        assert_grouped(&merge_mcp_section("model = \"gpt-5.6\"\n", &live));

        // MCP 组件和插件 CLI 的直写路径同样通过全局规范化入口。
        assert_grouped(&normalize_global_section_order(
            "[marketplaces.live]\nsource = \"https://example.com/market.git\"\n\n[desktop]\nlocale = \"zh-CN\"\n\n[plugins.\"live@market\"]\nenabled = true\n\n[hooks.state]\nversion = 1\n",
        ));
    }

    #[test]
    fn rebuilding_mcp_section_from_fragments_keeps_parent_implicit() {
        let mut document = DocumentMut::new();
        replace_mcp_section_from_fragments(
            &mut document,
            &[(
                "context7".to_string(),
                "[mcp_servers.context7]\nurl = \"https://mcp.context7.com/mcp\"\n".to_string(),
            )],
        );

        let text = document.to_string();
        assert!(
            !text.contains("[mcp_servers]\n"),
            "从片段重建时不能写出空的 MCP 根表：\n{text}"
        );
    }

    #[test]
    fn mcp_fragments_round_trip_losslessly() {
        // 片段是数据库镜像的存储形态：注释、子表必须原样往返
        let source = r#"
model = "gpt-5.6"

[mcp_servers.github]
# 手动维护的服务器
command = "github-mcp-server"
cwd = "/srv"

[mcp_servers.github.env]
TOKEN = "ghp_x"

[mcp_servers.tavily]
url = "https://mcp.tavily.com/mcp"
"#;
        let document = parse_document(source).unwrap();
        let fragments = mcp_server_fragments_from_document(&document);
        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0].0, "github");
        assert!(
            !fragments[0].1.contains("[mcp_servers]"),
            "单服务片段不能写出显式 MCP 根表：\n{}",
            fragments[0].1
        );
        assert!(fragments[0].1.contains("# 手动维护的服务器"));
        assert!(fragments[0].1.contains("cwd = \"/srv\""));

        let mut target =
            parse_document("model = \"other\"\n[mcp_servers.stale]\ncommand = \"x\"\n").unwrap();
        replace_mcp_section_from_fragments(&mut target, &fragments);
        let text = target.to_string();
        assert!(text.contains("# 手动维护的服务器"), "{text}");
        assert!(text.contains("[mcp_servers.github.env]"), "{text}");
        assert!(text.contains("[mcp_servers.tavily]"), "{text}");
        assert!(!text.contains("stale"), "{text}");
    }

    #[test]
    fn managed_mcp_entries_are_skipped_but_preserved() {
        // 这些条目由 Codex/ChatGPT 桌面版自动写入：提取时跳过，重建段时保留。
        let source = "[mcp_servers.node_repl]\ncommand = \"node_repl.exe\"\n\n[mcp_servers.computer-use]\ncommand = \"SkyComputerUseClient\"\n\n[mcp_servers.cua_repl]\ncommand = \"ChatGPT\"\n\n[mcp_servers.github]\ncommand = \"gh\"\n";
        let document = parse_document(source).unwrap();

        assert_eq!(
            mcp_servers_from_document(&document)
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>(),
            ["github"]
        );
        let fragments = mcp_server_fragments_from_document(&document);
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].0, "github");

        // 空片段重建（镜像清零场景）：托管条目仍在，用户条目被移除
        let mut target = document.clone();
        replace_mcp_section_from_fragments(&mut target, &[]);
        let text = target.to_string();
        assert!(text.contains("[mcp_servers.node_repl]"), "{text}");
        assert!(text.contains("[mcp_servers.computer-use]"), "{text}");
        assert!(text.contains("[mcp_servers.cua_repl]"), "{text}");
        assert!(!text.contains("mcp_servers.github"), "{text}");
    }

    #[test]
    fn mcp_fragment_patch_and_parse_round_trip() {
        // 表单 → 片段：重命名删旧建新；未建模键与注释原样保留
        let base = "[mcp_servers.old]\n# 手动维护\ncwd = \"/srv\"\ncommand = \"npx\"\n";
        let spec = McpServerSpec {
            name: "fresh".into(),
            command: Some("node".into()),
            args: vec!["-y".into()],
            ..Default::default()
        };
        let patched = patch_mcp_fragment(base, &spec).unwrap();
        assert!(patched.contains("[mcp_servers.fresh]"), "{patched}");
        assert!(!patched.contains("mcp_servers.old"), "{patched}");
        assert!(patched.contains("# 手动维护"), "{patched}");
        assert!(patched.contains("cwd = \"/srv\""), "{patched}");
        assert!(patched.contains("command = \"node\""), "{patched}");
        assert!(patched.contains("\"-y\""), "{patched}");

        // 片段 → 表单：解析回建模字段
        let parsed = parse_mcp_fragment(&patched).unwrap();
        assert_eq!(parsed.name, "fresh");
        assert_eq!(parsed.command.as_deref(), Some("node"));
        assert_eq!(parsed.args, ["-y"]);

        // 语法不完整 / 无服务器：Err，由前端忽略
        assert!(parse_mcp_fragment("[mcp_servers.x]\ncommand = \"un").is_err());
        assert!(parse_mcp_fragment("model = \"gpt\"\n").is_err());
    }

    #[test]
    fn consolidate_mcp_blocks_gathers_scattered_sections() {
        // Codex 官方应用写入器留下的布局：mcp 子表散插在 plugins / features 等表之间
        let scattered = concat!(
            "model = \"gpt-5.6\"\n\n",
            "[plugins.a]\nenabled = true\n\n",
            "[mcp_servers.one]\nurl = \"https://one\"\n\n",
            "[plugins.b]\nenabled = true\n\n",
            "[features]\njs = false\n\n",
            "[mcp_servers.one.env]\n# 手动维护\nTOKEN = \"t\"\n\n",
            "[mcp_servers.two]\ncommand = \"x\"\n\n",
            "[shell_environment_policy.set]\nK = \"v\"\n",
        );
        let text = consolidate_mcp_blocks(scattered);
        let pos = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("缺少 {needle}：\n{text}"))
        };
        // mcp 收拢为连续块：one → one.env → two 之间没有其他表
        assert!(pos("[mcp_servers.one]") < pos("[mcp_servers.one.env]"));
        assert!(pos("[mcp_servers.one.env]") < pos("[mcp_servers.two]"));
        assert!(
            !text[pos("[mcp_servers.one]")..pos("[mcp_servers.two]")].contains("[plugins."),
            "{text}"
        );
        // 锚定在最后一个 mcp 块的位置：plugins 收在 mcp 之前，其余表保持原相对顺序
        assert!(pos("[plugins.a]") < pos("[plugins.b]"));
        assert!(pos("[plugins.b]") < pos("[mcp_servers.one]"));
        assert!(pos("[mcp_servers.two]") < pos("[shell_environment_policy.set]"));
        // 注释与键值逐字保留，根级键值仍在最前
        assert!(text.contains("# 手动维护"), "{text}");
        assert!(text.contains("TOKEN = \"t\""), "{text}");
        assert!(pos("model =") < pos("[plugins.a]"));
    }

    #[test]
    fn consolidate_mcp_blocks_keeps_each_servers_child_tables_together() {
        let text = consolidate_mcp_blocks(
            "[mcp_servers.one]\nurl = \"https://one\"\n\n[mcp_servers.two]\nurl = \"https://two\"\n\n[mcp_servers.one.env]\nTOKEN = \"t\"\n",
        );
        let pos = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("缺少 {needle}：\n{text}"))
        };

        assert!(
            pos("[mcp_servers.one]") < pos("[mcp_servers.one.env]"),
            "{text}"
        );
        assert!(
            pos("[mcp_servers.one.env]") < pos("[mcp_servers.two]"),
            "{text}"
        );
    }

    #[test]
    fn consolidate_mcp_blocks_keeps_tidy_text_untouched() {
        // 已连续的多块 mcp 与没有 mcp 的文本：逐字节原样返回
        let tidy =
            "[a]\nx = 1\n\n[mcp_servers.one]\nurl = \"u\"\n\n[mcp_servers.two]\nurl = \"v\"\n";
        assert_eq!(consolidate_mcp_blocks(tidy), tidy);
        let no_mcp = "[a]\nx = 1\n";
        assert_eq!(consolidate_mcp_blocks(no_mcp), no_mcp);
    }
}
