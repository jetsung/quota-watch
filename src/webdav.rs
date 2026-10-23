//! WebDAV 手动同步客户端（PROPFIND / GET / PUT / DELETE）。
//!
//! 仅封装配置同步所需的四种操作，不引入完整 WebDAV crate。
//! 网络调用使用 `reqwest::blocking`，由 UI 层放在后台执行器中运行。

use crate::models::WebdavConfig;
use anyhow::{Context as _, anyhow};
use chrono::{DateTime, Local, Utc};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode, Url};
use std::time::Duration;

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getlastmodified/>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#;

/// 远端配置文件条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub last_modified: Option<DateTime<Utc>>,
}

/// WebDAV 客户端：持有 HTTP 客户端与归一化后的远程目录 URL。
pub struct WebdavClient {
    client: Client,
    base_url: Url,
    username: String,
    password: String,
}

impl WebdavClient {
    pub fn new(config: &WebdavConfig) -> anyhow::Result<Self> {
        let server = config.server_url.trim();
        if server.is_empty() {
            anyhow::bail!("请先配置 WebDAV 服务器地址");
        }
        let dir = config.remote_dir.trim().trim_matches('/');
        let mut base = server.trim_end_matches('/').to_string();
        if !dir.is_empty() {
            base.push('/');
            base.push_str(dir);
        }
        base.push('/');
        let base_url = Url::parse(&base).with_context(|| format!("WebDAV 地址无效: {}", base))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            anyhow::bail!("WebDAV 地址必须以 http:// 或 https:// 开头");
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("quota-watch/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("创建 HTTP 客户端失败")?;
        Ok(Self {
            client,
            base_url,
            username: config.username.trim().to_string(),
            password: config.password.clone(),
        })
    }

    fn with_auth(&self, request: RequestBuilder) -> RequestBuilder {
        if self.username.is_empty() && self.password.is_empty() {
            request
        } else {
            request.basic_auth(&self.username, Some(&self.password))
        }
    }

    fn file_url(&self, filename: &str) -> anyhow::Result<Url> {
        self.base_url
            .join(&encode_path_segment(filename))
            .with_context(|| format!("拼接远程文件地址失败: {}", filename))
    }

    /// 测试连接：对配置的远程目录执行 PROPFIND `Depth: 0`，状态成功即可用。
    pub fn test_connection(&self) -> anyhow::Result<()> {
        let request = self
            .client
            .request(propfind_method(), self.base_url.clone())
            .header("Depth", "0")
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY);
        let response = self
            .with_auth(request)
            .send()
            .context("连接 WebDAV 服务器失败")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(http_error("测试连接", status, &body));
        }
        Ok(())
    }

    /// 确保远程目录存在（MKCOL）。目录已存在时服务器返回 405 等状态，一律忽略，
    /// 真正的问题会在后续列表 / 上传操作中暴露。
    fn ensure_directory(&self) {
        let request = self.client.request(
            Method::from_bytes(b"MKCOL").expect("MKCOL 是合法 HTTP 方法"),
            self.base_url.clone(),
        );
        let _ = self.with_auth(request).send();
    }

    /// PROPFIND `Depth: 1` 列出远程目录中的配置文件（按修改时间降序）。
    pub fn list_files(&self) -> anyhow::Result<Vec<RemoteFile>> {
        self.ensure_directory();
        let request = self
            .client
            .request(propfind_method(), self.base_url.clone())
            .header("Depth", "1")
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .body(PROPFIND_BODY);
        let response = self
            .with_auth(request)
            .send()
            .context("连接 WebDAV 服务器失败")?;
        let status = response.status();
        let body = response.text().context("读取 WebDAV 响应失败")?;
        if !status.is_success() {
            return Err(http_error("拉取云端列表", status, &body));
        }
        parse_propfind(&body)
    }

    /// GET 下载指定文件内容。
    pub fn download_file(&self, filename: &str) -> anyhow::Result<Vec<u8>> {
        let url = self.file_url(filename)?;
        let response = self
            .with_auth(self.client.get(url))
            .send()
            .with_context(|| format!("下载 {} 失败：连接服务器失败", filename))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(http_error(&format!("下载 {}", filename), status, &body));
        }
        let bytes = response
            .bytes()
            .with_context(|| format!("读取 {} 内容失败", filename))?;
        Ok(bytes.to_vec())
    }

    /// PUT 上传文件（调用方负责生成不重名的时间戳文件名）。
    pub fn upload_file(&self, filename: &str, content: &[u8]) -> anyhow::Result<()> {
        self.ensure_directory();
        let url = self.file_url(filename)?;
        let request = self
            .client
            .put(url)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(content.to_vec());
        self.expect_success(request, &format!("上传 {}", filename))
    }

    /// DELETE 删除指定远程文件。
    pub fn delete_file(&self, filename: &str) -> anyhow::Result<()> {
        let url = self.file_url(filename)?;
        let request = self.client.delete(url);
        self.expect_success(request, &format!("删除 {}", filename))
    }

    fn expect_success(&self, request: RequestBuilder, action: &str) -> anyhow::Result<()> {
        let response = self
            .with_auth(request)
            .send()
            .with_context(|| format!("{} 失败：连接服务器失败", action))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(http_error(action, status, &body));
        }
        Ok(())
    }
}

