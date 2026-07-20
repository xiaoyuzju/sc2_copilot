fn main() {
    if let Err(error) = tracing_subscriber::fmt().with_target(false).try_init() {
        eprintln!("日志初始化失败：{error}");
    }
    if let Err(error) = sc2_copilot_app::desktop::run() {
        eprintln!("SC2 Copilot 启动失败：{error}");
    }
}
