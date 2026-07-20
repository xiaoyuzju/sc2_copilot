# Use egui and eframe for the product shell

SC2 Copilot 的设置窗口、诊断入口和游戏覆盖层统一使用原生 Rust 的 egui/eframe 实现，必要时只用 Win32 API 补强框架未覆盖的窗口行为。项目不引入 WebView 前端或独立的 JavaScript 构建链，以保持单一 Rust 技术栈，并直接利用原生透明、置顶、鼠标穿透和多窗口能力。覆盖层平时置顶且鼠标穿透，通过可配置全局热键临时进入交互模式；再次按热键或按 Esc 后恢复穿透，不模拟或转发任何游戏输入。事件提醒使用覆盖层内部的临时提示卡片，不调用 Windows 通知中心、不获取焦点，也不写入系统通知历史。