/// 生成 `quota-watch_%Y%m%d-%H%M%S.json` 形式的上传文件名。
pub fn timestamped_filename() -> String {
    format!("quota-watch_{}.json", Local::now().format("%Y%m%d-%H%M%S"))
}

/// 云端备份中加密密码字段的前缀标记（`enc:v1:<base64(nonce||ciphertext)>`）。
const PASSWORD_PREFIX: &str = "enc:v1:";

/// 内嵌于二进制的对称密钥。目的：避免 WebDAV 密码以明文出现在云端备份中
/// （云服务商或拿到备份文件者无法直接读取密码）；密钥随程序分发，
/// 不防御能反编译本程序的攻击者。
const CIPHER_KEY: [u8; 32] = [
    0x2c, 0x4d, 0xed, 0xe9, 0x48, 0x88, 0x19, 0xd5, 0x5e, 0xc9, 0x3e, 0x75, 0xeb, 0x0e, 0x1c, 0x39,
    0xef, 0xe3, 0x96, 0xb7, 0x6e, 0x37, 0xe6, 0x5a, 0x83, 0xd3, 0xaf, 0x03, 0xdb, 0xce, 0x50, 0xe6,
];

/// 加密单个密码：随机 nonce 的 XChaCha20-Poly1305，输出带前缀的 base64。
fn encrypt_password(plain: &str) -> anyhow::Result<String> {
    use base64::Engine as _;
    use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
    use chacha20poly1305::{AeadCore, XChaCha20Poly1305};
    let cipher = XChaCha20Poly1305::new((&CIPHER_KEY).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plain.as_bytes())
        .map_err(|_| anyhow!("加密 WebDAV 密码失败"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(format!(
        "{}{}",
        PASSWORD_PREFIX,
        base64::engine::general_purpose::STANDARD.encode(blob)
    ))
}

/// 解密带前缀的密码；无前缀视为历史明文备份，原样返回。
fn decrypt_password(stored: &str) -> anyhow::Result<String> {
    let Some(payload) = stored.strip_prefix(PASSWORD_PREFIX) else {
        return Ok(stored.to_string());
    };
    use base64::Engine as _;
    use chacha20poly1305::XChaCha20Poly1305;
    use chacha20poly1305::aead::generic_array::GenericArray;
    use chacha20poly1305::aead::generic_array::typenum::Unsigned as _;
    use chacha20poly1305::aead::{Aead, AeadCore, KeyInit};
    let blob = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|_| anyhow!("解密 WebDAV 密码失败"))?;
    let nonce_len = <XChaCha20Poly1305 as AeadCore>::NonceSize::to_usize();
    if blob.len() < nonce_len {
        anyhow::bail!("解密 WebDAV 密码失败（文件损坏）");
    }
    let (nonce, ciphertext) = blob.split_at(nonce_len);
    let cipher = XChaCha20Poly1305::new((&CIPHER_KEY).into());
    let plain = cipher
        .decrypt(GenericArray::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("解密 WebDAV 密码失败（密钥不匹配或文件损坏）"))?;
    String::from_utf8(plain).map_err(|_| anyhow!("解密 WebDAV 密码失败"))
}

/// 上传前调用：将持久化结构中的 WebDAV 密码替换为加密形式。
pub fn encrypt_persisted(persisted: &mut crate::models::PersistedState) -> anyhow::Result<()> {
    if let Some(config) = persisted.webdav_config.as_mut()
        && !config.password.is_empty()
    {
        config.password = encrypt_password(&config.password)?;
    }
    Ok(())
}

/// 恢复/导入前调用：解密云端备份中的密码（历史明文备份原样保留）。
pub fn decrypt_persisted(persisted: &mut crate::models::PersistedState) -> anyhow::Result<()> {
    if let Some(config) = persisted.webdav_config.as_mut() {
        config.password = decrypt_password(&config.password)?;
    }
    Ok(())
}

fn propfind_method() -> Method {
    Method::from_bytes(b"PROPFIND").expect("PROPFIND 是合法 HTTP 方法")
}

fn http_error(action: &str, status: StatusCode, body: &str) -> anyhow::Error {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return anyhow!(
            "{}失败：认证失败或权限不足（HTTP {}）",
            action,
            status.as_u16()
        );
    }
    let detail: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let detail: String = detail.chars().take(200).collect();
    if detail.is_empty() {
        anyhow!("{}失败：HTTP {}", action, status.as_u16())
    } else {
        anyhow!("{}失败：HTTP {}（{}）", action, status.as_u16(), detail)
    }
}

