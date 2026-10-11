use super::{app_err, now_ms, now_secs, prune_backups, AppContext, AppResult, Path, PathBuf};

pub(super) const DATABASE_BACKUP_PREFIX: &str = "cg-backup-";
// 历史备份的文件名固定，品牌改名不能改变识别前缀。
const LEGACY_DATABASE_BACKUP_PREFIX: &str = "budtty-export-";

// 手动备份带 manual- 标记，自动备份保持纯时间戳：前端靠文件名区分记录行的"手动/自动"前缀
fn database_backup_name(manual: bool) -> String {
    let now = chrono::Local::now();
    let marker = if manual { "manual-" } else { "" };
    format!(
        "{}{marker}{}-{:03}.db",
        DATABASE_BACKUP_PREFIX,
        now.format("%Y%m%d-%H%M%S"),
        now.timestamp_subsec_millis()
    )
}

pub(super) fn backup_keep_count(value: u32) -> usize {
    match value {
        3 | 5 | 10 | 15 | 20 | 30 => value as usize,
        _ => 5,
    }
}

/// 数据库备份文件信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct DatabaseBackupInfo {
    pub name: String,
    pub size_bytes: u64,
    pub created_at: i64,
}

impl AppContext {
    pub fn export_database(&self) -> AppResult<PathBuf> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let target = self.export_database_unlocked(true)?;
        tauri_plugin_log::log::info!(
            "[backup.export] target={:?} outcome=success msg=\"已创建数据库备份\"",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        );
        Ok(target)
    }

    pub fn export_database_to(&self, directory: &str) -> AppResult<PathBuf> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let directory = PathBuf::from(directory);
        if !directory.is_dir() {
            return Err(app_err!("导出目录不存在"));
        }
        let target = directory.join(database_backup_name(false));
        self.database.export_database(&target)?;
        tauri_plugin_log::log::info!(
            "[backup.export] target={:?} outcome=success msg=\"已导出数据库备份\"",
            target
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        );
        Ok(target)
    }

    pub(super) fn export_database_unlocked(&self, manual: bool) -> AppResult<PathBuf> {
        let directory = &self.paths.database_backup;
        std::fs::create_dir_all(directory)
            .map_err(|error| app_err!("无法创建备份目录: {error}"))?;
        let name = database_backup_name(manual);
        let target = directory.join(&name);
        self.database.export_database(&target)?;
        let keep = backup_keep_count(self.settings()?.database_backup_keep_count);
        prune_backups(directory, DATABASE_BACKUP_PREFIX, ".db", keep);
        self.database.record_event(
            None,
            "export",
            "success",
            Some("database exported"),
            &now_ms().to_string(),
        )?;
        Ok(target)
    }

    pub fn auto_backup_if_due(&self) -> AppResult<bool> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let settings = self.settings()?;
        if settings.auto_backup_interval_hours == 0 {
            return Ok(false);
        }

        let due = match self.list_database_backups()?.first() {
            None => true,
            Some(latest) => {
                let created_at = latest.created_at.max(0) as u64;
                now_secs().saturating_sub(created_at)
                    >= settings.auto_backup_interval_hours.saturating_mul(3600)
            }
        };
        if !due {
            return Ok(false);
        }

        self.export_database_unlocked(false)?;
        // 自动备份是静默发生的（无 UI 反馈），文件日志留一行便于事后确认它真的跑过
        if let Some(latest) = self.list_database_backups()?.first() {
            tauri_plugin_log::log::info!(
                "[backup.auto] path={} size_bytes={} outcome=success msg=\"自动备份完成\"",
                latest.name,
                latest.size_bytes
            );
        }
        Ok(true)
    }

    /// 从用户选择的备份文件导入并恢复。
    pub fn import_database(&self, path: &str) -> AppResult<()> {
        let source = PathBuf::from(path);
        let canonical = source
            .canonicalize()
            .map_err(|_| app_err!("备份文件不存在：{path}"))?;
        let live = self
            .paths
            .database
            .canonicalize()
            .unwrap_or_else(|_| self.paths.database.clone());
        if canonical == live {
            return Err(app_err!("不能导入当前正在使用的数据库文件"));
        }
        self.restore_database_from_path(&canonical)?;
        self.database.record_event(
            None,
            "import",
            "success",
            Some("database imported"),
            &now_ms().to_string(),
        )?;
        tauri_plugin_log::log::info!(
            "[backup.import] path={path:?} outcome=success msg=\"已导入数据库并恢复\""
        );
        Ok(())
    }

    pub fn list_database_backups(&self) -> AppResult<Vec<DatabaseBackupInfo>> {
        let directory = &self.paths.database_backup;
        let mut backups = Vec::new();
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !((name.starts_with(DATABASE_BACKUP_PREFIX)
                    || name.starts_with(LEGACY_DATABASE_BACKUP_PREFIX))
                    && name.ends_with(".db"))
                {
                    continue;
                }
                let size_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
                let created_at = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0);
                backups.push(DatabaseBackupInfo {
                    name,
                    size_bytes,
                    created_at,
                });
            }
        }
        backups.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.name.cmp(&left.name))
        });
        Ok(backups)
    }

    pub fn restore_database(&self, name: &str) -> AppResult<()> {
        let path = self.database_backup_path(name)?;
        self.restore_database_from_path(&path)?;
        self.database.record_event(
            None,
            "restore",
            "success",
            Some("database restored"),
            &now_ms().to_string(),
        )?;
        tauri_plugin_log::log::info!(
            "[backup.restore] path={:?} outcome=success msg=\"已恢复备份\"",
            path.display().to_string()
        );
        Ok(())
    }

    fn restore_database_from_path(&self, path: &Path) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| app_err!("操作锁已损坏"))?;
        let previous_mcp = self.database.mcp_server_records()?;
        // 恢复会整表替换 claude_profiles / app_state：先记下恢复前的激活行，
        // 恢复后把 live settings.json 的托管键收敛到恢复出的激活位（回到备份时刻状态）
        let previous_profile = self
            .database
            .active_claude_profile()?
            .as_deref()
            .map(|id| self.database.claude_profile(id))
            .transpose()?;
        std::fs::create_dir_all(&self.paths.database_backup)
            .map_err(|error| app_err!("无法创建备份目录: {error}"))?;
        // 使用随机后缀，避免连续恢复和导出在同一毫秒内撞文件名；保留恢复前数据库供人工恢复。
        let backup = tempfile::Builder::new()
            .prefix("cg-backup-manual-")
            .suffix(".db")
            .tempfile_in(&self.paths.database_backup)
            .map_err(|error| app_err!("无法创建恢复前备份: {error}"))?
            .into_temp_path();
        std::fs::remove_file(&backup).map_err(|error| app_err!("无法准备备份文件: {error}"))?;
        self.database.export_database(&backup)?;
        let backup = backup
            .keep()
            .map_err(|error| app_err!("无法保留恢复前备份: {error}"))?;
        crate::fsutil::with_file_rollback(
            &[
                self.paths.codex_config(),
                self.paths.claude_mcp_config(),
                self.paths.claude_home.join("settings.json"),
            ],
            || {
                let has_mcp = self.database.restore_from_backup(path)?;
                let result = (|| {
                    let restored_profile = self
                        .database
                        .active_claude_profile()?
                        .as_deref()
                        .map(|id| self.database.claude_profile(id))
                        .transpose()?;
                    self.reconcile_claude_live_after_restore(
                        previous_profile.as_ref(),
                        restored_profile.as_ref(),
                    )?;
                    // 备份里的 MCP 镜像写回 live config.toml（旧备份无 MCP 表则保持 live 现状）
                    if has_mcp {
                        self.write_mcp_to_live_from_database(&previous_mcp)?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    self.database
                        .restore_from_backup(&backup)
                        .map_err(|rollback| {
                            app_err!(
                                "{error}；数据库回滚失败: {rollback}；恢复前备份: {}",
                                backup.display()
                            )
                        })?;
                    return Err(error);
                }
                Ok(())
            },
        )
    }

    pub fn delete_database_backup(&self, name: &str) -> AppResult<()> {
        let path = self.database_backup_path(name)?;
        std::fs::remove_file(&path).map_err(|error| app_err!("删除备份失败: {error}"))?;
        tauri_plugin_log::log::info!(
            "[backup.delete] target={name:?} outcome=success msg=\"已删除数据库备份\""
        );
        Ok(())
    }

    /// 重命名备份（标题写入文件名，保留 cg-backup- 前缀与 .db 后缀）。
    pub fn rename_database_backup(&self, old_name: &str, title: &str) -> AppResult<()> {
        let from = self.database_backup_path(old_name)?;
        let mut stem = title.trim().to_string();
        if let Some(rest) = stem.strip_prefix(DATABASE_BACKUP_PREFIX) {
            stem = rest.to_string();
        } else if let Some(rest) = stem.strip_prefix(LEGACY_DATABASE_BACKUP_PREFIX) {
            stem = rest.to_string();
        }
        if stem.ends_with(".db") {
            stem.truncate(stem.len() - 3);
        }
        let stem: String = stem
            .chars()
            .filter(|ch| !matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
            .take(80)
            .collect();
        let stem = stem.trim();
        if stem.is_empty() {
            return Err(app_err!("备份标题不能为空"));
        }
        let to = self
            .paths
            .database_backup
            .join(format!("{DATABASE_BACKUP_PREFIX}{stem}.db"));
        if to == from {
            return Ok(());
        }
        if to.exists() {
            return Err(app_err!("同名备份已存在"));
        }
        std::fs::rename(&from, &to).map_err(|error| app_err!("重命名备份失败: {error}"))?;
        tauri_plugin_log::log::info!(
            "[backup.rename] target={:?} outcome=success msg=\"已重命名数据库备份\"",
            to.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        );
        Ok(())
    }

    pub(super) fn database_backup_path(&self, name: &str) -> AppResult<PathBuf> {
        let valid = (name.starts_with(DATABASE_BACKUP_PREFIX)
            || name.starts_with(LEGACY_DATABASE_BACKUP_PREFIX))
            && name.ends_with(".db")
            && Path::new(name).file_name().and_then(|file| file.to_str()) == Some(name);
        if !valid {
            return Err(app_err!("无效的备份文件名"));
        }
        Ok(self.paths.database_backup.join(name))
    }
}

#[cfg(test)]
mod tests {
    use super::database_backup_name;

    #[test]
    fn backup_name_carries_source_marker() {
        let auto = database_backup_name(false);
        assert!(auto.starts_with("cg-backup-"));
        assert!(!auto.contains("manual-"));
        assert!(database_backup_name(true).starts_with("cg-backup-manual-"));
    }
}
