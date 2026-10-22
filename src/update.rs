//! GitHub Release 更新检查。
//!
//! 仅做版本对比；自更新后续通过 `--self-update` 子命令提供。

use anyhow::Context as _;
use reqwest::blocking::Client;
use serde::Deserialize;
use std::io::Write as _;
use std::time::Duration;

/// 仓库首页（关于页展示）。
pub const REPO_URL: &str = "https://github.com/jetsung/quota-watch";
/// 提交问题的地址（关于页“问题反馈”按钮）。
pub const ISSUES_URL: &str = "https://github.com/jetsung/quota-watch/issues/new";
/// 当前版本。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/jetsung/quota-watch/releases/latest";

/// Releases latest API 地址。调试时可设环境变量 `QUOTA_WATCH_RELEASES_API`
/// 指向本地 Mock 服务器（如 `http://127.0.0.1:8929/latest.json`），避免每次拉取 GitHub 都很慢。
fn latest_release_api() -> String {
    std::env::var("QUOTA_WATCH_RELEASES_API").unwrap_or_else(|_| LATEST_RELEASE_API.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// 去掉 `v` 前缀的版本号。
    pub version: String,
    /// Release 资产列表（按平台匹配下载）。
    pub assets: Vec<Asset>,
}

/// Release 资产（对应 GitHub API 的 `assets` 数组项）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub download_url: String,
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<LatestAsset>,
}

#[derive(Deserialize)]
struct LatestAsset {
    name: String,
    browser_download_url: String,
}

/// 请求 GitHub Releases API 获取最新正式版本。
pub fn check_latest() -> anyhow::Result<ReleaseInfo> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("quota-watch/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("创建 HTTP 客户端失败")?;
    let response = client
        .get(latest_release_api())
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
    let assets = release
        .assets
        .into_iter()
        .map(|a| Asset {
            name: a.name,
            download_url: a.browser_download_url,
        })
        .collect();
    Ok(ReleaseInfo { version, assets })
}

/// `latest` 是否比 `current` 新（按 semver 比较：major.minor.patch，相同时按预发布段比较，预发布 < 正式版）。
pub fn is_newer(latest: &str, current: &str) -> bool {
    compare_version(latest, current) == std::cmp::Ordering::Greater
}

/// 预发布标识符（如 `beta.1` 中的 `beta` / `1`）：数字按数值比较，字母按字符串比较（semver 规则：数字 < 字母）。
#[derive(Debug, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Alpha(String),
}

/// 版本主版本段（`-` 之前）的 (major, minor, patch)。
fn version_key(version: &str) -> (u64, u64, u64) {
    let core = normalize_version(version)
        .split(['-', '+'])
        .next()
        .unwrap_or(version)
        .to_string();
    let mut parts = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// 解析预发布段（`-` 之后、`+` 之前）的标识符列表；无预发布返回空。
fn parse_pre(version: &str) -> Vec<PreId> {
    let normalized = normalize_version(version);
    let Some((_, pre)) = normalized.split_once('-') else {
        return Vec::new();
    };
    let pre = pre.split('+').next().unwrap_or(pre);
    pre.split('.')
        .filter(|id| !id.is_empty())
        .map(|id| match id.parse::<u64>() {
            Ok(n) => PreId::Num(n),
            Err(_) => PreId::Alpha(id.to_string()),
        })
        .collect()
}

fn compare_pre(a: &[PreId], b: &[PreId]) -> std::cmp::Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = match (x, y) {
            (PreId::Num(x), PreId::Num(y)) => x.cmp(y),
            (PreId::Num(_), PreId::Alpha(_)) => std::cmp::Ordering::Less,
            (PreId::Alpha(_), PreId::Num(_)) => std::cmp::Ordering::Greater,
            (PreId::Alpha(x), PreId::Alpha(y)) => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    // 标识符完全相同（如 beta.1 vs beta.1）视为相等；前缀更长的一方更大（semver：1.2 > 1）
    a.len().cmp(&b.len())
}

/// semver 全量比较：主版本段不同直接比；相同时预发布版本小于正式版，预发布之间逐标识符比较。
fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    let ord = version_key(a).cmp(&version_key(b));
    if ord != std::cmp::Ordering::Equal {
        return ord;
    }
    let (pa, pb) = (parse_pre(a), parse_pre(b));
    match (pa.is_empty(), pb.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => compare_pre(&pa, &pb),
    }
}

