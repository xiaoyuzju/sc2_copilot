use sc2_copilot_core::ScheduleCatalog;

const CATALOG_JSON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/maps/catalog.json"
));

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt().with_target(false).try_init()?;
    let catalog = ScheduleCatalog::from_json(CATALOG_JSON)?;
    tracing::info!(map_count = catalog.map_count(), "SC2 Copilot initialized");
    Ok(())
}
