use crate::models::{self, AppState, PersistedState, WebdavConfig};
use crate::ui::checkbox::render_checkbox;
use crate::ui::toast::{self, ToastState};
use crate::update;
use crate::webdav::{self, RemoteFile, WebdavClient};
use anyhow::Context as _;
use chrono::Local;
use gpui_kit::base::{Selectable as _, h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Entity, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Window, div, px,
};
use std::io::Write;
use std::path::Path;

/// 设置面板左侧导航的选项卡。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Notifications,
    BackupSync,
    ImportExport,
    About,
}

impl SettingsTab {
    const ALL: [Self; 4] = [
        Self::Notifications,
        Self::BackupSync,
        Self::ImportExport,
        Self::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Notifications => "桌面通知",
            Self::BackupSync => "备份与恢复",
            Self::ImportExport => "导入与导出",
            Self::About => "关于",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Notifications => "settings-tab-notifications",
            Self::BackupSync => "settings-tab-backup-sync",
            Self::ImportExport => "settings-tab-import-export",
            Self::About => "settings-tab-about",
        }
    }
}

/// 当前进行中的 WebDAV 操作（用于按钮 loading 与全局禁用）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Busy {
    List,
    Upload,
    Test,
    Download(String),
    Delete(String),
}

/// 设置面板：通知开关、WebDAV 连接与手动同步、配置导入导出。
pub struct SettingsView {
    state: Entity<AppState>,
    server_url: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    remote_dir: Entity<InputState>,
    active_tab: SettingsTab,
    busy: Option<Busy>,
    /// None = 尚未拉取；Some(empty) = 云端无配置文件。
    remote_files: Option<Vec<RemoteFile>>,
    checking_update: bool,
    /// 是否正在执行自升级（“立即升级”点击后到结果回调前）。
    upgrading: bool,
    /// 升级中的取消标志（“取消”按钮置位，后台下载在检查点中止）。
    upgrade_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    error: Option<SharedString>,
    toast: Option<ToastState>,
    toast_seq: u64,
}

fn field_label(text: impl Into<SharedString>) -> impl IntoElement {
    div().text_size(px(13.)).mb_1().child(text.into())
}

/// 设置页按钮统一使用手型光标（组件库默认按钮为系统箭头）。
fn pointer_button(id: impl Into<gpui_kit::ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}