/// 当前平台的资产文件名后缀：Linux/macOS 为 tar.xz 压缩包，Windows 为 zip（与 Release 资产命名一致）。
fn platform_archive_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.xz"
    }
}

/// Release 资产文件名中的 OS 关键词（macOS 资产以 `apple-darwin` 命名）。
fn os_keyword() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    }
}

/// 按编译期 `target_os` + `target_arch` 关键词匹配当前平台的压缩包资产
/// （文件名小写需同时包含 os 与 arch 关键词、以平台压缩包格式结尾），返回第一个匹配项。
/// deb/rpm 等包管理器格式与 SHA256SUMS 等非压缩包资产被扩展名过滤排除。
pub fn select_platform_asset(assets: &[Asset]) -> Option<&Asset> {
    let (os, arch) = (os_keyword(), std::env::consts::ARCH);
    let ext = platform_archive_ext();
    assets.iter().find(|asset| {
        let name = asset.name.to_lowercase();
        name.ends_with(ext) && name.contains(os) && name.contains(arch)
    })
}

/// 自升级结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// 已是最新版本，未做任何下载。
    AlreadyUpToDate,
    /// 已下载并替换当前可执行文件（重启后生效）。
    Updated,
}

/// 升级取消错误：用户在下载过程中主动取消，区别于失败。
#[derive(Debug)]
pub struct UpdateCancelled;

impl std::fmt::Display for UpdateCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "升级已取消")
    }
}

impl std::error::Error for UpdateCancelled {}

/// 升级进度事件（供 CLI 与 UI 展示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateProgress {
    /// 下载中：`total` 为 `None` 表示服务器未返回总大小。
    Downloading { downloaded: u64, total: Option<u64> },
    /// 下载完成，正在解压升级包。
    Extracting,
    /// 正在原子替换可执行文件。
    Replacing,
}

impl UpdateProgress {
    /// 下载进度百分比（0–100），总大小未知或非下载阶段返回 `None`。
    pub fn percent(&self) -> Option<u8> {
        match *self {
            UpdateProgress::Downloading {
                downloaded,
                total: Some(total),
            } if total > 0 => Some(((downloaded.min(total) * 100) / total) as u8),
            _ => None,
        }
    }
}

/// 当前可执行文件路径。Linux 下升级替换二进制后，`/proc/self/exe` 会带
/// ` (deleted)` 后缀（原 inode 已被 rename 覆盖），直接使用会导致新实例
/// 启动失败或二次升级写错目标，这里予以剥离。
pub fn current_exe_path() -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("获取当前可执行文件路径失败")?;
    match exe.to_string_lossy().strip_suffix(" (deleted)") {
        Some(clean) => Ok(std::path::PathBuf::from(clean)),
        None => Ok(exe),
    }
}

