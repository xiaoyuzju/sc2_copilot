use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, ViewportBuilder, ViewportId};
use sc2_copilot_core::{EngineView, ScheduleCatalog};

use crate::{
    AlertCard, AppController, AppSettings, ConnectionState, ControllerUpdate, LocalSc2HttpClient,
    NoopAlertPlayer, Sc2PollingHandle, Sc2StateSource, SessionHistory, SettingsStore,
    platform::{PlatformAction, PlatformIntegration, configure_overlay_lock_window},
};

const CATALOG_JSON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/maps/catalog.json"
));
const SETTINGS_VIEWPORT_ID: &str = "sc2-copilot-settings";
const OVERLAY_LOCK_VIEWPORT_ID: &str = "sc2-copilot-overlay-lock";
const OVERLAY_LOCK_AREA_ID: &str = "sc2-copilot-overlay-lock-inline";
const OVERLAY_WINDOW_TITLE: &str = "SC2 Copilot 覆盖层";
const ALERT_LIFETIME: Duration = Duration::from_secs(8);
const OVERLAY_UPCOMING_LIMIT: usize = 3;
const OVERLAY_LOCK_WINDOW_SIZE: [f32; 2] = [58.0, 32.0];
const OVERLAY_LOCK_WINDOW_OFFSET: f32 = 8.0;
const APP_BACKGROUND: Color32 = Color32::from_rgb(8, 13, 23);
const NAVIGATION_BACKGROUND: Color32 = Color32::from_rgb(11, 18, 31);
const SURFACE: Color32 = Color32::from_rgb(17, 26, 42);
const SURFACE_RAISED: Color32 = Color32::from_rgb(23, 35, 55);
const BORDER: Color32 = Color32::from_rgb(42, 58, 82);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(235, 242, 252);
const TEXT_MUTED: Color32 = Color32::from_rgb(143, 160, 184);
const ACCENT: Color32 = Color32::from_rgb(77, 199, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgb(21, 65, 89);
const SUCCESS: Color32 = Color32::from_rgb(78, 211, 145);
const WARNING: Color32 = Color32::from_rgb(255, 181, 71);
const DANGER: Color32 = Color32::from_rgb(255, 105, 120);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SettingsPage {
    #[default]
    Overview,
    Match,
    Preferences,
    Diagnostics,
}

impl SettingsPage {
    const ALL: [Self; 4] = [
        Self::Overview,
        Self::Match,
        Self::Preferences,
        Self::Diagnostics,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Overview => "概览",
            Self::Match => "当前对局",
            Self::Preferences => "偏好设置",
            Self::Diagnostics => "诊断",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Overview => "连接、会话与提醒服务的实时状态",
            Self::Match => "地图上下文、分支与阶段锚点",
            Self::Preferences => "提醒时间、覆盖层与交互热键",
            Self::Diagnostics => "运行状态、数据版本与本地日志",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OverlayLockWindowState {
    #[default]
    Pending,
    Showing,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug)]
struct OverlayGeometryTransition {
    client_origin: egui::Pos2,
    interactive: bool,
}

pub fn run() -> Result<(), String> {
    let catalog = ScheduleCatalog::from_json(CATALOG_JSON).map_err(|error| error.to_string())?;
    let store = SettingsStore::for_current_user().map_err(|error| error.to_string())?;
    let (settings, mut startup_diagnostics) = match store.load() {
        Ok(settings) => (settings, Vec::new()),
        Err(error) => (
            AppSettings::default(),
            vec![format!("设置加载失败，已使用默认值：{error}")],
        ),
    };
    let poller = match LocalSc2HttpClient::new(Duration::from_millis(750)) {
        Ok(client) => Some(Sc2PollingHandle::spawn(
            Sc2StateSource::new(client),
            Duration::from_millis(250),
        )),
        Err(error) => {
            startup_diagnostics.push(error.to_string());
            None
        }
    };
    let overlay_position = settings.overlay_position;
    let overlay_size = settings.overlay_size;
    let controller = AppController::new(catalog, settings, Box::new(NoopAlertPlayer));
    let native_options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title(OVERLAY_WINDOW_TITLE)
            .with_app_id("sc2-copilot")
            .with_inner_size(overlay_size)
            .with_min_inner_size([300.0, 220.0])
            .with_position(overlay_position)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_active(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "SC2 Copilot",
        native_options,
        Box::new(move |creation_context| {
            if let Err(error) = configure_chinese_font(&creation_context.egui_ctx) {
                startup_diagnostics.push(error);
            }
            configure_ui_style(&creation_context.egui_ctx);
            Ok(Box::new(DesktopApp::new(
                controller,
                store,
                poller,
                startup_diagnostics,
                creation_context.egui_ctx.clone(),
            )))
        }),
    )
    .map_err(|error| error.to_string())
}

struct DesktopAlert {
    card: AlertCard,
    expires_at: Instant,
}

struct DesktopApp {
    controller: AppController,
    store: SettingsStore,
    poller: Option<Sc2PollingHandle>,
    platform: PlatformIntegration,
    alerts: Vec<DesktopAlert>,
    interactive_overlay: bool,
    applied_interactive_overlay: Option<bool>,
    overlay_geometry_transition: Option<OverlayGeometryTransition>,
    settings_visible: bool,
    quitting: bool,
    recording_hotkey: bool,
    settings_page: SettingsPage,
    overlay_lock_window_title: String,
    overlay_lock_window_state: OverlayLockWindowState,
    history: Option<SessionHistory>,
    history_status: String,
}

impl DesktopApp {
    fn new(
        mut controller: AppController,
        store: SettingsStore,
        poller: Option<Sc2PollingHandle>,
        startup_diagnostics: Vec<String>,
        egui_context: egui::Context,
    ) -> Self {
        let (platform, platform_diagnostics) =
            PlatformIntegration::new(controller.settings().hotkey.as_deref(), move || {
                egui_context.request_repaint();
            });
        for diagnostic in startup_diagnostics.into_iter().chain(platform_diagnostics) {
            controller.record_external_diagnostic(diagnostic);
        }
        let (history, history_status) = match SessionHistory::for_current_user() {
            Ok(mut history) => {
                let status = history.path().display().to_string();
                match history.record(&controller, &[]) {
                    Ok(_) => (Some(history), status),
                    Err(error) => {
                        controller.record_external_diagnostic(format!("历史日志写入失败：{error}"));
                        (None, format!("不可用：{error}"))
                    }
                }
            }
            Err(error) => {
                let status = format!("不可用：{error}");
                controller.record_external_diagnostic(format!("历史日志初始化失败：{error}"));
                (None, status)
            }
        };
        Self {
            controller,
            store,
            poller,
            platform,
            alerts: Vec::new(),
            interactive_overlay: false,
            applied_interactive_overlay: None,
            overlay_geometry_transition: None,
            settings_visible: true,
            quitting: false,
            recording_hotkey: false,
            settings_page: SettingsPage::default(),
            overlay_lock_window_title: format!("SC2 Copilot Overlay Lock {}", std::process::id()),
            overlay_lock_window_state: OverlayLockWindowState::default(),
            history,
            history_status,
        }
    }

    fn poll_sources(&mut self) {
        if let Some(poller) = &self.poller
            && let Some(poll) = poller.take_latest()
        {
            let update = self.controller.handle_poll(poll);
            self.apply_controller_update(update);
        }
        for action in self.platform.poll_actions() {
            match action {
                PlatformAction::ShowSettings => self.settings_visible = true,
                PlatformAction::ToggleInteraction => {
                    self.interactive_overlay = !self.interactive_overlay;
                }
                PlatformAction::Quit => self.quitting = true,
            }
        }
        self.alerts
            .retain(|alert| alert.expires_at > Instant::now());
    }

    fn push_alerts(&mut self, alerts: Vec<AlertCard>) {
        let expires_at = Instant::now() + ALERT_LIFETIME;
        self.alerts.extend(
            alerts
                .into_iter()
                .map(|card| DesktopAlert { card, expires_at }),
        );
    }

    fn apply_controller_update(&mut self, update: ControllerUpdate) {
        if let Some(history) = &mut self.history
            && let Err(error) = history.record(&self.controller, &update.new_alerts)
        {
            self.history_status = format!("不可用：{error}");
            self.controller
                .record_external_diagnostic(format!("历史日志写入失败：{error}"));
            self.history = None;
        }
        self.push_alerts(update.new_alerts);
    }

    fn stabilize_overlay_geometry(&mut self, ctx: &egui::Context) {
        let Some(transition) = self.overlay_geometry_transition else {
            return;
        };
        let geometry = ctx.input(|input| {
            let viewport = input.viewport();
            viewport.outer_rect.zip(viewport.inner_rect)
        });
        let Some((outer_rect, inner_rect)) = geometry else {
            return;
        };
        if viewport_has_native_frame(outer_rect, inner_rect) != transition.interactive {
            return;
        }
        if inner_rect.min.distance(transition.client_origin) <= 0.5 {
            self.overlay_geometry_transition = None;
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(
            corrected_overlay_outer_position(outer_rect, inner_rect, transition.client_origin),
        ));
    }

    fn save_settings(&mut self) {
        if let Err(error) = self.store.save(self.controller.settings()) {
            self.controller
                .record_external_diagnostic(format!("设置保存失败：{error}"));
        }
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("settings-navigation")
            .exact_size(208.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(NAVIGATION_BACKGROUND)
                    .inner_margin(18.0),
            )
            .show(ui, |ui| self.show_settings_navigation(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(APP_BACKGROUND).inner_margin(26.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.heading(
                            RichText::new(self.settings_page.label())
                                .size(27.0)
                                .color(TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.label(RichText::new(self.settings_page.description()).color(TEXT_MUTED));
                        ui.add_space(20.0);

                        match self.settings_page {
                            SettingsPage::Overview => self.show_overview_page(ui),
                            SettingsPage::Match => self.show_match_page(ui),
                            SettingsPage::Preferences => self.show_preferences_page(ui),
                            SettingsPage::Diagnostics => self.show_diagnostics_page(ui),
                        }
                    });
            });
    }

    fn show_settings_navigation(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("SC2").size(12.0).color(ACCENT).strong());
        ui.label(
            RichText::new("COPILOT")
                .size(23.0)
                .color(TEXT_PRIMARY)
                .strong(),
        );
        ui.label(RichText::new("合作任务实时辅助").color(TEXT_MUTED));
        ui.add_space(26.0);

        for page in SettingsPage::ALL {
            let selected = self.settings_page == page;
            let button = egui::Button::new(
                RichText::new(page.label())
                    .color(if selected { ACCENT } else { TEXT_PRIMARY })
                    .strong(),
            )
            .fill(if selected {
                ACCENT_SOFT
            } else {
                Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::new(
                1.0,
                if selected { ACCENT_SOFT } else { BORDER },
            ))
            .corner_radius(8.0);
            if ui.add_sized([ui.available_width(), 42.0], button).clicked() {
                self.settings_page = page;
                if page != SettingsPage::Preferences {
                    self.recording_hotkey = false;
                }
            }
            ui.add_space(4.0);
        }

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(14.0);
        ui.label(
            RichText::new("实时连接")
                .size(12.0)
                .color(TEXT_MUTED)
                .strong(),
        );
        ui.add_space(7.0);
        let connection = connection_presentation(self.controller.connection());
        status_badge(ui, connection.headline, connection.color);
        ui.add_space(8.0);
        ui.label(
            RichText::new("仅访问本机 127.0.0.1:6119")
                .size(12.0)
                .color(TEXT_MUTED),
        );
    }

    fn show_overview_page(&mut self, ui: &mut egui::Ui) {
        let connection = self.controller.connection();
        let presentation = connection_presentation(connection);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_label(ui, "SC2 连接");
                    ui.label(
                        RichText::new(presentation.headline)
                            .size(22.0)
                            .color(TEXT_PRIMARY)
                            .strong(),
                    );
                    ui.label(RichText::new(presentation.detail).color(TEXT_MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    status_badge(ui, presentation.badge, presentation.color);
                });
            });
        });

        ui.add_space(12.0);
        let session = self
            .controller
            .view()
            .session_id
            .as_deref()
            .unwrap_or("—")
            .to_owned();
        let map = self.selected_map_name().to_owned();
        let game_time = format_time(
            self.controller
                .view()
                .game_time_milliseconds
                .unwrap_or_default(),
        );
        ui.columns(3, |columns| {
            metric_card(&mut columns[0], "当前会话", &session, "离开对局后自动清除");
            metric_card(&mut columns[1], "任务地图", &map, "6119 自动识别或手动选择");
            metric_card(
                &mut columns[2],
                "游戏时间",
                &game_time,
                "由本地游戏状态提供",
            );
        });

        ui.add_space(12.0);
        ui.columns(2, |columns| {
            card(&mut columns[0], |ui| {
                section_label(ui, "覆盖层");
                ui.label(
                    RichText::new(if self.interactive_overlay {
                        "正在调整"
                    } else {
                        "鼠标穿透已启用"
                    })
                    .size(18.0)
                    .color(if self.interactive_overlay {
                        WARNING
                    } else {
                        SUCCESS
                    })
                    .strong(),
                );
                ui.label(
                    RichText::new(if self.interactive_overlay {
                        "拖动窗口标题栏和边框；按 Esc 完成。"
                    } else {
                        "不会拦截游戏鼠标和键盘焦点。"
                    })
                    .color(TEXT_MUTED),
                );
                ui.add_space(12.0);
                if primary_button(
                    ui,
                    if self.interactive_overlay {
                        "完成调整"
                    } else {
                        "调整覆盖层"
                    },
                )
                .clicked()
                {
                    self.interactive_overlay = !self.interactive_overlay;
                }
            });

            card(&mut columns[1], |ui| {
                section_label(ui, "提醒服务");
                service_row(ui, "播放接口", self.controller.player_status());
                service_row(ui, "托盘", self.platform.tray_status());
                service_row(ui, "全局热键", self.platform.hotkey_status());
            });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "安全边界");
            ui.label(
                RichText::new(
                    "只读本机游戏状态，不截屏、不读取游戏内存、不修改游戏文件，也不模拟输入。",
                )
                .color(TEXT_PRIMARY),
            );
            ui.label(
                RichText::new("关闭本窗口后程序继续驻留托盘；可从托盘菜单重新打开或退出。")
                    .color(TEXT_MUTED),
            );
        });
    }

    fn show_match_page(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            section_label(ui, "任务上下文");
            ui.label(
                RichText::new("用于补充 6119 无法提供的地图、分支和突变信息。").color(TEXT_MUTED),
            );
            ui.add_space(10.0);
            self.map_selector(ui);
            self.variant_selector(ui);
            self.mutator_selector(ui);
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "阶段锚点");
            ui.label(RichText::new("阶段标记只在当前对局有效，离局后不会保留。").color(TEXT_MUTED));
            ui.add_space(10.0);
            self.stage_controls(ui);
        });

        let unsupported = self
            .controller
            .selected_map_id()
            .and_then(|map_id| self.controller.map(map_id))
            .filter(|map| map.unsupported_row_count > 0)
            .map(|map| (map.unsupported_row_count, map.unsupported_reasons.clone()));
        if let Some((unsupported_row_count, reasons)) = unsupported {
            ui.add_space(12.0);
            card(ui, |ui| {
                section_label(ui, "暂不支持");
                ui.colored_label(
                    WARNING,
                    format!("{unsupported_row_count} 行数据当前不会触发提醒"),
                );
                ui.add_space(6.0);
                for reason in reasons {
                    ui.label(RichText::new(reason).color(TEXT_MUTED));
                }
            });
        }
    }

    fn show_preferences_page(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.controller.settings().clone();
        card(ui, |ui| {
            section_label(ui, "提醒提前量");
            ui.label(RichText::new("事件进入提前窗口时只提醒一次。").color(TEXT_MUTED));
            ui.add_space(10.0);
            ui.add(
                egui::Slider::new(&mut settings.lead_time_seconds, 0..=120)
                    .suffix(" 秒")
                    .trailing_fill(true),
            );
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "覆盖层交互热键");
            ui.horizontal(|ui| {
                keycap(ui, settings.hotkey.as_deref().unwrap_or("未配置"));
                if primary_button(
                    ui,
                    if self.recording_hotkey {
                        "请按组合键…"
                    } else {
                        "录制热键"
                    },
                )
                .clicked()
                {
                    self.recording_hotkey = true;
                }
                if ui
                    .add_enabled(settings.hotkey.is_some(), egui::Button::new("清除"))
                    .clicked()
                {
                    self.recording_hotkey = false;
                    match self.platform.configure_hotkey(None) {
                        Ok(()) => settings.hotkey = None,
                        Err(error) => self.controller.record_external_diagnostic(error),
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(if self.recording_hotkey {
                    "请直接按下新的组合键；按 Esc 取消。"
                } else {
                    "在游戏中临时打开覆盖层交互模式；不会向游戏转发输入。"
                })
                .color(if self.recording_hotkey {
                    ACCENT
                } else {
                    TEXT_MUTED
                }),
            );
            if self.recording_hotkey {
                let capture = ui.input(|input| input.events.iter().find_map(capture_hotkey));
                match capture {
                    Some(HotkeyCapture::Captured(value)) => {
                        match self.platform.configure_hotkey(Some(&value)) {
                            Ok(()) => {
                                self.recording_hotkey = false;
                                settings.hotkey = Some(value);
                            }
                            Err(error) => self.controller.record_external_diagnostic(error),
                        }
                    }
                    Some(HotkeyCapture::Cancelled) => self.recording_hotkey = false,
                    Some(HotkeyCapture::Unsupported(value)) => self
                        .controller
                        .record_external_diagnostic(format!("不支持该全局热键：{value}")),
                    None => {}
                }
            }
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "覆盖层布局");
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(if self.interactive_overlay {
                        "预览已显示，可拖动和缩放"
                    } else {
                        "当前为鼠标穿透模式"
                    });
                    ui.label(RichText::new("无需进入合作任务即可预览并调整。").color(TEXT_MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(
                        ui,
                        if self.interactive_overlay {
                            "完成调整"
                        } else {
                            "开始调整"
                        },
                    )
                    .clicked()
                    {
                        self.interactive_overlay = !self.interactive_overlay;
                    }
                });
            });
        });

        if settings != *self.controller.settings() {
            let update = self.controller.update_settings(settings);
            self.apply_controller_update(update);
            self.save_settings();
        }
    }

    fn show_diagnostics_page(&mut self, ui: &mut egui::Ui) {
        let connection = connection_presentation(self.controller.connection());
        let variant_name = self.selected_variant_name().to_owned();
        let unsupported = self
            .controller
            .selected_map_id()
            .and_then(|map_id| self.controller.map(map_id))
            .map(|map| (map.unsupported_row_count, map.unsupported_reasons.clone()));
        card(ui, |ui| {
            section_label(ui, "运行状态");
            egui::Grid::new("diagnostic-status-grid")
                .num_columns(2)
                .spacing([24.0, 9.0])
                .show(ui, |ui| {
                    diagnostic_row(ui, "6119 状态", connection.badge);
                    diagnostic_row(
                        ui,
                        "会话",
                        self.controller.view().session_id.as_deref().unwrap_or("—"),
                    );
                    diagnostic_row(ui, "地图", self.selected_map_name());
                    diagnostic_row(ui, "地图分支", &variant_name);
                    diagnostic_row(ui, "数据快照", self.controller.snapshot_batch());
                    diagnostic_row(ui, "播放接口", self.controller.player_status());
                    diagnostic_row(ui, "托盘", self.platform.tray_status());
                    diagnostic_row(ui, "全局热键", self.platform.hotkey_status());
                });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            match unsupported {
                None => service_row(ui, "暂不支持", "未选择地图"),
                Some((0, _)) => service_row(ui, "暂不支持", "当前地图无暂不支持条目"),
                Some((count, reasons)) => {
                    ui.colored_label(WARNING, format!("暂不支持 · {count} 行"));
                    for reason in reasons {
                        ui.label(RichText::new(reason).color(TEXT_MUTED));
                    }
                }
            }
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "诊断消息");
            egui::ScrollArea::vertical()
                .max_height(250.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.controller.diagnostics().is_empty() {
                        ui.label(RichText::new("暂无诊断消息").color(TEXT_MUTED));
                    } else {
                        for line in self.controller.diagnostics() {
                            ui.label(line);
                        }
                    }
                });
        });

        ui.add_space(12.0);
        card(ui, |ui| {
            section_label(ui, "本地文件");
            service_row(ui, "设置文件", &self.store.path().display().to_string());
            service_row(ui, "历史日志", &self.history_status);
        });
    }

    fn map_selector(&mut self, ui: &mut egui::Ui) {
        if self.controller.connection() != ConnectionState::InGame {
            ui.label("进入合作任务后可选择地图上下文。");
            return;
        }
        if self.controller.auto_map_id().is_some() {
            ui.label(format!(
                "地图已由 6119 自动识别：{}",
                self.selected_map_name()
            ));
            return;
        }
        let current = self.controller.manual_map_id().map(str::to_owned);
        let mut requested = current.clone();
        egui::ComboBox::from_label("地图手动兜底")
            .selected_text(
                current
                    .as_deref()
                    .and_then(|id| self.controller.map(id))
                    .map(|map| map.display_name.as_str())
                    .unwrap_or("请选择地图"),
            )
            .show_ui(ui, |ui| {
                for map in self.controller.maps() {
                    ui.selectable_value(&mut requested, Some(map.id.clone()), &map.display_name);
                }
            });
        if requested != current {
            let update = self.controller.select_manual_map(requested);
            self.apply_controller_update(update);
        }
    }

    fn variant_selector(&mut self, ui: &mut egui::Ui) {
        let Some(map_id) = self.controller.selected_map_id().map(str::to_owned) else {
            return;
        };
        let Some(map) = self.controller.map(&map_id) else {
            return;
        };
        if map.variants.is_empty() {
            return;
        }
        let variants = map.variants.clone();
        let current = self.controller.view().variant_id.clone();
        let mut requested = current.clone();
        let selected_name = current
            .as_deref()
            .and_then(|id| variants.iter().find(|variant| variant.id == id))
            .map(|variant| variant.display_name.as_str())
            .unwrap_or("请选择分支");
        egui::ComboBox::from_label("地图分支 / 条件")
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut requested, None, "不选择");
                for variant in variants {
                    ui.selectable_value(&mut requested, Some(variant.id), variant.display_name);
                }
            });
        if requested != current {
            let update = self.controller.select_variant(requested);
            self.apply_controller_update(update);
        }
    }

    fn stage_controls(&mut self, ui: &mut egui::Ui) {
        let Some(map_id) = self.controller.selected_map_id().map(str::to_owned) else {
            return;
        };
        let Some(map) = self.controller.map(&map_id) else {
            return;
        };
        let stage_ids = map.stage_ids.clone();
        if stage_ids.is_empty() {
            return;
        }
        ui.label("阶段锚点（仅当前对局）");
        for stage_id in stage_ids {
            let anchored = self
                .controller
                .view()
                .stage_anchors
                .iter()
                .any(|anchor| anchor.stage_id == stage_id);
            ui.horizontal(|ui| {
                ui.label(&stage_id);
                if ui
                    .button(if anchored {
                        "重新标记"
                    } else {
                        "标记开始"
                    })
                    .clicked()
                {
                    let update = self.controller.set_stage_anchor(stage_id.clone());
                    self.apply_controller_update(update);
                }
                if anchored && ui.button("清除").clicked() {
                    let update = self.controller.clear_stage_anchor(stage_id.clone());
                    self.apply_controller_update(update);
                }
            });
        }
    }

    fn mutator_selector(&mut self, ui: &mut egui::Ui) {
        let Some(map_id) = self.controller.selected_map_id().map(str::to_owned) else {
            return;
        };
        let Some(map) = self.controller.map(&map_id) else {
            return;
        };
        let mutators = map.mutators.clone();
        if mutators.is_empty() {
            return;
        }
        ui.label("当前突变因子（手动）");
        for mutator in mutators {
            let mut active = self
                .controller
                .view()
                .active_mutator_ids
                .iter()
                .any(|id| id == &mutator.id);
            if ui.checkbox(&mut active, &mutator.display_name).changed() {
                let update = self.controller.set_mutator_active(mutator.id, active);
                self.apply_controller_update(update);
            }
        }
    }

    fn selected_map_name(&self) -> &str {
        self.controller
            .selected_map_id()
            .and_then(|id| self.controller.map(id))
            .map(|map| map.display_name.as_str())
            .unwrap_or("—")
    }

    fn selected_variant_name(&self) -> &str {
        let Some(map_id) = self.controller.selected_map_id() else {
            return "—";
        };
        let Some(map) = self.controller.map(map_id) else {
            return "—";
        };
        if map.variants.is_empty() {
            return "不适用";
        }
        let Some(variant_id) = self.controller.view().variant_id.as_deref() else {
            return "未选择";
        };
        map.variants
            .iter()
            .find(|variant| variant.id == variant_id)
            .map(|variant| variant.display_name.as_str())
            .unwrap_or("未选择")
    }

    fn show_overlay(&mut self, ui: &mut egui::Ui) {
        let has_session = self.controller.view().session_id.is_some();
        if !overlay_should_render(has_session, self.interactive_overlay) {
            return;
        }
        if has_session {
            let view = self.controller.view().clone();
            let map_name = self.selected_map_name().to_owned();
            let alerts = self
                .alerts
                .iter()
                .map(|alert| alert.card.clone())
                .collect::<Vec<_>>();
            let upcoming = view
                .upcoming_events
                .iter()
                .take(OVERLAY_UPCOMING_LIMIT)
                .map(|event| {
                    (
                        event.remaining_milliseconds,
                        self.controller.event_label(&event.event_id),
                    )
                })
                .collect::<Vec<_>>();
            overlay_surface(ui, |ui| {
                overlay_contents(
                    ui,
                    &map_name,
                    &view,
                    &alerts,
                    &upcoming,
                    self.interactive_overlay,
                );
                if self.interactive_overlay {
                    ui.separator();
                    self.variant_selector(ui);
                    self.mutator_selector(ui);
                    self.stage_controls(ui);
                }
            });
        } else {
            overlay_surface(ui, |ui| {
                ui.add_space(OVERLAY_LOCK_WINDOW_SIZE[1]);
                section_label(ui, "OVERLAY PREVIEW");
                ui.heading(RichText::new("覆盖层布局预览").color(TEXT_PRIMARY).strong());
                status_badge(ui, "正在调整位置和大小", WARNING);
                ui.separator();
                ui.label(RichText::new("拖动窗口标题栏改变位置").color(TEXT_PRIMARY));
                ui.label(RichText::new("拖动窗口边框改变大小").color(TEXT_PRIMARY));
                ui.label(RichText::new("按 Esc 结束并恢复鼠标穿透").color(TEXT_MUTED));
            });
        }
        if self.interactive_overlay && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.interactive_overlay = false;
        }
        if self.interactive_overlay && self.overlay_geometry_transition.is_none() {
            let (position, size) = ui.input(|input| {
                let viewport = input.viewport();
                (
                    viewport.inner_rect.map(|rect| rect.min),
                    viewport.inner_rect.map(|rect| rect.size()),
                )
            });
            let mut settings = self.controller.settings().clone();
            let mut changed = false;
            if let Some(position) = position {
                let new_position = [position.x, position.y];
                if new_position != settings.overlay_position {
                    settings.overlay_position = new_position;
                    changed = true;
                }
            }
            if let Some(size) = size {
                let new_size = [size.x, size.y];
                if new_size != settings.overlay_size {
                    settings.overlay_size = new_size;
                    changed = true;
                }
            }
            if changed {
                self.controller.update_settings(settings);
                self.save_settings();
            }
        }
        if overlay_lock_control_should_render(
            self.interactive_overlay,
            self.overlay_geometry_transition.is_none(),
        ) {
            let lock_requested = egui::Area::new(egui::Id::new(OVERLAY_LOCK_AREA_ID))
                .fixed_pos(egui::pos2(
                    OVERLAY_LOCK_WINDOW_OFFSET,
                    OVERLAY_LOCK_WINDOW_OFFSET,
                ))
                .movable(false)
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| overlay_lock_button(ui, true))
                .inner;
            if lock_requested {
                self.interactive_overlay = false;
            }
        }
    }

    fn show_overlay_lock_button(&mut self, ctx: &egui::Context) {
        if self.interactive_overlay {
            self.overlay_lock_window_state = OverlayLockWindowState::Pending;
            return;
        }
        if self.overlay_lock_window_state == OverlayLockWindowState::Failed {
            return;
        }

        let [overlay_x, overlay_y] = self.controller.settings().overlay_position;
        let button_position = [
            overlay_x + OVERLAY_LOCK_WINDOW_OFFSET,
            overlay_y + OVERLAY_LOCK_WINDOW_OFFSET,
        ];
        let visible = matches!(
            self.overlay_lock_window_state,
            OverlayLockWindowState::Showing | OverlayLockWindowState::Ready
        );
        let title = self.overlay_lock_window_title.clone();
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(OVERLAY_LOCK_VIEWPORT_ID),
            ViewportBuilder::default()
                .with_title(title)
                .with_inner_size(OVERLAY_LOCK_WINDOW_SIZE)
                .with_min_inner_size(OVERLAY_LOCK_WINDOW_SIZE)
                .with_position(button_position)
                .with_decorations(false)
                .with_resizable(false)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_always_on_top()
                .with_mouse_passthrough(false)
                .with_taskbar(false)
                .with_active(false)
                .with_visible(visible),
            |ui, _class| {
                if overlay_lock_button(ui, false) {
                    self.interactive_overlay = true;
                }
            },
        );

        if matches!(
            self.overlay_lock_window_state,
            OverlayLockWindowState::Pending | OverlayLockWindowState::Showing
        ) {
            match configure_overlay_lock_window(
                &self.overlay_lock_window_title,
                OVERLAY_WINDOW_TITLE,
            ) {
                Ok(true) => {
                    self.overlay_lock_window_state =
                        if self.overlay_lock_window_state == OverlayLockWindowState::Pending {
                            OverlayLockWindowState::Showing
                        } else {
                            OverlayLockWindowState::Ready
                        };
                }
                Ok(false) => {}
                Err(error) => {
                    self.controller.record_external_diagnostic(error);
                    self.overlay_lock_window_state = OverlayLockWindowState::Failed;
                }
            }
        }
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_visible {
            return;
        }
        ctx.show_viewport_immediate(
            ViewportId::from_hash_of(SETTINGS_VIEWPORT_ID),
            ViewportBuilder::default()
                .with_title("SC2 Copilot")
                .with_inner_size([980.0, 720.0])
                .with_min_inner_size([820.0, 620.0])
                .with_taskbar(true)
                .with_active(true),
            |ui, _class| {
                if ui.input(|input| input.viewport().close_requested()) {
                    self.settings_visible = false;
                    return;
                }
                self.show_settings(ui);
            },
        );
    }
}

