use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    thread,
    time::Duration,
};

use sc2_copilot_app::{LocalSc2HttpClient, MonitorReducer, Sc2StateSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("logs/sc2-monitor.jsonl"));
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)?;
    let mut writer = BufWriter::new(file);
    let client = LocalSc2HttpClient::new(Duration::from_millis(750))?;
    let mut source = Sc2StateSource::new(client);
    let mut monitor = MonitorReducer::default();

    eprintln!("SC2 6119 脱敏监控已启动：{}", output_path.display());
    loop {
        let poll = source.poll();
        if let Some(record) = monitor.observe(&poll) {
            let line = serde_json::to_string(&record)?;
            writeln!(writer, "{line}")?;
            writer.flush()?;
            println!("{line}");
        }
        thread::sleep(Duration::from_millis(250));
    }
}