/// 解析 PROPFIND Multi-Status 响应：按 localname 匹配，忽略命名空间前缀差异。
fn parse_propfind(xml: &str) -> anyhow::Result<Vec<RemoteFile>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut files = Vec::new();
    let mut in_response = false;
    let mut current_tag: Option<Vec<u8>> = None;
    let mut href: Option<String> = None;
    let mut modified: Option<DateTime<Utc>> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                let name = event.local_name().as_ref().to_vec();
                if name == b"response" {
                    in_response = true;
                    href = None;
                    modified = None;
                }
                current_tag = Some(name);
            }
            Ok(Event::Text(event)) if in_response => {
                let tag = current_tag.take();
                if let Some(tag) = tag {
                    let text = event.decode().ok().map(|raw| {
                        unescape(&raw)
                            .map(|text| text.into_owned())
                            .unwrap_or_else(|_| raw.into_owned())
                            .trim()
                            .to_string()
                    });
                    if let Some(text) = text {
                        match tag.as_slice() {
                            b"href" => href = Some(text),
                            b"getlastmodified" => modified = parse_http_date(&text),
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(event)) => {
                if event.local_name().as_ref() == b"response" && in_response {
                    if let Some(href) = href.take()
                        && let Some(name) = filename_from_href(&href)
                        && is_services_file(&name)
                    {
                        files.push(RemoteFile {
                            name,
                            last_modified: modified.take(),
                        });
                    }
                    in_response = false;
                    modified = None;
                }
                current_tag = None;
            }
            Ok(Event::Empty(_)) => current_tag = None,
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("解析 WebDAV 响应失败: {}", e)),
            _ => {}
        }
    }

    // 最新在前；无时间的条目排在最后
    files.sort_by_key(|file| std::cmp::Reverse(file.last_modified));
    Ok(files)
}