fn overlay_surface<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let frame = egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(8, 13, 23, 238))
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(14.0)
        .inner_margin(16.0);
    egui::CentralPanel::default()
        .frame(frame)
        .show(ui, add_contents)
}

fn overlay_lock_button(ui: &mut egui::Ui, interactive: bool) -> bool {
    let button_text = if interactive { "锁定" } else { "解锁" };
    let button_fill = if interactive {
        Color32::from_rgb(92, 57, 22)
    } else {
        ACCENT_SOFT
    };
    let button = egui::Button::new(
        RichText::new(button_text)
            .size(12.0)
            .color(TEXT_PRIMARY)
            .strong(),
    )
    .fill(button_fill)
    .stroke(egui::Stroke::new(1.0, BORDER))
    .corner_radius(8.0);
    ui.add_sized(OVERLAY_LOCK_WINDOW_SIZE, button).clicked()
}

fn card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(12.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(TEXT_MUTED).strong());
    ui.add_space(4.0);
}

fn metric_card(ui: &mut egui::Ui, label: &str, value: &str, detail: &str) {
    card(ui, |ui| {
        section_label(ui, label);
        ui.label(RichText::new(value).size(19.0).color(TEXT_PRIMARY).strong());
        ui.label(RichText::new(detail).size(12.0).color(TEXT_MUTED));
    });
}

