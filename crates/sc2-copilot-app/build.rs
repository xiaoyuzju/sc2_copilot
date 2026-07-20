use std::{env, fs, path::PathBuf};

use sc2_copilot_core::ScheduleCatalog;

fn main() {
    let catalog_path = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("../../data/maps/catalog.json");

    println!("cargo::rerun-if-changed={}", catalog_path.display());

    let bytes = fs::read(&catalog_path).unwrap_or_else(|error| {
        panic!(
            "failed to read schedule catalog at {}: {error}",
            catalog_path.display()
        )
    });
    ScheduleCatalog::from_json(&bytes).unwrap_or_else(|error| {
        panic!(
            "invalid schedule catalog at {}: {error}",
            catalog_path.display()
        )
    });
}
