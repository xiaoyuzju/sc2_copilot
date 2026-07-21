use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, ViewportBuilder, ViewportId};
use sc2_copilot_core::{EngineView, EventTiming, ScheduleCatalog};

use crate::{
    AlertCard, AppController, AppSettings, ConnectionState, LocalSc2HttpClient, NoopAlertPlayer,
    Sc2PollingHandle, Sc2StateSource, SettingsStore,
    platform::{PlatformAction, PlatformIntegration},
};

const CATALOG_JSON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/maps/catalog.json"
));
const SETTINGS_VIEWPORT_ID: &str = "sc2-copilot-settings";
const ALERT_LIFETIME: Duration = Duration::from_secs(8);

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
    let controller = AppController::new(catalog, settings, Box::new(NoopAlertPlayer));
    let native_options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("SC2 Copilot 覆盖层")
            .with_app_id("sc2-copilot")
            .with_inner_size([440.0, 430.0])
            .with_position(overlay_position)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_active(false),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "SC2 Copilot",
        native_options,
        Box::new(move |creation_context| {
            if let Err(error) = configure_chinese_font(&creation_context.egui_ctx) {
                startup_diagnostics.push(error);
            }
            Ok(Box::new(DesktopApp::new(
                controller,
                store,
                poller,
                startup_diagnostics,
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
    settings_visible: bool,
    quitting: bool,
    hotkey_editor: String,
}

impl DesktopApp {
    fn new(
        mut controller: AppController,
        store: SettingsStore,
        poller: Option<Sc2PollingHandle>,
        startup_diagnostics: Vec<String>,
    ) -> Self {
        let hotkey_editor = controller.settings().hotkey.clone().unwrap_or_default();
        let (platform, platform_diagnostics) =
            PlatformIntegration::new(controller.settings().hotkey.as_deref());
        for diagnostic in startup_diagnostics.into_iter().chain(platform_diagnostics) {
            controller.record_external_diagnostic(diagnostic);
        }
        Self {
            controller,
            store,
            poller,
            platform,
            alerts: Vec::new(),
            interactive_overlay: false,
            applied_interactive_overlay: None,
            settings_visible: true,
            quitting: false,
            hotkey_editor,
        }
    }

    fn poll_sources(&mut self) {
        if let Some(poller) = &self.poller
            && let Some(poll) = poller.take_latest()
        {
            let update = self.controller.handle_poll(poll);
            self.push_alerts(update.new_alerts);
        }
        for action in self.platform.poll_actions() {
            match action {
                PlatformAction::ShowSettings => self.settings_visible = true,
                PlatformAction::ToggleInteraction => {
                    if overlay_interaction_available(
                        self.controller.connection(),
                        self.controller.view().session_id.is_some(),
                    ) {
                        self.interactive_overlay = !self.interactive_overlay;
                    }
                }
                PlatformAction::Quit => self.quitting = true,
            }
        }
        if !overlay_interaction_available(
            self.controller.connection(),
            self.controller.view().session_id.is_some(),
        ) {
            self.interactive_overlay = false;
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

    fn save_settings(&mut self) {
        if let Err(error) = self.store.save(self.controller.settings()) {
            self.controller
                .record_external_diagnostic(format!("设置保存失败：{error}"));
        }
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("SC2 Copilot");
            ui.label("只读本机 6119 状态；不截屏、不读取游戏内存、不模拟输入。");
            ui.separator();

            egui::Grid::new("status-grid")
                .num_columns(2)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    ui.label("6119 状态");
                    ui.label(connection_text(self.controller.connection()));
                    ui.end_row();
                    ui.label("会话");
                    ui.label(self.controller.view().session_id.as_deref().unwrap_or("—"));
                    ui.end_row();
                    ui.label("地图");
                    ui.label(self.selected_map_name());
                    ui.end_row();
                    ui.label("游戏时间");
                    ui.label(format_time(
                        self.controller
                            .view()
                            .game_time_milliseconds
                            .unwrap_or_default(),
                    ));
                    ui.end_row();
                    ui.label("数据快照");
                    ui.label(self.controller.snapshot_batch());
                    ui.end_row();
                    ui.label("播放接口");
                    ui.label(self.controller.player_status());
                    ui.end_row();
                    ui.label("托盘");
                    ui.label(self.platform.tray_status());
                    ui.end_row();
                    ui.label("全局热键");
                    ui.label(self.platform.hotkey_status());
                    ui.end_row();
                });

            ui.separator();
            ui.heading("当前对局");
            self.map_selector(ui);
            self.variant_selector(ui);
            self.mutator_selector(ui);
            if let Some(map_id) = self.controller.selected_map_id()
                && let Some(map) = self.controller.map(map_id)
                && map.unsupported_row_count > 0
            {
                ui.collapsing(
                    format!("暂不支持的数据行：{}", map.unsupported_row_count),
                    |ui| {
                        for reason in &map.unsupported_reasons {
                            ui.label(reason);
                        }
                    },
                );
            }
            let interaction_available = overlay_interaction_available(
                self.controller.connection(),
                self.controller.view().session_id.is_some(),
            );
            if ui
                .add_enabled(
                    interaction_available,
                    egui::Button::new(if self.interactive_overlay {
                        "恢复覆盖层鼠标穿透"
                    } else {
                        "临时启用覆盖层交互"
                    }),
                )
                .clicked()
            {
                self.interactive_overlay = !self.interactive_overlay;
            }

            ui.separator();
            ui.heading("设置");
            let mut settings = self.controller.settings().clone();
            ui.horizontal(|ui| {
                ui.label("提前提醒（秒）");
                ui.add(egui::Slider::new(&mut settings.lead_time_seconds, 0..=120));
            });
            ui.horizontal(|ui| {
                ui.label("覆盖层交互热键");
                ui.text_edit_singleline(&mut self.hotkey_editor);
                if ui.button("应用").clicked() {
                    let value = (!self.hotkey_editor.trim().is_empty())
                        .then(|| self.hotkey_editor.trim().to_owned());
                    match self.platform.configure_hotkey(value.as_deref()) {
                        Ok(()) => settings.hotkey = value,
                        Err(error) => self.controller.record_external_diagnostic(error),
                    }
                }
            });
            ui.small("示例格式：Control+Shift+KeyO；留空表示不注册默认热键。");
            if settings != *self.controller.settings() {
                let update = self.controller.update_settings(settings);
                self.push_alerts(update.new_alerts);
                self.save_settings();
            }

            ui.separator();
            ui.heading("诊断");
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.controller.diagnostics().is_empty() {
                        ui.label("暂无诊断。");
                    } else {
                        for line in self.controller.diagnostics() {
                            ui.label(line);
                        }
                    }
                });
            ui.separator();
            ui.small(format!("设置文件：{}", self.store.path().display()));
            ui.small("关闭本窗口后程序保留在托盘；从托盘菜单可重新打开或退出。");
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
            self.push_alerts(update.new_alerts);
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
            self.push_alerts(update.new_alerts);
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
                    self.push_alerts(update.new_alerts);
                }
                if anchored && ui.button("清除").clicked() {
                    let update = self.controller.clear_stage_anchor(stage_id.clone());
                    self.push_alerts(update.new_alerts);
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
                self.push_alerts(update.new_alerts);
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

    fn show_overlay(&mut self, ui: &mut egui::Ui) {
        if self.controller.view().session_id.is_none() {
            return;
        }
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
            .take(6)
            .map(|event| {
                (
                    event.remaining_milliseconds,
                    event.timing,
                    self.controller.event_label(&event.event_id),
                )
            })
            .collect::<Vec<_>>();
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(7, 14, 24, 225))
            .corner_radius(10.0)
            .inner_margin(14.0)
            .show(ui, |ui| {
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
        if self.interactive_overlay {
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                self.interactive_overlay = false;
            }
            if let Some(position) =
                ui.input(|input| input.viewport().outer_rect.map(|rect| rect.min))
            {
                let new_position = [position.x, position.y];
                if new_position != self.controller.settings().overlay_position {
                    let mut settings = self.controller.settings().clone();
                    settings.overlay_position = new_position;
                    self.controller.update_settings(settings);
                    self.save_settings();
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
                .with_title("SC2 Copilot 设置与诊断")
                .with_inner_size([720.0, 760.0])
                .with_min_inner_size([620.0, 640.0])
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
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(
                !self.interactive_overlay,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(self.interactive_overlay));
            self.applied_interactive_overlay = Some(self.interactive_overlay);
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show_overlay(ui);
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
    upcoming: &[(u64, EventTiming, String)],
    interactive: bool,
) {
    ui.heading(RichText::new(map_name).color(Color32::from_rgb(100, 215, 255)));
    ui.label(format!(
        "游戏时间 {}{}",
        format_time(view.game_time_milliseconds.unwrap_or_default()),
        if interactive {
            " · 交互模式（Esc 退出）"
        } else {
            ""
        }
    ));
    if !alerts.is_empty() {
        ui.separator();
        for alert in alerts {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(120, 55, 15, 230))
                .corner_radius(6.0)
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.strong(format!("即将发生 · {}", format_timing(alert.timing)));
                    for label in &alert.event_labels {
                        ui.label(label);
                    }
                });
        }
    }
    ui.separator();
    ui.strong("接下来");
    if upcoming.is_empty() {
        ui.label("当前上下文没有可运行的后续事件。");
    } else {
        for (remaining_milliseconds, timing, label) in upcoming {
            ui.horizontal(|ui| {
                ui.monospace(format_remaining(*remaining_milliseconds));
                ui.label(format!("{} · {}", format_timing(*timing), label));
            });
        }
    }
}

fn overlay_interaction_available(connection: ConnectionState, has_session: bool) -> bool {
    connection == ConnectionState::InGame && has_session
}

fn connection_text(connection: ConnectionState) -> &'static str {
    match connection {
        ConnectionState::Disconnected => "未连接 / 游戏未启动",
        ConnectionState::Menu => "已连接，位于菜单",
        ConnectionState::InGame => "对局中",
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn format_timing(timing: EventTiming) -> String {
    match timing {
        EventTiming::Exact { milliseconds } => format_time(milliseconds),
        EventTiming::Window {
            earliest_milliseconds,
            latest_milliseconds,
        } => format!(
            "{}–{}",
            format_time(earliest_milliseconds),
            format_time(latest_milliseconds)
        ),
    }
}

fn format_remaining(milliseconds: u64) -> String {
    let seconds = milliseconds.div_ceil(1_000);
    format!("-{:02}:{:02}", seconds / 60, seconds % 60)
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
    use super::{ConnectionState, overlay_interaction_available};

    #[test]
    fn overlay_interaction_requires_an_active_game_session() {
        assert!(overlay_interaction_available(ConnectionState::InGame, true));
        assert!(!overlay_interaction_available(
            ConnectionState::InGame,
            false
        ));
        assert!(!overlay_interaction_available(
            ConnectionState::Disconnected,
            true
        ));
        assert!(!overlay_interaction_available(ConnectionState::Menu, false));
    }
}