fn status_badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            32,
        ))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.75)))
        .corner_radius(999.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color).strong());
        });
}

fn keycap(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(APP_BACKGROUND)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(7.0)
        .inner_margin(egui::Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.monospace(RichText::new(text).color(TEXT_PRIMARY));
        });
}

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(APP_BACKGROUND).strong())
            .fill(ACCENT)
            .stroke(egui::Stroke::NONE)
            .corner_radius(7.0),
    )
}

fn service_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(TEXT_MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).color(TEXT_PRIMARY));
        });
    });
}

fn diagnostic_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).color(TEXT_MUTED));
    ui.label(RichText::new(value).color(TEXT_PRIMARY));
    ui.end_row();
}

#[derive(Clone, Copy)]
struct ConnectionPresentation {
    headline: &'static str,
    detail: &'static str,
    badge: &'static str,
    color: Color32,
}

fn connection_presentation(connection: ConnectionState) -> ConnectionPresentation {
    match connection {
        ConnectionState::Disconnected => ConnectionPresentation {
            headline: "等待游戏",
            detail: "SC2 未启动，或本机 6119 状态暂不可用。",
            badge: "未连接 / 游戏未启动",
            color: TEXT_MUTED,
        },
        ConnectionState::Menu => ConnectionPresentation {
            headline: "SC2 已连接",
            detail: "当前位于菜单；进入合作任务后将自动建立会话。",
            badge: "已连接，位于菜单",
            color: ACCENT,
        },
        ConnectionState::InGame => ConnectionPresentation {
            headline: "对局进行中",
            detail: "已建立游戏会话；提醒状态取决于当前地图与运行支持条件。",
            badge: "对局中",
            color: SUCCESS,
        },
    }
}