/// 同 `self_update()` 的完整自升级流程，通过 `on_progress` 回调报告进度（下载字节/解压/替换）。
/// `is_cancelled` 在每个阶段边界被轮询，返回 true 时以 `UpdateCancelled` 错误中止（已下载部分丢弃）。
pub fn self_update_with_progress(
    is_cancelled: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(UpdateProgress),
) -> anyhow::Result<UpdateOutcome> {
    let release = check_latest()?;
    if !is_newer(&release.version, CURRENT_VERSION) {
        return Ok(UpdateOutcome::AlreadyUpToDate);
    }
    let asset = select_platform_asset(&release.assets)
        .ok_or_else(|| anyhow::anyhow!("未找到当前平台的升级包"))?
        .clone();

    let exe = current_exe_path()?;
    let temp_dir = std::env::temp_dir();
    let archive = temp_dir.join(format!("quota-watch-update-{}.pack", std::process::id()));
    let temp = temp_dir.join(format!("quota-watch-update-{}", std::process::id()));

    // 取消检查：中止升级并清理已下载的临时文件（未替换前原文件不受影响）
    let cancelled = || -> anyhow::Result<()> {
        if is_cancelled() {
            let _ = std::fs::remove_file(&archive);
            let _ = std::fs::remove_file(&temp);
            return Err(anyhow::Error::new(UpdateCancelled));
        }
        Ok(())
    };
    cancelled()?;

    // 清理上次更新遗留的旧可执行文件（Windows 下运行中无法立即删除，留待下次尝试）
    #[cfg(target_os = "windows")]
    if let Some(old) = exe.extension() {
        let _ = std::fs::remove_file(exe.with_extension(format!("{}.old", old.to_string_lossy())));
    }

    let client = Client::builder()
        .timeout(Duration::from_secs(600))
        .user_agent(concat!("quota-watch/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("创建 HTTP 客户端失败")?;
    let mut response = client
        .get(&asset.download_url)
        .send()
        .with_context(|| format!("下载升级包失败：{}", asset.name))?;
    if !response.status().is_success() {
        anyhow::bail!("下载升级包失败：HTTP {}", response.status().as_u16());
    }
    // 服务器返回的总大小（Content-Length），用于计算下载百分比
    let total = response.content_length();
    {
        use std::io::Read as _;
        let mut file = std::fs::File::create(&archive).context("创建临时升级文件失败")?;
        let mut downloaded: u64 = 0;
        let mut buf = [0u8; 64 * 1024];
        loop {
            cancelled()?;
            let n = response.read(&mut buf).context("读取升级包下载流失败")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).context("写入临时升级文件失败")?;
            downloaded += n as u64;
            on_progress(UpdateProgress::Downloading { downloaded, total });
        }
    }
    on_progress(UpdateProgress::Extracting);

    // 压缩包（tar.xz / zip）内为单个裸二进制，解压到临时文件后再替换
    extract_binary(&archive, &temp).with_context(|| format!("解压升级包失败：{}", asset.name))?;
    let _ = std::fs::remove_file(&archive);

    // /tmp 与可执行文件可能跨文件系统，rename 会报 EXDEV；
    // 先把新文件落到 exe 同目录（同一文件系统内），再原子 rename 替换
    let staged = exe.with_extension("update-new");
    std::fs::copy(&temp, &staged).context("暂存新可执行文件失败")?;
    let _ = std::fs::remove_file(&temp);

    // 新二进制需要可执行权限（rename 保留暂存文件权限）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .context("设置暂存文件权限失败")?;
    }

    on_progress(UpdateProgress::Replacing);

    // 原子替换：Linux 允许 rename 覆盖运行中的二进制；Windows 需先把旧文件改名腾出路径
    #[cfg(target_os = "windows")]
    {
        let old = exe.with_extension("exe.old");
        std::fs::rename(&exe, &old).context("重命名旧可执行文件失败")?;
        if let Err(e) = std::fs::rename(&staged, &exe) {
            // 回滚：尽量恢复旧文件名，避免应用无法再次更新
            let _ = std::fs::rename(&old, &exe);
            return Err(anyhow::Error::new(e).context("替换可执行文件失败"));
        }
    }
    #[cfg(not(target_os = "windows"))]
    std::fs::rename(&staged, &exe).context("替换可执行文件失败")?;

    Ok(UpdateOutcome::Updated)
}

/// 从升级压缩包中解出裸二进制写入 `dest`。
/// Linux/macOS 资产为 tar.xz（包内单文件 `quota-watch`）；Windows 资产为 zip（包内 `quota-watch.exe`）。
fn extract_binary(archive_path: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::io::Read as _;
        let file = std::fs::File::open(archive_path).context("打开升级压缩包失败")?;
        let mut zip = zip::ZipArchive::new(file).context("读取 zip 升级包失败")?;
        // 包内为单个可执行文件，按名字后缀匹配
        let mut names: Vec<String> = (0..zip.len())
            .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
            .collect();
        names.sort();
        let target = names
            .into_iter()
            .find(|n| n.to_lowercase().ends_with(".exe"))
            .context("zip 升级包内未找到可执行文件")?;
        let mut entry = zip.by_name(&target).context("读取升级包内文件失败")?;
        let mut out = std::fs::File::create(dest).context("创建临时升级文件失败")?;
        std::io::copy(&mut entry, &mut out).context("解压升级包内容失败")?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let file = std::fs::File::open(archive_path).context("打开升级压缩包失败")?;
        let decoder = xz2::read::XzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().context("遍历 tar 升级包失败")? {
            let mut entry = entry.context("读取 tar 升级包条目失败")?;
            let is_binary = entry
                .path()
                .ok()
                .and_then(|p| p.file_name().map(|n| n == "quota-watch"))
                .unwrap_or(false);
            if is_binary {
                entry.unpack(dest).context("解压升级包内容失败")?;
                return Ok(());
            }
        }
        anyhow::bail!("tar 升级包内未找到 quota-watch 可执行文件")
    }
}

