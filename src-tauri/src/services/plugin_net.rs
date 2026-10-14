//! 插件源的 GitHub 访问层：仓库元数据、文件树、raw 文件下载。
//! 低频下载按需读取共享网络策略，保留本链路的超时与 User-Agent。

use std::time::Duration;

use serde::Deserialize;

use super::{app_err, AppResult};

const USER_AGENT: &str = "agmux-plugin-marketplace";
const PREVIEW_FILE_LIMIT: usize = 200;

/// 解析后的 GitHub 来源：仓库 + 可选 ref 与插件子目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSource {
    pub owner: String,
    pub repo: String,
    pub ref_name: Option<String>,
    pub sub_path: Option<String>,
}

/// GitHub git/trees 条目（只保留需要的字段）。
#[derive(Debug, Clone, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
}

pub struct RepoTree {
    pub default_branch: String,
    pub reference: String,
    pub entries: Vec<TreeEntry>,
}

/// 接受 `https://github.com/owner/repo`、`.../tree/<ref>[/子目录]`、`owner/repo` 简写。
/// 返回的 sub_path 是去掉首尾斜杠的目录形式（仓库根为 None）。
pub fn parse_github_url(input: &str) -> AppResult<GithubSource> {
    let text = input.trim();
    if text.contains("://") && !text.contains("github.com") {
        return Err(app_err!(
            "目前仅支持 GitHub 来源，请提供 github.com 仓库地址"
        ));
    }
    let text = text.strip_suffix(".git").unwrap_or(text);
    let text = text.strip_prefix("https://github.com/").unwrap_or(text);
    let text = text.strip_prefix("http://github.com/").unwrap_or(text);
    let text = text.trim_matches('/');

    let (head, ref_name, sub_path) = match text.split_once("/tree/") {
        Some((head, rest)) => {
            let mut segments = rest.split('/').filter(|part| !part.is_empty());
            let reference = segments.next().map(str::to_string);
            let joined = segments.collect::<Vec<_>>().join("/");
            (
                head,
                reference,
                if joined.is_empty() {
                    None
                } else {
                    Some(joined)
                },
            )
        }
        None => (text, None, None),
    };
    let mut parts = head.split('/').filter(|part| !part.is_empty());
    let owner = parts.next().unwrap_or_default().to_string();
    let repo = parts.next().unwrap_or_default().to_string();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return Err(app_err!(
            "无法识别 GitHub 地址，请使用 https://github.com/<owner>/<repo> 或 owner/repo（可带 /tree/<分支>/<子目录>）"
        ));
    }

    Ok(GithubSource {
        owner,
        repo,
        ref_name,
        sub_path,
    })
}

async fn build_client() -> AppResult<reqwest::Client> {
    crate::network::Network::current()
        .await
        .map_err(|error| app_err!("{error}"))?
        .builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| app_err!("创建网络客户端失败: {error}"))
}

fn map_status(
    status: reqwest::StatusCode,
    body: &str,
    context: &str,
) -> Option<crate::error::AppError> {
    if status.is_success() {
        return None;
    }
    let rate_limited = status.as_u16() == 403 && body.to_ascii_lowercase().contains("rate limit");
    Some(app_err!(
        "{}失败（HTTP {}）：{}",
        context,
        status.as_u16(),
        if rate_limited {
            "GitHub 匿名请求已达限额，稍后再试，或改用精选目录 / 手动导入"
        } else if status.as_u16() == 404 {
            "仓库、分支或路径不存在，请检查地址"
        } else {
            "GitHub 拒绝了请求"
        }
    ))
}

async fn get_bytes(url: &str, context: &str) -> AppResult<Vec<u8>> {
    let client = build_client().await?;
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| app_err!("{context}失败：{error}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|error| app_err!("{context}失败：{error}"))?;
    if let Some(error) = map_status(status, &String::from_utf8_lossy(&body), context) {
        return Err(error);
    }
    Ok(body.to_vec())
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str, context: &str) -> AppResult<T> {
    let bytes = get_bytes(url, context).await?;
    serde_json::from_slice(&bytes).map_err(|error| app_err!("{context}返回的数据无效: {error}"))
}

#[derive(Deserialize)]
struct RepoMeta {
    default_branch: String,
}

#[derive(Deserialize)]
struct TreeResponse {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

/// 拉取仓库默认分支与完整文件树（一次 API 调用，之后逐文件走 raw CDN，避开 API 限额）。
pub async fn fetch_repo_tree(source: &GithubSource) -> AppResult<RepoTree> {
    let meta: RepoMeta = get_json(
        &format!(
            "https://api.github.com/repos/{}/{}",
            source.owner, source.repo
        ),
        "获取仓库信息",
    )
    .await?;
    let reference = source
        .ref_name
        .clone()
        .unwrap_or_else(|| meta.default_branch.clone());
    let response: TreeResponse = get_json(
        &format!(
            "https://api.github.com/repos/{}/{}/git/trees/{reference}?recursive=1",
            source.owner, source.repo
        ),
        "获取文件树",
    )
    .await?;
    if response.truncated {
        return Err(app_err!(
            "仓库文件树过大，GitHub 返回了截断结果；请指定插件所在子目录"
        ));
    }
    Ok(RepoTree {
        default_branch: meta.default_branch,
        reference,
        entries: response.tree,
    })
}

/// raw 文件下载：不走 API，无 60 次/小时限额。
pub async fn fetch_raw_file(
    source: &GithubSource,
    reference: &str,
    path: &str,
) -> AppResult<Vec<u8>> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{}/{reference}/{}",
        source.owner,
        source.repo,
        path.trim_matches('/')
    );
    get_bytes(&url, "下载插件文件").await
}

/// 预览用的文件数上限，避免巨型仓库刷爆前端。
pub fn preview_file_limit() -> usize {
    PREVIEW_FILE_LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_tree_urls() {
        let plain = parse_github_url("https://github.com/openai/plugins").unwrap();
        assert_eq!(plain.owner, "openai");
        assert_eq!(plain.repo, "plugins");
        assert_eq!(plain.ref_name, None);
        assert_eq!(plain.sub_path, None);

        let with_ref = parse_github_url("https://github.com/a/b/tree/main").unwrap();
        assert_eq!(with_ref.ref_name.as_deref(), Some("main"));

        let with_sub =
            parse_github_url("https://github.com/a/b/tree/v1.0/plugins/memory-bank/").unwrap();
        assert_eq!(with_sub.ref_name.as_deref(), Some("v1.0"));
        assert_eq!(with_sub.sub_path.as_deref(), Some("plugins/memory-bank"));

        let shorthand = parse_github_url("owner/repo.git").unwrap();
        assert_eq!(shorthand.owner, "owner");
        assert_eq!(shorthand.repo, "repo");
    }

    #[test]
    fn rejects_non_github_or_incomplete_urls() {
        assert!(parse_github_url("https://gitlab.com/a/b").is_err());
        assert!(parse_github_url("https://github.com/only-owner").is_err());
        assert!(parse_github_url("https://github.com/a/b/blob/main/x").is_err());
    }
}