impl eframe::App for DesktopApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_sources();
        if self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.interactive_overlay = false;
        }
        if self.applied_interactive_overlay != Some(self.interactive_overlay) {
            let client_origin = if self.applied_interactive_overlay.is_none() {
                let [x, y] = self.controller.settings().overlay_position;
                Some(egui::pos2(x, y))
            } else {
                ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min))
            };
            self.overlay_geometry_transition =
                client_origin.map(|client_origin| OverlayGeometryTransition {
                    client_origin,
                    interactive: self.interactive_overlay,
                });
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
                !self.interactive_overlay,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(self.interactive_overlay));
            ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(self.interactive_overlay));
            let [width, height] = self.controller.settings().overlay_size;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(width, height)));
            self.applied_interactive_overlay = Some(self.interactive_overlay);
        }
        self.stabilize_overlay_geometry(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show_overlay(ui);
        self.show_overlay_lock_button(ui.ctx());
        self.show_settings_window(ui.ctx());
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn overlay_contents(
    ui: &mut egui::Ui,
    map_name: &str,
    view: &EngineView,
    alerts: &[AlertCard],
    upcoming: &[(u64, String)],
    interactive: bool,
) {
    ui.horizontal(|ui| {
        ui.add_space(OVERLAY_LOCK_WINDOW_SIZE[0]);
        ui.heading(
            RichText::new(map_name)
                .size(21.0)
                .color(TEXT_PRIMARY)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format_time(view.game_time_milliseconds.unwrap_or_default()))
                    .size(19.0)
                    .color(TEXT_MUTED)
                    .monospace(),
            );
        });
    });
    if interactive {
        ui.colored_label(ACCENT, "交互模式 · 按 Esc 退出");
    }
    if !alerts.is_empty() {
        ui.add_space(10.0);
        for alert in alerts {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(76, 45, 13, 238))
                .stroke(egui::Stroke::new(1.0, WARNING.gamma_multiply(0.8)))
                .corner_radius(9.0)
                .inner_margin(11.0)
                .show(ui, |ui| {
                    ui.colored_label(WARNING, RichText::new("即将发生").strong());
                    for label in &alert.event_labels {
                        ui.label(RichText::new(label).size(16.0).color(TEXT_PRIMARY).strong());
                    }
                });
            ui.add_space(6.0);
        }
    }
    ui.add_space(12.0);
    if upcoming.is_empty() {
        ui.label(RichText::new("当前上下文没有可运行的后续事件。").color(TEXT_MUTED));
    } else {
        section_label(ui, "下一个事件");
        let (remaining_milliseconds, label) = &upcoming[0];
        overlay_event_row(ui, *remaining_milliseconds, label, true);

        for (remaining_milliseconds, label) in &upcoming[1..] {
            overlay_event_row(ui, *remaining_milliseconds, label, false);
        }
    }
}

