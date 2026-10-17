use std::path::{Path, PathBuf};

use crate::error::{app_err, AppResult};

/// 数据目录名。改名发布时旧数据由用户从设置页「导出备份 / 恢复」自行搬过来，
/// 应用不做目录迁移。
const ROOT_DIR: &str = ".agmux";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub settings: PathBuf,
    /// 应用内更新「已更新到 vX」一次性标记：安装成功前写入，新版本启动时消费。
    /// 存文件而不是前端 localStorage——Windows 安装器会立即结束进程，WebView 的
    /// localStorage 异步落盘可能来不及提交导致标记丢失、通知不弹。
    pub update_marker: PathBuf,
    pub config_backup: PathBuf,
    pub database_backup: PathBuf,
    pub codex_files_backup: PathBuf,
    /// 运行日志目录：tauri-plugin-log 按大小轮转写入，保留最近几个归档
    pub logs: PathBuf,
    pub codex_home: PathBuf,
    /// Claude Code 的家目录（~/.claude）：供应商配置写入其下 settings.json。
    pub claude_home: PathBuf,
    /// 供应商终端的 per-session settings 覆盖文件目录：只放 `claude --settings`
    /// 的临时覆盖，不进 ~/.claude，不参与 apply 与回写。
    pub terminal: PathBuf,
    /// 插件图标瘦身缓存：超重位图解码缩放后的重编码结果落这里，键含源文件 mtime。
    pub plugin_icons: PathBuf,
}

impl AppPaths {
    pub fn codex_config(&self) -> PathBuf {
        self.codex_home.join("config.toml")
    }

    /// Claude Code 用户范围 MCP 配置（官方格式：用户目录下的 ~/.claude.json）。
    pub fn claude_mcp_config(&self) -> PathBuf {
        self.claude_home
            .parent()
            .unwrap_or(&self.claude_home)
            .join(".claude.json")
    }

    /// 某个 Claude 供应商的终端覆盖文件：固定命名、每次覆写，不随进程删除。
    pub fn terminal_settings(&self, profile_id: &str) -> PathBuf {
        self.terminal.join(format!("claude_{profile_id}.json"))
    }

    pub fn ensure(&self) -> AppResult<()> {
        for dir in [
            &self.root,
            &self.config_backup,
            &self.database_backup,
            &self.codex_files_backup,
            &self.logs,
            &self.terminal,
            &self.plugin_icons,
        ] {
            std::fs::create_dir_all(dir)
                .map_err(|error| app_err!("无法创建目录 {}: {error}", dir.display()))?;
        }
        Ok(())
    }
}

pub fn app_paths() -> AppResult<AppPaths> {
    from_home(&home_dir())
}

pub fn from_home(home: &Path) -> AppResult<AppPaths> {
    let root = home.join(ROOT_DIR);
    Ok(AppPaths {
        database: root.join("agmux.db"),
        settings: root.join("settings.json"),
        update_marker: root.join("update-marker"),
        config_backup: root.join("backups").join("config"),
        database_backup: root.join("backups").join("database"),
        codex_files_backup: root.join("backups").join("codex-files"),
        logs: root.join("logs"),
        terminal: root.join("terminal"),
        plugin_icons: root.join("plugin-icons"),
        codex_home: home.join(".codex"),
        claude_home: home.join(".claude"),
        root,
    })
}

pub fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|base| base.home_dir().to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

/// Unix 秒（i64 语义由调用方按需转换）：备份文件名与 token 时间戳共用同一个"当前时刻"定义。
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_agmux_and_codex_directories() {
        let home = Path::new("/home/user");
        let paths = from_home(home).unwrap();
        assert_eq!(paths.root, home.join(".agmux"));
        assert_eq!(paths.database, home.join(".agmux").join("agmux.db"));
        assert_eq!(paths.logs, home.join(".agmux").join("logs"));
        assert_eq!(
            paths.codex_config(),
            home.join(".codex").join("config.toml")
        );
        assert_eq!(paths.claude_home, home.join(".claude"));
        assert_eq!(paths.claude_mcp_config(), home.join(".claude.json"));
        assert_eq!(paths.terminal, home.join(".agmux").join("terminal"));
        assert_eq!(
            paths.terminal_settings("p-1"),
            home.join(".agmux").join("terminal").join("claude_p-1.json")
        );
    }
}
