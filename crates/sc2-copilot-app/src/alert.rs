use sc2_copilot_core::AlertBatch;

pub trait AlertPlayer {
    fn play(&mut self, batch: &AlertBatch) -> Result<(), String>;

    fn status(&self) -> &str;
}

#[derive(Debug, Default)]
pub struct NoopAlertPlayer;

impl AlertPlayer for NoopAlertPlayer {
    fn play(&mut self, _batch: &AlertBatch) -> Result<(), String> {
        Ok(())
    }

    fn status(&self) -> &str {
        "未配置（提醒接口已保留）"
    }
}