fn overlay_event_row(
    ui: &mut egui::Ui,
    remaining_milliseconds: u64,
    label: &str,
    primary: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        if primary {
            ui.monospace(
                RichText::new(format_remaining(remaining_milliseconds))
                    .size(22.0)
                    .color(ACCENT)
                    .strong(),
            );
            ui.add(
                egui::Label::new(RichText::new(label).size(16.0).color(TEXT_PRIMARY).strong())
                    .wrap(),
            )
        } else {
            ui.monospace(RichText::new(format_remaining(remaining_milliseconds)).color(TEXT_MUTED));
            ui.add(egui::Label::new(RichText::new(label).color(TEXT_MUTED)).wrap())
        }
    })
    .inner
}

fn overlay_should_render(has_session: bool, interactive: bool) -> bool {
    has_session || interactive
}

fn overlay_lock_control_should_render(interactive: bool, geometry_stable: bool) -> bool {
    interactive && geometry_stable
}

fn viewport_has_native_frame(outer_rect: egui::Rect, inner_rect: egui::Rect) -> bool {
    outer_rect.width() - inner_rect.width() > 1.0 || outer_rect.height() - inner_rect.height() > 1.0
}

fn corrected_overlay_outer_position(
    outer_rect: egui::Rect,
    inner_rect: egui::Rect,
    client_origin: egui::Pos2,
) -> egui::Pos2 {
    outer_rect.min + (client_origin - inner_rect.min)
}

