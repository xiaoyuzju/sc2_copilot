#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformAction {
    ShowSettings,
    ToggleInteraction,
    Quit,
}

#[cfg(windows)]
mod windows {
    use std::sync::{
        Arc, Mutex, OnceLock, Weak,
        mpsc::{self, Receiver, Sender},
    };

    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuId, MenuItem},
    };
    use windows_sys::Win32::{
        Foundation::{GetLastError, SetLastError},
        UI::WindowsAndMessaging::{
            FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        },
    };

    use super::PlatformAction;

    type WakeEventLoop = Arc<dyn Fn() + Send + Sync>;

    struct MenuEventTarget {
        sender: Sender<MenuEvent>,
        receiver: Mutex<Receiver<MenuEvent>>,
        wake_event_loop: WakeEventLoop,
    }

    impl MenuEventTarget {
        fn new(wake_event_loop: WakeEventLoop) -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                sender,
                receiver: Mutex::new(receiver),
                wake_event_loop,
            }
        }

        fn forward(&self, event: MenuEvent) {
            let _ = self.sender.send(event);
            (self.wake_event_loop)();
        }

        fn try_recv(&self) -> Option<MenuEvent> {
            self.receiver
                .lock()
                .expect("menu event lock poisoned")
                .try_recv()
                .ok()
        }
    }

    struct MenuEventDispatcher {
        targets: Mutex<Vec<Weak<MenuEventTarget>>>,
    }

    impl MenuEventDispatcher {
        fn new() -> Self {
            Self {
                targets: Mutex::new(Vec::new()),
            }
        }

        fn subscribe(&self, target: &Arc<MenuEventTarget>) {
            self.targets
                .lock()
                .expect("menu target lock poisoned")
                .push(Arc::downgrade(target));
        }

        fn dispatch(&self, event: MenuEvent) {
            let live_targets = {
                let mut targets = self.targets.lock().expect("menu target lock poisoned");
                let live_targets = targets.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
                targets.retain(|target| target.strong_count() > 0);
                live_targets
            };
            for target in live_targets {
                target.forward(event.clone());
            }
        }
    }

    static MENU_EVENT_DISPATCHER: OnceLock<MenuEventDispatcher> = OnceLock::new();

    fn menu_event_target(wake_event_loop: WakeEventLoop) -> Arc<MenuEventTarget> {
        let dispatcher = MENU_EVENT_DISPATCHER.get_or_init(|| {
            let dispatcher = MenuEventDispatcher::new();
            MenuEvent::set_event_handler(Some(|event| {
                if let Some(dispatcher) = MENU_EVENT_DISPATCHER.get() {
                    dispatcher.dispatch(event);
                }
            }));
            dispatcher
        });
        let target = Arc::new(MenuEventTarget::new(wake_event_loop));
        dispatcher.subscribe(&target);
        target
    }

    pub struct PlatformIntegration {
        _tray: Option<TrayIcon>,
        show_settings_id: Option<MenuId>,
        toggle_interaction_id: Option<MenuId>,
        quit_id: Option<MenuId>,
        hotkey_manager: Option<GlobalHotKeyManager>,
        registered_hotkey: Option<HotKey>,
        menu_events: Arc<MenuEventTarget>,
        tray_status: String,
        hotkey_status: String,
    }

    impl PlatformIntegration {
        pub fn new(
            hotkey: Option<&str>,
            wake_event_loop: impl Fn() + Send + Sync + 'static,
        ) -> (Self, Vec<String>) {
            let mut diagnostics = Vec::new();
            let menu_events = menu_event_target(Arc::new(wake_event_loop));
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
                menu_events,
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
            while let Some(event) = self.menu_events.try_recv() {
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

    pub fn make_window_nonactivating(title: &str) -> Result<bool, String> {
        let title = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if window.is_null() {
            return Ok(false);
        }

        unsafe { SetLastError(0) };
        let current_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
        let error = unsafe { GetLastError() };
        if current_style == 0 && error != 0 {
            return Err(format!(
                "读取锁定按钮窗口样式失败：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }

        let requested_style = current_style | WS_EX_NOACTIVATE as isize | WS_EX_TOOLWINDOW as isize;
        if requested_style == current_style {
            return Ok(true);
        }

        unsafe { SetLastError(0) };
        let previous_style = unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, requested_style) };
        let error = unsafe { GetLastError() };
        if previous_style == 0 && error != 0 {
            return Err(format!(
                "设置锁定按钮窗口样式失败：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        let updated = unsafe {
            SetWindowPos(
                window,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if updated == 0 {
            return Err(format!(
                "刷新锁定按钮窗口样式失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(true)
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

    #[cfg(test)]
    mod tests {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        use super::{MenuEvent, MenuEventDispatcher, MenuEventTarget, MenuId};

        #[test]
        fn menu_event_dispatcher_fans_out_and_does_not_retain_dropped_targets() {
            let dispatcher = MenuEventDispatcher::new();
            let first_wake_count = Arc::new(AtomicUsize::new(0));
            let first_counter = Arc::clone(&first_wake_count);
            let first = Arc::new(MenuEventTarget::new(Arc::new(move || {
                first_counter.fetch_add(1, Ordering::Relaxed);
            })));
            let first_weak = Arc::downgrade(&first);
            dispatcher.subscribe(&first);

            let second_wake_count = Arc::new(AtomicUsize::new(0));
            let second_counter = Arc::clone(&second_wake_count);
            let second = Arc::new(MenuEventTarget::new(Arc::new(move || {
                second_counter.fetch_add(1, Ordering::Relaxed);
            })));
            dispatcher.subscribe(&second);

            dispatcher.dispatch(MenuEvent {
                id: MenuId::new("first"),
            });
            assert_eq!(first_wake_count.load(Ordering::Relaxed), 1);
            assert_eq!(second_wake_count.load(Ordering::Relaxed), 1);
            assert_eq!(
                first.try_recv().expect("event should be forwarded").id(),
                &MenuId::new("first")
            );
            assert_eq!(
                second.try_recv().expect("event should be forwarded").id(),
                &MenuId::new("first")
            );

            drop(first);
            dispatcher.dispatch(MenuEvent {
                id: MenuId::new("second"),
            });

            assert!(first_weak.upgrade().is_none());
            assert_eq!(first_wake_count.load(Ordering::Relaxed), 1);
            assert_eq!(second_wake_count.load(Ordering::Relaxed), 2);
        }
    }
}

#[cfg(not(windows))]
mod windows {
    use super::PlatformAction;

    pub struct PlatformIntegration;

    impl PlatformIntegration {
        pub fn new(
            _hotkey: Option<&str>,
            _wake_event_loop: impl Fn() + Send + Sync + 'static,
        ) -> (Self, Vec<String>) {
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

    pub fn make_window_nonactivating(_title: &str) -> Result<bool, String> {
        Ok(true)
    }
}

pub use windows::{PlatformIntegration, make_window_nonactivating};