/// 远程目录留空时使用的默认目录（项目专属子目录）。
const DEFAULT_REMOTE_DIR: &str = "/quota-watch";

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, state: Entity<AppState>) -> Self {
        let config = state.read(cx).webdav_config.clone().unwrap_or_default();

        let view = Self {
            state,
            server_url: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(config.server_url.clone())
                    .placeholder("https://dav.example.com/dav")
            }),
            username: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(config.username.clone())
                    .placeholder("用户名")
            }),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(config.password.clone())
                    .placeholder("密码")
            }),
            remote_dir: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(config.remote_dir.clone())
                    .placeholder("/quota-watch")
            }),
            active_tab: SettingsTab::Notifications,
            busy: None,
            remote_files: None,
            checking_update: false,
            upgrading: false,
            upgrade_cancel: None,
            error: None,
            toast: None,
            toast_seq: 0,
        };
        // 外部状态变更（如导入覆盖、通知开关同步）后刷新面板
        cx.subscribe(&view.state, |_, _, _: &models::AppStateChanged, cx| {
            cx.notify();
        })
        .detach();
        view
    }

    fn form_config(&self, cx: &App) -> WebdavConfig {
        WebdavConfig {
            server_url: self.server_url.read(cx).value().trim().to_string(),
            username: self.username.read(cx).value().trim().to_string(),
            password: self.password.read(cx).value().to_string(),
            remote_dir: self.remote_dir.read(cx).value().trim().to_string(),
        }
    }

    fn stored_form_config(&self, cx: &App) -> Option<WebdavConfig> {
        let config = self.form_config(cx);
        if config.server_url.is_empty()
            && config.username.is_empty()
            && config.password.is_empty()
            && config.remote_dir.is_empty()
        {
            None
        } else {
            Some(config)
        }
    }

    fn fill_inputs(&self, config: &WebdavConfig, window: &mut Window, cx: &mut App) {
        let values: [(&Entity<InputState>, &String); 4] = [
            (&self.server_url, &config.server_url),
            (&self.username, &config.username),
            (&self.password, &config.password),
            (&self.remote_dir, &config.remote_dir),
        ];
        for (input, value) in values {
            input.update(cx, |state, cx| {
                state.set_value(value.clone(), window, cx);
            });
        }
    }

    fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast_internal(message, false, false, false, cx);
    }

    /// 显示带“立即升级”操作按钮的提示（检查更新发现新版本时），持久显示不自动关闭。
    fn show_upgrade_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast_internal(message, true, false, true, cx);
    }

    /// 升级成功后的持久提示，附“立即重启 / 稍后”按钮。
    fn show_restart_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast_internal(message, false, true, true, cx);
    }

    fn push_toast_internal(
        &mut self,
        message: impl Into<SharedString>,
        action_upgrade: bool,
        action_restart: bool,
        persistent: bool,
        cx: &mut Context<Self>,
    ) {
        self.toast_seq = self.toast_seq.wrapping_add(1);
        let seq = self.toast_seq;
        self.toast = Some(ToastState {
            seq,
            message: message.into(),
            exiting: false,
            action_upgrade,
            action_restart,
            action_cancel: false,
            persistent,
        });
        cx.notify();
        cx.spawn(async move |this, async_cx| {
            async_cx
                .background_executor()
                .timer(std::time::Duration::from_millis(2000))
                .await;
            let _ = async_cx.update(|cx| {
                this.update(cx, |view, cx| {
                    if view.toast.as_ref().map(|toast| toast.seq) == Some(seq) {
                        let persistent = view.toast.as_ref().unwrap().persistent;
                        if !persistent {
                            view.toast.as_mut().unwrap().exiting = true;
                            cx.notify();
                        }
                    }
                })
            });
            async_cx
                .background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = async_cx.update(|cx| {
                this.update(cx, |view, cx| {
                    if view.toast.as_ref().map(|toast| toast.seq) == Some(seq) {
                        let persistent = view.toast.as_ref().unwrap().persistent;
                        if !persistent {
                            view.toast = None;
                            cx.notify();
                        }
                    }
                })
            });
        })
        .detach();
    }

    fn set_error(
        &mut self,
        message: impl Into<SharedString>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        self.error = Some(message.clone());
        self.show_toast(message, cx);
    }

    /// 校验表单并返回可用连接配置；地址为空时提示先完成配置。
    fn require_config(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<WebdavConfig> {
        let mut config = self.form_config(cx);
        if config.server_url.is_empty() {
            self.show_toast("请先填写并保存 WebDAV 服务器地址", cx);
            return None;
        }
        if config.remote_dir.is_empty() {
            config.remote_dir = DEFAULT_REMOTE_DIR.to_string();
        }
        Some(config)
    }

    /// 保存 WebDAV 连接配置到本地并持久化。
    fn save_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.form_config(cx);
        if !config.server_url.is_empty()
            && !(config.server_url.starts_with("http://")
                || config.server_url.starts_with("https://"))
        {
            self.set_error("服务器地址必须以 http:// 或 https:// 开头", window, cx);
            return;
        }
        // 远程目录置空时回填项目专属目录，避免备份上传到服务器根目录与其它项目混在一起
        if !config.server_url.is_empty() && config.remote_dir.is_empty() {
            self.remote_dir.update(cx, |state, cx| {
                state.set_value(DEFAULT_REMOTE_DIR.to_string(), window, cx);
            });
        }
        let stored = self.stored_form_config(cx);
        let state = self.state.clone();
        let entity = cx.entity();
        let result = models::mutate(
            &state,
            cx,
            move |app_state, _| {
                app_state.webdav_config = stored.clone();
                Ok(())
            },
            move |message, cx| {
                let entity = entity.clone();
                cx.defer(move |cx| {
                    entity.update(cx, |view, cx| view.show_toast(message, cx));
                });
            },
        );
        if result.is_some() {
            self.error = None;
            self.show_toast("WebDAV 配置已保存", cx);
        }
    }

    /// 测试 WebDAV 连接：PROPFIND Depth 0，成功后提示连接可用。
    fn start_test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.require_config(window, cx) else {
            return;
        };
        self.busy = Some(Busy::Test);
        self.error = None;
        self.show_toast("正在测试连接…", cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    WebdavClient::new(&config).and_then(|client| client.test_connection())
                })
                .await;
            match result {
                Ok(()) => {
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.show_toast("WebDAV 连接可用", cx);
                    });
                }
                Err(e) => {
                    let message = format!("{:#}", e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.error = Some(message.clone().into());
                        view.show_toast(message, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// 拉取云端配置文件列表（后台执行，不阻塞 UI）。
    fn start_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.require_config(window, cx) else {
            return;
        };
        self.busy = Some(Busy::List);
        self.error = None;
        self.show_toast("正在获取云端列表…", cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        WebdavClient::new(&config).and_then(|client| client.list_files())
                    })
                    .await;
            match result {
                Ok(files) => {
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.remote_files = Some(files);
                        view.show_toast("云端列表已更新", cx);
                    });
                }
                Err(e) => {
                    let message = format!("{:#}", e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.error = Some(message.clone().into());
                        view.show_toast(message, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// 上传当前配置，文件名带时间戳以保留历史版本。
    fn start_upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.require_config(window, cx) else {
            return;
        };
        let mut persisted = self.state.read(cx).to_persisted();
        persisted.webdav_config = Some(config.clone());
        // 云端备份不落明文密码
        if let Err(e) = webdav::encrypt_persisted(&mut persisted) {
            self.set_error(format!("{:#}", e), window, cx);
            return;
        }
        let content = match serde_json::to_vec_pretty(&persisted) {
            Ok(content) => content,
            Err(e) => {
                self.set_error(format!("序列化配置失败: {}", e), window, cx);
                return;
            }
        };
        let filename = webdav::timestamped_filename();
        let uploaded = filename.clone();
        self.busy = Some(Busy::Upload);
        self.error = None;
        self.show_toast("正在上传配置…", cx);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = WebdavClient::new(&config)?;
                    client.upload_file(&filename, &content)
                })
                .await;
            match result {
                Ok(()) => {
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.show_toast(format!("已上传 {}", uploaded), cx);
                    });
                }
                Err(e) => {
                    let message = format!("{:#}", e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.error = Some(message.clone().into());
                        view.show_toast(message, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// 恢复单个云端文件（覆盖前确认）。
    fn confirm_download(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, win, _| {
            let entity = entity.clone();
            let name = name.clone();
            let description = format!("将下载 “{}” 并覆盖当前配置，确定继续？", name);
            // 文件名较长，描述用小一号字体避免折行
            let description = div().text_size(px(12.)).child(description);
            // 叠加在设置对话框之上时 layer 偏移为 16px，居中计算需一并扣除
            let margin_top = ((win.viewport_size().height - px(170.)) / 2. - px(16.)).max(px(24.));
            alert
                .margin_top(margin_top)
                .title("恢复云端配置")
                .description(description)
                .ok_text("确定恢复")
                .cancel_text("取消")
                .confirm()
                .on_ok(move |_, win, app| {
                    let name = name.clone();
                    entity.update(app, |view, cx| {
                        view.start_download(name, win, cx);
                    });
                    true
                })
        });
    }

    fn start_download(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.require_config(window, cx) else {
            return;
        };
        self.busy = Some(Busy::Download(name.clone()));
        self.error = None;
        self.show_toast(format!("正在恢复 {name}…"), cx);
        cx.notify();
        let state = self.state.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = WebdavClient::new(&config)?;
                    let bytes = client.download_file(&name)?;
                    let mut parsed: PersistedState =
                        serde_json::from_slice(&bytes).context("配置文件解析失败")?;
                    webdav::decrypt_persisted(&mut parsed)?;
                    Ok::<_, anyhow::Error>((parsed, name))
                })
                .await;
            match result {
                Ok((parsed, filename)) => {
                    let config = parsed.webdav_config.clone().unwrap_or_default();
                    let state = state.clone();
                    let _ = this.update_in(cx, |view, window, cx| {
                        let entity = cx.entity();
                        let next = parsed.clone();
                        let result = models::mutate(
                            &state,
                            cx,
                            move |app_state, _| {
                                *app_state = AppState::from_persisted(next.clone());
                                Ok(())
                            },
                            move |message, cx| {
                                let entity = entity.clone();
                                cx.defer(move |cx| {
                                    entity.update(cx, |view, cx| view.show_toast(message, cx));
                                });
                            },
                        );
                        view.busy = None;
                        if result.is_some() {
                            view.fill_inputs(&config, window, cx);
                            view.error = None;
                            view.show_toast(format!("已恢复 {}", filename), cx);
                        }
                        cx.notify();
                    });
                }
                Err(e) => {
                    let message = format!("{:#}", e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.error = Some(message.clone().into());
                        view.show_toast(message, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// 删除单个云端文件（确认后删除并刷新列表）。
    fn confirm_delete(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, win, _| {
            let entity = entity.clone();
            let name = name.clone();
            let description = format!("确定从云端删除 “{}”？本地配置不受影响。", name);
            // 文件名较长，描述用小一号字体避免折行
            let description = div().text_size(px(12.)).child(description);
            // 叠加在设置对话框之上时 layer 偏移为 16px，居中计算需一并扣除
            let margin_top = ((win.viewport_size().height - px(170.)) / 2. - px(16.)).max(px(24.));
            alert
                .margin_top(margin_top)
                .title("删除云端配置")
                .description(description)
                .ok_text("确定删除")
                .cancel_text("取消")
                .confirm()
                .on_ok(move |_, win, app| {
                    let name = name.clone();
                    entity.update(app, |view, cx| {
                        view.start_delete(name, win, cx);
                    });
                    true
                })
        });
    }

    fn start_delete(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.require_config(window, cx) else {
            return;
        };
        self.busy = Some(Busy::Delete(name.clone()));
        self.error = None;
        self.show_toast(format!("正在删除 {name}…"), cx);
        cx.notify();
        let deleted = name.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = WebdavClient::new(&config)?;
                    client.delete_file(&name)?;
                    Ok::<_, anyhow::Error>(client.list_files())
                })
                .await;
            match result {
                Ok(Ok(files)) => {
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.remote_files = Some(files);
                        view.show_toast(format!("已删除 {}", deleted), cx);
                    });
                }
                Ok(Err(e)) => {
                    let message = format!("已删除 {}，但刷新列表失败: {:#}", deleted, e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.remote_files = None;
                        view.show_toast(message, cx);
                    });
                }
                Err(e) => {
                    let message = format!("{:#}", e);
                    let _ = this.update_in(cx, |view, _window, cx| {
                        view.busy = None;
                        view.error = Some(message.clone().into());
                        view.show_toast(message, cx);
                    });
                }
            }
        })
        .detach();
    }

    /// 从本地 JSON 文件导入配置（覆盖前确认）。
    fn import_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON 配置", &["json"])
            .pick_file()
        else {
            return;
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                self.set_error(format!("读取文件失败: {}", e), window, cx);
                return;
            }
        };
        let mut parsed: PersistedState = match serde_json::from_str(&content) {
            Ok(parsed) => parsed,
            Err(e) => {
                self.set_error(format!("配置文件解析失败: {}", e), window, cx);
                return;
            }
        };
        // 兼容云端下载的加密备份
        if let Err(e) = webdav::decrypt_persisted(&mut parsed) {
            self.set_error(format!("{:#}", e), window, cx);
            return;
        }
        let state = self.state.clone();
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let state = state.clone();
            let entity = entity.clone();
            let parsed = parsed.clone();
            let config = parsed.webdav_config.clone().unwrap_or_default();
            alert
                .title("导入配置")
                .description(
                    "导入将覆盖当前配置（服务列表、通知开关与 WebDAV 连接配置），确定继续？",
                )
                .ok_text("确定导入")
                .cancel_text("取消")
                .confirm()
                .on_ok(move |_, win, app| {
                    let next = parsed.clone();
                    let error_entity = entity.clone();
                    let result = models::mutate(
                        &state,
                        app,
                        move |app_state, _| {
                            *app_state = AppState::from_persisted(next.clone());
                            Ok(())
                        },
                        move |message, app| {
                            error_entity.update(app, |view, cx| view.show_toast(message, cx));
                        },
                    );
                    if result.is_some() {
                        entity.update(app, |view, cx| {
                            view.fill_inputs(&config, win, cx);
                            view.remote_files = None;
                            view.error = None;
                            view.show_toast("配置已导入", cx);
                        });
                    }
                    true
                })
        });
    }

    /// 导出当前配置为本地 JSON 文件（文件名带时间戳，原子写入）。
    fn export_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut persisted = self.state.read(cx).to_persisted();
        persisted.webdav_config = self.stored_form_config(cx);
        let json = match serde_json::to_string_pretty(&persisted) {
            Ok(json) => json,
            Err(e) => {
                self.set_error(format!("序列化配置失败: {}", e), window, cx);
                return;
            }
        };
        let filename = webdav::timestamped_filename();
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&filename)
            .add_filter("JSON 配置", &["json"])
            .save_file()
        else {
            return;
        };
        match write_json_atomic(&path, &json) {
            Ok(()) => {
                self.error = None;
                self.show_toast(format!("配置已导出到 {}", path.display()), cx);
            }
            Err(e) => {
                self.set_error(format!("导出失败: {:#}", e), window, cx);
            }
        }
    }

    /// 检查更新：请求 GitHub Releases API 并对比当前版本（自更新后续由 `--self-update` 提供）。
    fn check_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking_update {
            return;
        }
        self.checking_update = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { update::check_latest() })
                .await;
            let _ = this.update_in(cx, |view, _window, cx| {
                view.checking_update = false;
                match result {
                    Ok(release) => {
                        if update::is_newer(&release.version, update::CURRENT_VERSION) {
                            view.show_upgrade_toast(
                                format!(
                                    "发现新版本！\n当前版本: v{}\n最新版本: v{}",
                                    update::CURRENT_VERSION,
                                    release.version
                                ),
                                cx,
                            );
                        } else {
                            view.show_toast(
                                format!("已是最新版本 v{}", update::CURRENT_VERSION),
                                cx,
                            );
                        }
                    }
                    Err(e) => {
                        view.show_toast(format!("检查更新失败：{:#}", e), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// “立即升级”：后台执行与 `self-update` 子命令相同的升级流程，
    /// 经共享进度状态轮询刷新持久 Toast，结果回调到 UI；升级中可点“取消”中断。
    fn start_upgrade(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

        if self.upgrading {
            return;
        }
        self.upgrading = true;
        // 当前提示改为持久进度提示，并移除按钮避免重复触发
        if let Some(toast) = self.toast.as_mut() {
            toast.message = "正在下载升级包…".into();
            toast.action_upgrade = false;
            toast.action_cancel = true;
            toast.persistent = true;
        }
        cx.notify();

        // 进度共享状态：0=下载中 1=解压 2=替换；percent 为下载百分比；cancelled 为用户取消标志
        let phase = Arc::new(AtomicU8::new(0));
        let percent = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        self.upgrade_cancel = Some(cancelled.clone());
        let (p_phase, p_percent, p_done) = (phase.clone(), percent.clone(), done.clone());
        cx.spawn_in(window, async move |this, cx| {
            let task = cx.background_executor().spawn(async move {
                let result = update::self_update_with_progress(
                    &|| cancelled.load(Ordering::Relaxed),
                    &mut |progress| match progress {
                        update::UpdateProgress::Downloading { .. } => {
                            p_phase.store(0, Ordering::Relaxed);
                            p_percent
                                .store(progress.percent().unwrap_or(0) as u64, Ordering::Relaxed);
                        }
                        update::UpdateProgress::Extracting => p_phase.store(1, Ordering::Relaxed),
                        update::UpdateProgress::Replacing => p_phase.store(2, Ordering::Relaxed),
                    },
                );
                p_done.store(true, Ordering::Relaxed);
                result
            });
            // 轮询进度并刷新持久 Toast 文案，任务结束后展示最终结果
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let finished = done.load(Ordering::Relaxed);
                let message = match phase.load(Ordering::Relaxed) {
                    0 => {
                        format!("正在下载升级包… {}%", percent.load(Ordering::Relaxed))
                    }
                    1 => "下载完成，正在解压…".to_string(),
                    _ => "正在替换可执行文件…".to_string(),
                };
                let _ = this.update_in(cx, |view, _window, cx| {
                    if let Some(toast) = view.toast.as_mut()
                        && toast.persistent
                    {
                        toast.message = message.into();
                    }
                    cx.notify();
                });
                if finished {
                    break;
                }
            }
            let result = task.await;
            let _ = this.update_in(cx, |view, _window, cx| {
                view.upgrading = false;
                view.upgrade_cancel = None;
                let cancelled = result
                    .as_ref()
                    .err()
                    .is_some_and(|e| e.is::<update::UpdateCancelled>());
                match result {
                    Ok(update::UpdateOutcome::Updated) => {
                        view.show_restart_toast("升级成功！", cx);
                    }
                    Ok(update::UpdateOutcome::AlreadyUpToDate) => {
                        view.show_toast(format!("已是最新版本 v{}", update::CURRENT_VERSION), cx);
                    }
                    // 用户主动取消：直接关闭进度提示，不提示失败
                    Err(e) if cancelled => {
                        view.toast = None;
                        let _ = e;
                    }
                    Err(e) => {
                        view.show_toast(format!("升级失败：{:#}", e), cx);
                    }
                }
            });
        })
        .detach();
    }

    /// 升级中“取消”：置位取消标志，立即关闭进度提示并恢复界面；
    /// 后台下载在下一个检查点自行中止（未替换前原文件不受影响）。
    fn cancel_upgrade(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::Ordering;
        if let Some(flag) = &self.upgrade_cancel {
            flag.store(true, Ordering::Relaxed);
        }
        self.toast = None;
        self.upgrading = false;
        cx.notify();
    }

    /// “立即重启”：脱离当前会话延迟拉起新版本实例，再退出当前应用。
    /// 直接 spawn 子进程会随旧实例的进程组一并被终止，因此用 setsid（Windows 用 detached 标志）脱离。
    fn restart_app(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // 升级替换后 current_exe 会带 “ (deleted)” 后缀，需规范化后才能启动新实例
        if let Ok(exe) = update::current_exe_path() {
            let exe = exe.display().to_string();
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                let _ = std::process::Command::new(&exe)
                    .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
                    .spawn();
            }
            #[cfg(not(target_os = "windows"))]
            {
                // 延迟 0.3s 启动，避开旧实例退出与新实例初始化的竞态
                let script = format!("sleep 0.3; exec '{exe}'");
                let _ = std::process::Command::new("setsid")
                    .arg("sh")
                    .arg("-c")
                    .arg(&script)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            }
        }
        self.toast = None;
        self.upgrading = false;
        cx.quit();
    }

    /// “稍后”：关闭当前持久提示（发现新版本 / 升级成功），保持当前实例。
    fn dismiss_restart_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
        cx.notify();
    }

    fn render_remote_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match &self.remote_files {
            None => div().into_any_element(),
            Some(files) if files.is_empty() => div()
                .w_full()
                .text_center()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child("云端无配置文件")
                .into_any_element(),
            Some(files) => {
                let rows: Vec<AnyElement> = files
                    .iter()
                    .map(|file| {
                        let name = file.name.clone();
                        let modified = file
                            .last_modified
                            .map(|time| {
                                time.with_timezone(&Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| "修改时间未知".to_string());
                        let downloading = self.busy.as_ref() == Some(&Busy::Download(name.clone()));
                        let deleting = self.busy.as_ref() == Some(&Busy::Delete(name.clone()));
                        let any_busy = self.busy.is_some();
                        let sync_name = name.clone();
                        let delete_name = name.clone();
                        // 恢复中：保持条目原有背景、圆角与高度，仅将内容替换为居中文案
                        if downloading {
                            return h_flex()
                                .w_full()
                                .h(px(30.))
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .bg(theme.colors.secondary)
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .child("恢复中"),
                                )
                                .into_any_element();
                        }
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(theme.colors.secondary)
                            .child(
                                div()
                                    .flex_shrink_1()
                                    .text_size(px(11.))
                                    .text_color(theme.muted_foreground)
                                    .child(modified),
                            )
                            .child(
                                h_flex()
                                    .flex_shrink_0()
                                    .gap_1()
                                    .child(
                                        pointer_button(format!("sync-{}", name))
                                            .ghost()
                                            .xsmall()
                                            .child(div().text_size(px(10.)).child("恢复"))
                                            .loading(downloading)
                                            .disabled(any_busy && !downloading)
                                            .on_click(cx.listener(
                                                move |this, _: &ClickEvent, window, cx| {
                                                    this.confirm_download(
                                                        sync_name.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                    )
                                    .child(
                                        pointer_button(format!("delete-remote-{}", name))
                                            .ghost()
                                            .danger()
                                            .xsmall()
                                            .child(div().text_size(px(10.)).child("删除"))
                                            .loading(deleting)
                                            .disabled(any_busy && !deleting)
                                            .on_click(cx.listener(
                                                move |this, _: &ClickEvent, window, cx| {
                                                    this.confirm_delete(
                                                        delete_name.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                },
                                            )),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect();
                v_flex().w_full().gap_1().children(rows).into_any_element()
            }
        }
    }

    fn render_notifications(&self, enabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child("控制所有服务的桌面通知；倒计时与重置检测不受影响。"),
            )
            .child(h_flex().w_full().justify_center().child(render_checkbox(
                "settings-notifications",
                "启用桌面通知",
                enabled,
                cx.listener(move |this, checked: &bool, _window, cx| {
                    let enabled = *checked;
                    let state = this.state.clone();
                    let entity = cx.entity();
                    let _ = models::mutate(
                        &state,
                        cx,
                        move |app_state, _| {
                            app_state.global_notifications_enabled = enabled;
                            Ok(())
                        },
                        move |message, cx| {
                            let entity = entity.clone();
                            cx.defer(move |cx| {
                                entity.update(cx, |view, cx| view.show_toast(message, cx));
                            });
                        },
                    );
                }),
                Some(px(13.)),
                cx,
            )))
            .into_any_element()
    }

    fn render_backup_sync(&self, cx: &mut Context<Self>) -> AnyElement {
        let any_busy = self.busy.is_some();
        let listing = self.busy.as_ref() == Some(&Busy::List);
        let uploading = self.busy.as_ref() == Some(&Busy::Upload);
        let testing = self.busy.as_ref() == Some(&Busy::Test);

        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("服务器地址"))
                    .child(Input::new(&self.server_url).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("用户名"))
                    .child(Input::new(&self.username).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("密码"))
                    .child(Input::new(&self.password).w_full()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(field_label("远程目录"))
                    .child(Input::new(&self.remote_dir).w_full()),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_center()
                    .gap_2()
                    .child(
                        pointer_button("save-webdav")
                            .label("保存连接配置")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.save_config(window, cx);
                            })),
                    )
                    .child(
                        pointer_button("webdav-test")
                            .small()
                            .label("测试")
                            .loading(testing)
                            .disabled(any_busy && !testing)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.start_test_connection(window, cx);
                            })),
                    ),
            )
            .child(
                pointer_button("webdav-upload")
                    .primary()
                    .label("上传当前配置")
                    .w_full()
                    .loading(uploading)
                    .disabled(any_busy && !uploading)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.start_upload(window, cx);
                    })),
            )
            .child(
                pointer_button("webdav-list")
                    .label("查看云端列表")
                    .w_full()
                    .loading(listing)
                    .disabled(any_busy && !listing)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.start_list(window, cx);
                    })),
            )
            .child(self.render_remote_files(cx))
            .into_any_element()
    }

    fn render_import_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let any_busy = self.busy.is_some();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child("导入会覆盖当前配置；导出文件名包含时间戳。"),
            )
            .child(
                pointer_button("config-import")
                    .label("导入配置")
                    .w_full()
                    .disabled(any_busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.import_config(window, cx);
                    })),
            )
            .child(
                pointer_button("config-export")
                    .label("导出配置")
                    .w_full()
                    .disabled(any_busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.export_config(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_about(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .w_full()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.foreground)
                    .child("配额重置提醒"),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(format!("v{}", update::CURRENT_VERSION)),
            )
            .child(
                h_flex().pt_3().child(
                    pointer_button("check-update")
                        .label("检查更新")
                        .rounded(px(18.))
                        .loading(self.checking_update)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.check_update(window, cx);
                        })),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_center()
                    .child(
                        pointer_button("about-github")
                            .ghost()
                            .rounded(px(18.))
                            .icon(IconName::Github)
                            .label("GitHub")
                            .on_click(|_: &ClickEvent, _, cx| cx.open_url(update::REPO_URL)),
                    )
                    .child(
                        pointer_button("about-feedback")
                            .ghost()
                            .rounded(px(18.))
                            .icon(IconName::Github)
                            .label("问题反馈")
                            .on_click(|_: &ClickEvent, _, cx| cx.open_url(update::ISSUES_URL)),
                    ),
            )
            .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let notifications_enabled = self.state.read(cx).global_notifications_enabled;
        let active_tab = self.active_tab;

        let title_bar = h_flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(17.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.foreground)
                    .child("设置"),
            )
            .child(
                div()
                    .id("settings-close-x")
                    .text_size(px(14.))
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(|style| style.text_color(theme.foreground))
                    .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                        window.close_all_dialogs(cx);
                    }))
                    .child("✕"),
            );

        let nav = v_flex()
            .flex_shrink_0()
            .w(px(76.))
            .gap_1()
            .children(SettingsTab::ALL.map(|tab| {
                let selected = active_tab == tab;
                pointer_button(tab.id())
                    .ghost()
                    .selected(selected)
                    .w_full()
                    .px_1()
                    .cursor_pointer()
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .text_size(px(13.))
                            .child(tab.label()),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.active_tab != tab {
                            this.active_tab = tab;
                            cx.notify();
                        }
                    }))
            }));

        let content = match active_tab {
            SettingsTab::Notifications => self.render_notifications(notifications_enabled, cx),
            SettingsTab::BackupSync => self.render_backup_sync(cx),
            SettingsTab::ImportExport => self.render_import_export(cx),
            SettingsTab::About => self.render_about(cx),
        };
        let toast_state = self.toast.clone();

        v_flex()
            .id("settings-view")
            .relative()
            .w_full()
            .max_h(px(580.))
            .p_3()
            .gap_3()
            .child(title_bar)
            .child(
                h_flex()
                    .w_full()
                    .h(px(460.))
                    .items_start()
                    .gap_1()
                    .child(nav)
                    .child(div().w(px(1.)).h_full().flex_shrink_0().bg(theme.border))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .pr_3()
                            .gap_3()
                            .child(content)
                            .when_some(self.error.clone(), |el, error| {
                                el.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.danger)
                                        .child(error),
                                )
                            })
                            .overflow_y_scrollbar(),
                    ),
            )
            .when_some(toast_state, |el, state| {
                let action = if state.action_restart {
                    // 升级成功：立即重启（拉起新实例并退出）或稍后（关闭提示）
                    Some(
                        h_flex()
                            .gap_2()
                            .justify_center()
                            .child(
                                pointer_button("toast-restart")
                                    .primary()
                                    .label("立即重启")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.restart_app(window, cx);
                                    })),
                            )
                            .child(
                                pointer_button("toast-later")
                                    .ghost()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dismiss_restart_toast(cx);
                                    })),
                            )
                            .into_any_element(),
                    )
                } else if state.action_upgrade && !self.upgrading {
                    // 发现新版本：立即升级或取消（提示持久显示，不自动关闭）
                    Some(
                        h_flex()
                            .gap_2()
                            .justify_center()
                            .child(
                                pointer_button("toast-upgrade")
                                    .primary()
                                    .label("立即升级")
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.start_upgrade(window, cx);
                                    })),
                            )
                            .child(
                                pointer_button("toast-cancel")
                                    .ghost()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dismiss_restart_toast(cx);
                                    })),
                            )
                            .into_any_element(),
                    )
                } else if state.action_cancel && self.upgrading {
                    // 升级进行中：允许取消中断下载（网络卡住时不被永久阻塞）
                    Some(
                        h_flex()
                            .justify_center()
                            .child(
                                pointer_button("toast-upgrade-cancel")
                                    .ghost()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.cancel_upgrade(cx);
                                    })),
                            )
                            .into_any_element(),
                    )
                } else {
                    None
                };
                el.child(toast::toast_overlay(&state, action, false, cx))
            })
    }
}

/// 原子写入导出文件：先写同目录临时文件，再 rename；失败时清理临时文件。
fn write_json_atomic(path: &Path, json: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("创建临时文件失败: {}", tmp.display()))?;
        file.write_all(json.as_bytes())
            .context("写入临时文件失败")?;
        file.sync_all().ok();
        #[cfg(target_os = "windows")]
        if path.exists() {
            std::fs::remove_file(path).context("删除旧导出文件失败")?;
        }
        std::fs::rename(&tmp, path)
            .with_context(|| format!("写入导出文件失败: {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