#[derive(Debug, PartialEq, Eq)]
enum HotkeyCapture {
    Captured(String),
    Cancelled,
    Unsupported(String),
}

fn capture_hotkey(event: &egui::Event) -> Option<HotkeyCapture> {
    let egui::Event::Key {
        key,
        physical_key,
        pressed: true,
        repeat: false,
        modifiers,
    } = event
    else {
        return None;
    };
    if *key == egui::Key::Escape {
        return Some(HotkeyCapture::Cancelled);
    }
    let key = hotkey_key_name(physical_key.unwrap_or(*key))?;
    let mut parts = Vec::with_capacity(4);
    if modifiers.ctrl {
        parts.push("Control".to_owned());
    }
    if modifiers.shift {
        parts.push("Shift".to_owned());
    }
    if modifiers.alt {
        parts.push("Alt".to_owned());
    }
    if modifiers.mac_cmd {
        parts.push("Super".to_owned());
    }
    parts.push(key);
    let value = parts.join("+");
    Some(if hotkey_is_supported(&value) {
        HotkeyCapture::Captured(value)
    } else {
        HotkeyCapture::Unsupported(value)
    })
}

#[cfg(windows)]
fn hotkey_is_supported(value: &str) -> bool {
    value.parse::<global_hotkey::hotkey::HotKey>().is_ok()
}