fn parse_http_date(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(text)
        .ok()
        .or_else(|| DateTime::parse_from_rfc3339(text).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn filename_from_href(href: &str) -> Option<String> {
    let path = href.split(['?', '#']).next().unwrap_or(href);
    // 目录 href 以 '/' 结尾，不作为配置文件
    if path.ends_with('/') {
        return None;
    }
    let last = path.rsplit('/').next()?;
    if last.is_empty() {
        return None;
    }
    Some(percent_decode(last))
}

/// 云端配置文件识别：当前前缀 `quota-watch`，兼容历史 `services` 前缀备份。
fn is_services_file(name: &str) -> bool {
    (name.starts_with("quota-watch") || name.starts_with("services")) && name.ends_with(".json")
}

fn encode_path_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const PROPFIND_RESPONSE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/quota-watch/</d:href>
    <d:propstat>
      <d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/quota-watch/quota-watch_20261022-013000.json</d:href>
    <d:propstat>
      <d:prop><d:getlastmodified>Thu, 22 Oct 2026 01:30:00 GMT</d:getlastmodified></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/quota-watch/services_20261021-120000.json</d:href>
    <d:propstat>
      <d:prop><d:getlastmodified>Wed, 21 Oct 2026 12:00:00 GMT</d:getlastmodified></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/quota-watch/other.json</d:href>
    <d:propstat>
      <d:prop><d:getlastmodified>Wed, 21 Oct 2026 12:00:00 GMT</d:getlastmodified></d:prop>
      <d:status>HTTP/1.1 200 OK</d:status>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn parse_propfind_extracts_services_files_sorted_desc() {
        let files = parse_propfind(PROPFIND_RESPONSE).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "quota-watch_20261022-013000.json");
        assert_eq!(files[1].name, "services_20261021-120000.json");
        assert_eq!(
            files[0].last_modified.unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 22, 1, 30, 0).unwrap()
        );
    }

    #[test]
    fn parse_propfind_without_last_modified_still_lists_file() {
        let xml = r#"<?xml version="1.0"?>
<multistatus xmlns="DAV:">
  <response>
    <href>/dav/quota-watch/services%20copy.json</href>
    <propstat><prop><displayname>services copy.json</displayname></prop></propstat>
  </response>
</multistatus>"#;
        let files = parse_propfind(xml).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "services copy.json");
        assert!(files[0].last_modified.is_none());
    }

    #[test]
    fn parse_propfind_ignores_unrelated_files() {
        let xml = r#"<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/dav/quota-watch/notes.txt</d:href></d:response>
  <d:response><d:href>/dav/quota-watch/backup.json</d:href></d:response>
</d:multistatus>"#;
        assert!(parse_propfind(xml).unwrap().is_empty());
    }

    #[test]
    fn filename_from_href_decodes_percent_encoding() {
        assert_eq!(
            filename_from_href("/dav/dir/services%202026.json").as_deref(),
            Some("services 2026.json")
        );
        assert_eq!(filename_from_href("/dav/dir/"), None);
    }

    #[test]
    fn timestamped_filename_matches_pattern() {
        let name = timestamped_filename();
        assert!(name.starts_with("quota-watch_"));
        assert!(name.ends_with(".json"));
        assert_eq!(name.len(), "quota-watch_20261022-013000.json".len());
    }

    #[test]
    fn client_rejects_empty_server_url() {
        let config = WebdavConfig::default();
        assert!(WebdavClient::new(&config).is_err());
    }

    #[test]
    fn client_normalizes_remote_dir() {
        let config = WebdavConfig {
            server_url: "https://dav.example.com/".into(),
            remote_dir: "/quota-watch/".into(),
            ..Default::default()
        };
        let client = WebdavClient::new(&config).unwrap();
        assert_eq!(
            client.base_url.as_str(),
            "https://dav.example.com/quota-watch/"
        );
    }

    #[test]
    fn password_encrypt_decrypt_roundtrip() {
        let encrypted = encrypt_password("s3cr3t-密码").unwrap();
        assert!(encrypted.starts_with(PASSWORD_PREFIX));
        assert!(!encrypted.contains("s3cr3t"));
        assert_eq!(decrypt_password(&encrypted).unwrap(), "s3cr3t-密码");
        // 每次加密使用不同 nonce，密文不同但均可解密
        let other = encrypt_password("s3cr3t-密码").unwrap();
        assert_ne!(other, encrypted);
        assert_eq!(decrypt_password(&other).unwrap(), "s3cr3t-密码");
    }

    #[test]
    fn plaintext_password_passthrough() {
        assert_eq!(decrypt_password("legacy-plain").unwrap(), "legacy-plain");
    }

    #[test]
    fn persisted_config_password_roundtrip() {
        let mut persisted = crate::models::PersistedState {
            webdav_config: Some(WebdavConfig {
                server_url: "https://dav.example.com".into(),
                username: "u".into(),
                password: "topsecret".into(),
                remote_dir: "/quota-watch".into(),
            }),
            ..Default::default()
        };
        encrypt_persisted(&mut persisted).unwrap();
        assert!(
            persisted
                .webdav_config
                .as_ref()
                .unwrap()
                .password
                .starts_with(PASSWORD_PREFIX)
        );
        decrypt_persisted(&mut persisted).unwrap();
        assert_eq!(persisted.webdav_config.unwrap().password, "topsecret");
    }

    #[test]
    fn tampered_ciphertext_rejected() {
        let encrypted = encrypt_password("abc").unwrap();
        let tampered = format!("{}AA", encrypted.trim_end_matches('='));
        assert!(decrypt_password(&tampered).is_err());
    }

    /// 模拟「上传 PUT 字节 → 恢复 GET 解析 → 解密回写」完整数据流。
    #[test]
    fn upload_download_pipeline_roundtrip() {
        use crate::models::PersistedState;
        let secret = "s3cr3t-webdav-pw";
        let mut persisted = PersistedState {
            webdav_config: Some(WebdavConfig {
                server_url: "https://dav.example.com/dav".into(),
                username: "u".into(),
                password: secret.into(),
                remote_dir: "/quota-watch".into(),
            }),
            ..Default::default()
        };
        // 上传侧：加密后序列化的字节不得含明文密码
        encrypt_persisted(&mut persisted).unwrap();
        let bytes = serde_json::to_vec_pretty(&persisted).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        // 恢复侧：解析字节并解密，密码还原
        let mut parsed: PersistedState = serde_json::from_slice(&bytes).unwrap();
        decrypt_persisted(&mut parsed).unwrap();
        assert_eq!(parsed.webdav_config.unwrap().password, secret);
    }
}
