# Keep game integration read-only

SC2 Copilot 只实现依赖游戏状态或游戏画面的观察型能力，并提供最小产品外壳。它不注入或读取游戏进程内存，不修改游戏文件，不模拟玩家输入，不控制游戏单位；也不复刻 Keiframe 的通用管理界面、Excel 导入导出或视觉样式。这个边界在保留完整辅助价值的同时，避免产品演变为游戏控制工具或 Keiframe 的界面克隆。