#[cfg(not(windows))]
fn hotkey_is_supported(_value: &str) -> bool {
    true
}

fn hotkey_key_name(key: egui::Key) -> Option<String> {
    let name = key.name();
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
        return Some(format!("Key{name}"));
    }
    if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() {
        return Some(format!("Digit{name}"));
    }
    Some(
        match key {
            egui::Key::ShiftLeft
            | egui::Key::ShiftRight
            | egui::Key::ControlLeft
            | egui::Key::ControlRight
            | egui::Key::AltLeft
            | egui::Key::AltRight
            | egui::Key::SuperLeft
            | egui::Key::SuperRight => return None,
            egui::Key::Equals => "Equal",
            egui::Key::OpenBracket => "BracketLeft",
            egui::Key::CloseBracket => "BracketRight",
            egui::Key::Backtick => "Backquote",
            _ => name,
        }
        .to_owned(),
    )
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn format_remaining(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1_000);
    format!("-{:02}:{:02}", seconds / 60, seconds % 60)
}

fn configure_ui_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 9.0);
    style.spacing.button_padding = egui::vec2(13.0, 8.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.slider_width = 260.0;
    style.spacing.combo_width = 240.0;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.weak_text_color = Some(TEXT_MUTED);
    visuals.panel_fill = APP_BACKGROUND;
    visuals.window_fill = APP_BACKGROUND;
    visuals.window_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.faint_bg_color = SURFACE;
    visuals.extreme_bg_color = APP_BACKGROUND;
    visuals.code_bg_color = APP_BACKGROUND;
    visuals.hyperlink_color = ACCENT;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = DANGER;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.bg_fill = SURFACE_RAISED;
    visuals.widgets.inactive.weak_bg_fill = SURFACE_RAISED;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
    visuals.widgets.hovered.weak_bg_fill = ACCENT_SOFT;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.active.bg_fill = ACCENT_SOFT;
    visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.slider_trailing_fill = true;
    style.visuals = visuals;
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn configure_chinese_font(ctx: &egui::Context) -> Result<(), String> {
    let windows_directory = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    for candidate in ["msyh.ttc", "simhei.ttf"] {
        let path = windows_directory.join("Fonts").join(candidate);
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "sc2-copilot-cjk".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "sc2-copilot-cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return Ok(());
    }
    Err("未找到 Windows 中文字体，界面中文可能无法正确显示".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        HotkeyCapture, capture_hotkey, corrected_overlay_outer_position, egui, overlay_event_row,
        overlay_lock_control_should_render, overlay_should_render, overlay_surface,
        viewport_has_native_frame,
    };

    #[test]
    fn overlay_surface_fills_the_available_client_area() {
        egui::__run_test_ui(|ui| {
            let available = ui.available_rect_before_wrap();
            let response = overlay_surface(ui, |_| {});
            assert_eq!(response.response.rect, available);
        });
    }

    #[test]
    fn overlay_outer_position_compensates_for_the_native_frame() {
        let outer = egui::Rect::from_min_size(egui::pos2(220.0, 220.0), egui::vec2(458.0, 350.0));
        let inner = egui::Rect::from_min_size(egui::pos2(227.0, 249.0), egui::vec2(443.0, 313.0));
        let client_origin = egui::pos2(220.0, 220.0);

        assert!(viewport_has_native_frame(outer, inner));
        assert_eq!(
            corrected_overlay_outer_position(outer, inner, client_origin),
            egui::pos2(213.0, 191.0)
        );
    }

    #[test]
    fn overlay_event_text_stays_within_the_panel_width() {
        let context = egui::Context::default();
        let _ = context.run_ui(Default::default(), |ui| {
            for primary in [true, false] {
                ui.allocate_ui(egui::vec2(180.0, 200.0), |ui| {
                    let right_edge = ui.max_rect().right();
                    let label = overlay_event_row(
                        ui,
                        90_000,
                        "A very long event description that must wrap inside the overlay panel",
                        primary,
                    );

                    assert!(
                        label.rect.right() <= right_edge,
                        "event label right edge {} exceeded panel right edge {right_edge}",
                        label.rect.right()
                    );
                });
            }
        });
    }

    #[test]
    fn overlay_adjustment_renders_a_preview_without_an_active_session() {
        assert!(overlay_should_render(false, true));
        assert!(overlay_should_render(true, false));
        assert!(!overlay_should_render(false, false));
    }

    #[test]
    fn overlay_lock_control_waits_for_stable_geometry() {
        assert!(!overlay_lock_control_should_render(true, false));
        assert!(overlay_lock_control_should_render(true, true));
        assert!(!overlay_lock_control_should_render(false, true));
    }

    #[test]
    fn hotkey_capture_uses_pressed_modifiers_and_escape_cancels() {
        let modifiers = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let pressed = egui::Event::Key {
            key: egui::Key::O,
            physical_key: Some(egui::Key::O),
            pressed: true,
            repeat: false,
            modifiers,
        };
        assert_eq!(
            capture_hotkey(&pressed),
            Some(HotkeyCapture::Captured("Control+Shift+KeyO".to_owned()))
        );

        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        };
        assert_eq!(capture_hotkey(&escape), Some(HotkeyCapture::Cancelled));

        let unsupported = egui::Event::Key {
            key: egui::Key::F25,
            physical_key: Some(egui::Key::F25),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        assert_eq!(
            capture_hotkey(&unsupported),
            Some(HotkeyCapture::Unsupported("Control+F25".to_owned()))
        );
    }
}
