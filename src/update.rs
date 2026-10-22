//! GitHub Release 更新检查。
//!
//! 仅做版本对比；自更新后续通过 `--self-update` 子命令提供。

use anyhow::Context as _;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::time::Duration;

/// 仓库首页（关于页展示）。
pub const REPO_URL: &str = "https://github.com/jetsung/quota-watch";
/// 提交问题的地址（关于页“问题反馈”按钮）。
pub const ISSUES_URL: &str = "https://github.com/jetsung/quota-watch/issues/new";
/// 当前版本。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/jetsung/quota-watch/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// 去掉 `v` 前缀的版本号。
    pub version: String,
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

/// 请求 GitHub Releases API 获取最新正式版本。
pub fn check_latest() -> anyhow::Result<ReleaseInfo> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("quota-watch/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("创建 HTTP 客户端失败")?;
    let response = client
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("连接 GitHub API 失败")?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("GitHub API 请求失败：HTTP {}", status.as_u16());
    }
    let text = response.text().context("读取 GitHub API 响应失败")?;
    let release: LatestRelease =
        serde_json::from_str(&text).context("解析 GitHub Release 响应失败")?;
    let version = normalize_version(&release.tag_name);
    if version.is_empty() {
        anyhow::bail!("GitHub Release 未提供版本号");
    }
    Ok(ReleaseInfo { version })
}

/// `latest` 是否比 `current` 新（按 major.minor.patch 比较）。
pub fn is_newer(latest: &str, current: &str) -> bool {
    version_key(latest) > version_key(current)
}

fn normalize_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

fn version_key(version: &str) -> (u64, u64, u64) {
    let normalized = normalize_version(version);
    let mut parts = normalized.split('.').map(|part| {
        part.split(['-', '+'])
            .next()
            .unwrap_or(part)
            .parse::<u64>()
            .unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_numeric_segments() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
    }

    #[test]
    fn version_key_ignores_v_prefix_and_suffix() {
        assert_eq!(version_key("v1.2.3"), (1, 2, 3));
        assert_eq!(version_key("1.2.3-beta.1"), (1, 2, 3));
        assert_eq!(version_key("1.2"), (1, 2, 0));
        assert_eq!(version_key("weird"), (0, 0, 0));
    }
}