fn normalize_version(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
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

    #[test]
    fn is_newer_compares_prerelease_segments() {
        // 预发布之间按 semver 比较：beta.2 > beta.1
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0-beta.2"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0-beta.1"));
        // 预发布 < 正式版
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        // 主版本段仍优先
        assert!(is_newer("0.3.0-alpha", "0.2.9"));
        // 标识符更多的一方更大（beta.1.1 > beta.1）
        assert!(is_newer("0.2.0-beta.1.1", "0.2.0-beta.1"));
    }

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.to_string(),
            download_url: format!("https://example.com/{name}"),
        }
    }

    /// 与真实 Release 一致的资产清单（v0.1.0 格式）。
    fn release_assets() -> Vec<Asset> {
        vec![
            asset("quota-watch-0.1.0-1.aarch64.rpm"),
            asset("quota-watch-0.1.0-1.x86_64.rpm"),
            asset("quota-watch-v0.1.0-aarch64-apple-darwin.tar.xz"),
            asset("quota-watch-v0.1.0-aarch64-pc-windows-msvc.zip"),
            asset("quota-watch-v0.1.0-aarch64-unknown-linux-gnu.tar.xz"),
            asset("quota-watch-v0.1.0-x86_64-apple-darwin.tar.xz"),
            asset("quota-watch-v0.1.0-x86_64-pc-windows-msvc.zip"),
            asset("quota-watch-v0.1.0-x86_64-unknown-linux-gnu.tar.xz"),
            asset("quota-watch_0.1.0-1_amd64.deb"),
            asset("quota-watch_0.1.0-1_arm64.deb"),
            asset("SHA256SUMS.txt"),
        ]
    }

    #[test]
    fn select_platform_asset_matches_current_platform() {
        let (os, arch) = (os_keyword(), std::env::consts::ARCH);
        let assets = release_assets();
        let matched = select_platform_asset(&assets).expect("应匹配当前平台资产");
        let name = matched.name.to_lowercase();
        assert!(name.contains(os));
        assert!(name.contains(arch));
        assert!(name.ends_with(platform_archive_ext()));
    }

    #[test]
    fn select_platform_asset_skips_non_archive_assets() {
        // deb/rpm/SHA256SUMS 不含 os 关键词或扩展名不符，均不应命中
        let assets = vec![
            asset("quota-watch-0.1.0-1.x86_64.rpm"),
            asset("quota-watch_0.1.0-1_amd64.deb"),
            asset("SHA256SUMS.txt"),
        ];
        if os_keyword() != "windows" {
            assert!(select_platform_asset(&assets).is_none());
        }
    }

    #[test]
    fn extract_binary_unpacks_tar_xz() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("qw-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive_path = dir.join("pack.tar.xz");
        let dest = dir.join("binary-out");

        // 构造与真实 Release 相同结构的 tar.xz：单文件 quota-watch
        let payload = b"#!/bin/sh\necho fake-binary\n";
        let tar_bytes = {
            let mut builder = tar::Builder::new(Vec::new());
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "quota-watch", &payload[..])
                .unwrap();
            builder.into_inner().unwrap()
        };
        let encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
        let mut compressed = encoder;
        compressed.write_all(&tar_bytes).unwrap();
        std::fs::write(&archive_path, compressed.finish().unwrap()).unwrap();

        extract_binary(&archive_path, &dest).expect("解压应成功");
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_binary_fails_without_binary_entry() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join(format!("qw-test-neg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive_path = dir.join("pack.tar.xz");
        let dest = dir.join("binary-out");

        let tar_bytes = {
            let mut builder = tar::Builder::new(Vec::new());
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "README.txt", &b"hello"[..])
                .unwrap();
            builder.into_inner().unwrap()
        };
        let mut compressed = xz2::write::XzEncoder::new(Vec::new(), 1);
        compressed.write_all(&tar_bytes).unwrap();
        std::fs::write(&archive_path, compressed.finish().unwrap()).unwrap();

        assert!(extract_binary(&archive_path, &dest).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_platform_asset_returns_none_when_no_match() {
        let assets = vec![asset("quota-watch-armv7-unknown-freebsd.tar.xz")];
        assert!(select_platform_asset(&assets).is_none());
        assert!(select_platform_asset(&[]).is_none());
    }
}
