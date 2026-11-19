//! ADR 0003 : le hub est la seule crate sous AGPL, et aucune crate MIT/Apache
//! n'en dépend. Ce test lit les `Cargo.toml` du dépôt pour le garantir.

#![allow(clippy::unwrap_used)]

use std::path::Path;

fn read_toml(path: &Path) -> toml::Table {
    let text = std::fs::read_to_string(path).unwrap();
    text.parse::<toml::Table>().unwrap()
}

/// Vrai si une des sections de dépendances du manifeste nomme la crate `hub`.
fn depends_on_hub(manifest: &toml::Table) -> bool {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .any(|section| {
            manifest
                .get(*section)
                .and_then(|value| value.as_table())
                .is_some_and(|table| {
                    table.contains_key("hub")
                        || table
                            .values()
                            .any(|dep| dep.get("package").and_then(|p| p.as_str()) == Some("hub"))
                })
        })
}

#[test]
fn aucune_crate_mit_apache_ne_depend_du_hub() {
    let hub_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = hub_dir.parent().unwrap();
    let root = crates_dir.parent().unwrap();

    // Le workspace ne propose pas le hub comme dépendance partagée.
    let workspace = read_toml(&root.join("Cargo.toml"));
    let shared = workspace["workspace"]
        .get("dependencies")
        .and_then(|d| d.as_table())
        .unwrap();
    assert!(!shared.contains_key("hub"));

    let mut checked = 0;
    for entry in std::fs::read_dir(crates_dir).unwrap() {
        let dir = entry.unwrap().path();
        let manifest_path = dir.join("Cargo.toml");
        if dir == hub_dir || !manifest_path.exists() {
            continue;
        }
        let manifest = read_toml(&manifest_path);
        assert!(
            !depends_on_hub(&manifest),
            "{} dépend du hub (interdit par l'ADR 0003)",
            manifest_path.display()
        );
        // Les autres crates gardent la licence du workspace (MIT OR Apache-2.0).
        let license = &manifest["package"]["license"];
        assert!(
            license.get("workspace").and_then(|w| w.as_bool()) == Some(true),
            "{} : licence différente de celle du workspace",
            manifest_path.display()
        );
        checked += 1;
    }
    assert!(checked >= 5, "trop peu de crates trouvées : {checked}");

    let hub = read_toml(&hub_dir.join("Cargo.toml"));
    assert_eq!(
        hub["package"]["license"].as_str(),
        Some("AGPL-3.0-or-later")
    );
    assert!(hub_dir.join("LICENSE-AGPL").exists());
}
