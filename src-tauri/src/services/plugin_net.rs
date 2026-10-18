//! 插件市场来源的 GitHub 地址解析。

use super::{app_err, AppResult};

/// 解析后的 GitHub 来源：仓库 + 可选 ref 与插件子目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSource {
    pub owner: String,
    pub repo: String,
    pub ref_name: Option<String>,
    pub sub_path: Option<String>,
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
