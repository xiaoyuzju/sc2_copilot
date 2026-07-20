#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformAction {
    ShowSettings,
    ToggleInteraction,
    Quit,
}

#[cfg(windows)]
mod windows {
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuId, MenuItem},
    };

    use super::PlatformAction;

    pub struct PlatformIntegration {
        _tray: Option<TrayIcon>,
        show_settings_id: Option<MenuId>,
        toggle_interaction_id: Option<MenuId>,
        quit_id: Option<MenuId>,
        hotkey_manager: Option<GlobalHotKeyManager>,
        registered_hotkey: Option<HotKey>,
        tray_status: String,
        hotkey_status: String,
    }

    impl PlatformIntegration {
        pub fn new(hotkey: Option<&str>) -> (Self, Vec<String>) {
            let mut diagnostics = Vec::new();
            let (tray, show_settings_id, toggle_interaction_id, quit_id, tray_status) =
                match create_tray() {
                    Ok((tray, show, toggle, quit)) => (
                        Some(tray),
                        Some(show),
                        Some(toggle),
                        Some(quit),
                        "已就绪".to_owned(),
                    ),
                    Err(error) => {
                        diagnostics.push(format!("托盘初始化失败：{error}"));
                        (None, None, None, None, format!("失败：{error}"))
                    }
                };

            let (hotkey_manager, hotkey_status) = match GlobalHotKeyManager::new() {
                Ok(manager) => (Some(manager), "未配置".to_owned()),
                Err(error) => {
                    diagnostics.push(format!("全局热键初始化失败：{error}"));
                    (None, format!("失败：{error}"))
                }
            };
            let mut integration = Self {
                _tray: tray,
                show_settings_id,
                toggle_interaction_id,
                quit_id,
                hotkey_manager,
                registered_hotkey: None,
                tray_status,
                hotkey_status,
            };
            if let Err(error) = integration.configure_hotkey(hotkey) {
                diagnostics.push(error);
            }
            (integration, diagnostics)
        }

        pub fn poll_actions(&self) -> Vec<PlatformAction> {
            let mut actions = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if self.show_settings_id.as_ref() == Some(event.id()) {
                    actions.push(PlatformAction::ShowSettings);
                } else if self.toggle_interaction_id.as_ref() == Some(event.id()) {
                    actions.push(PlatformAction::ToggleInteraction);
                } else if self.quit_id.as_ref() == Some(event.id()) {
                    actions.push(PlatformAction::Quit);
                }
            }
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == HotKeyState::Pressed
                    && self
                        .registered_hotkey
                        .is_some_and(|hotkey| hotkey.id() == event.id)
                {
                    actions.push(PlatformAction::ToggleInteraction);
                }
            }
            actions
        }

        pub fn configure_hotkey(&mut self, value: Option<&str>) -> Result<(), String> {
            let Some(manager) = self.hotkey_manager.as_ref() else {
                return Err("全局热键不可用，请使用托盘或设置窗口".to_owned());
            };
            if let Some(previous) = self.registered_hotkey.take()
                && let Err(error) = manager.unregister(previous)
            {
                self.hotkey_status = format!("注销旧热键失败：{error}");
                return Err(self.hotkey_status.clone());
            }
            let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
                self.hotkey_status = "未配置".to_owned();
                return Ok(());
            };
            let hotkey = value
                .parse::<HotKey>()
                .map_err(|error| format!("热键格式无效：{error}"))?;
            manager
                .register(hotkey)
                .map_err(|error| format!("热键注册失败：{error}"))?;
            self.registered_hotkey = Some(hotkey);
            self.hotkey_status = format!("已注册：{hotkey}");
            Ok(())
        }

        pub fn tray_status(&self) -> &str {
            &self.tray_status
        }

        pub fn hotkey_status(&self) -> &str {
            &self.hotkey_status
        }
    }

    fn create_tray() -> Result<(TrayIcon, MenuId, MenuId, MenuId), String> {
        let menu = Menu::new();
        let show = MenuItem::new("打开设置", true, None);
        let toggle = MenuItem::new("切换覆盖层交互", true, None);
        let quit = MenuItem::new("退出", true, None);
        menu.append_items(&[&show, &toggle, &quit])
            .map_err(|error| error.to_string())?;
        let icon = Icon::from_rgba(tray_pixels(), 32, 32).map_err(|error| error.to_string())?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("SC2 Copilot")
            .build()
            .map_err(|error| error.to_string())?;
        Ok((
            tray,
            show.id().clone(),
            toggle.id().clone(),
            quit.id().clone(),
        ))
    }

    fn tray_pixels() -> Vec<u8> {
        let mut pixels = Vec::with_capacity(32 * 32 * 4);
        for y in 0..32 {
            for x in 0..32 {
                let inside = (4..28).contains(&x) && (4..28).contains(&y);
                let accent = x >= 15 && (7..25).contains(&y);
                let color = if !inside {
                    [0, 0, 0, 0]
                } else if accent {
                    [90, 210, 255, 255]
                } else {
                    [18, 45, 75, 255]
                };
                pixels.extend_from_slice(&color);
            }
        }
        pixels
    }
}

#[cfg(not(windows))]
mod windows {
    use super::PlatformAction;

    pub struct PlatformIntegration;

    impl PlatformIntegration {
        pub fn new(_hotkey: Option<&str>) -> (Self, Vec<String>) {
            (
                Self,
                vec!["托盘与全局热键仅在 Windows 11 版本启用".to_owned()],
            )
        }

        pub fn poll_actions(&self) -> Vec<PlatformAction> {
            Vec::new()
        }

        pub fn configure_hotkey(&mut self, _value: Option<&str>) -> Result<(), String> {
            Err("全局热键仅在 Windows 11 版本启用".to_owned())
        }

        pub fn tray_status(&self) -> &str {
            "当前平台不支持"
        }

        pub fn hotkey_status(&self) -> &str {
            "当前平台不支持"
        }
    }
}

pub use windows::PlatformIntegration;
