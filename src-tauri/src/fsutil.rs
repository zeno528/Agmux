use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::error::{app_err, AppResult};
use crate::paths::now_ms;

/// 跨文件/数据库操作失败时恢复原文件；调用方负责持有应用操作锁。
pub fn with_file_rollback<T>(
    paths: &[PathBuf],
    operation: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    let originals = paths
        .iter()
        .map(|path| match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(app_err!("无法读取 {}: {error}", path.display())),
        })
        .collect::<AppResult<Vec<_>>>()?;
    match operation() {
        Ok(value) => Ok(value),
        Err(error) => {
            let mut failures = Vec::new();
            for (path, original) in paths.iter().zip(originals) {
                // 未发生写入的不重写，避免无权限文件导致虚假的回滚失败。
                if fs::read(path).ok() == original {
                    continue;
                }
                let result = match original {
                    Some(bytes) => atomic_write(path, &bytes),
                    None => fs::remove_file(path)
                        .or_else(|error| {
                            if error.kind() == io::ErrorKind::NotFound {
                                Ok(())
                            } else {
                                Err(error)
                            }
                        })
                        .map_err(|error| app_err!("无法撤下 {}: {error}", path.display())),
                };
                if let Err(error) = result {
                    failures.push(error.to_string());
                }
            }
            if failures.is_empty() {
                Err(error)
            } else {
                Err(app_err!("{error}；原文件恢复失败：{}", failures.join("；")))
            }
        }
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| app_err!("无法创建目录 {}: {error}", parent.display()))?;
    }

    let mut temp = NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))
        .map_err(|error| app_err!("无法创建临时文件: {error}"))?;
    temp.write_all(bytes)
        .and_then(|_| temp.flush())
        .map_err(|error| app_err!("无法写入临时文件: {error}"))?;
    let temp_path = temp.into_temp_path();
    replace_file(&temp_path, path)
        .map_err(|error| app_err!("无法替换 {}: {error}", path.display()))?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_utf16: Vec<_> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let target_utf16: Vec<_> = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let result = MoveFileExW(
            PCWSTR::from_raw(source_utf16.as_ptr()),
            PCWSTR::from_raw(target_utf16.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        );
        result.map_err(|error| io::Error::other(error.to_string()))
    }
}

pub fn backup_file(source: &Path, directory: &Path, stem: &str) -> AppResult<Option<PathBuf>> {
    if !source.exists() {
        return Ok(None);
    }
    fs::create_dir_all(directory)
        .map_err(|error| app_err!("无法创建备份目录 {}: {error}", directory.display()))?;
    let target = directory.join(format!("{stem}-{}.bak", now_ms()));
    fs::copy(source, &target)
        .map_err(|error| app_err!("无法备份 {}: {error}", source.display()))?;
    prune_backups(directory, stem, ".bak", 5);
    Ok(Some(target))
}

pub fn prune_backups(directory: &Path, prefix: &str, extension: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut backups: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(extension))
        })
        .collect();
    // 手动、自动及重命名后的备份按文件时间统一淘汰，文件名仅用于同时间排序。
    backups.sort_by_cached_key(|path| {
        (
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok(),
            path.clone(),
        )
    });
    while backups.len() > keep {
        let oldest = backups.remove(0);
        let _ = fs::remove_file(oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_retention_keeps_latest_files_regardless_of_source_or_name() {
        let dir = tempfile::tempdir().unwrap();
        let names = [
            "cg-backup-manual-20260101.db",
            "cg-backup-20260102.db",
            "cg-backup-manual-renamed.db",
            "cg-backup-20260104.db",
            "cg-backup-manual-20260105.db",
        ];
        for (index, name) in names.iter().enumerate() {
            let file = fs::File::create(dir.path().join(name)).unwrap();
            file.set_times(fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(1_700_000_000 + index as u64),
            ))
            .unwrap();
            prune_backups(dir.path(), "cg-backup-", ".db", 3);
            for (position, candidate) in names[..=index].iter().enumerate() {
                assert_eq!(
                    dir.path().join(candidate).exists(),
                    position >= (index + 1).saturating_sub(3),
                    "after creating {name}: {candidate}"
                );
            }
        }
    }

    #[test]
    fn atomic_write_replaces_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"old = true\n").unwrap();

        atomic_write(&path, b"new = true\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new = true\n");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
