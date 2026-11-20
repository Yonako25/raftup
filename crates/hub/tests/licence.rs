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

/// Empreinte SHA-256 de https://www.gnu.org/licenses/agpl-3.0.txt, récupéré
/// le 2026-11-20 (voir l'ADR 0003). Le fichier doit rester le texte exact de
/// la FSF : la moindre retouche change l'empreinte et fait échouer ce test.
const AGPL_SHA256: &str = "0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0";

#[test]
fn licence_agpl_est_le_texte_officiel_de_la_fsf() {
    use sha2::{Digest, Sha256};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("LICENSE-AGPL");
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();

    // Contrôles lisibles d'abord : ils disent ce qui ne va pas si l'empreinte diffère.
    assert!(
        !text.contains('\r'),
        "LICENSE-AGPL doit avoir des fins de ligne LF (voir .gitattributes)"
    );
    let mut lines = text.lines();
    assert_eq!(
        lines.next().map(str::trim),
        Some("GNU AFFERO GENERAL PUBLIC LICENSE")
    );
    assert_eq!(
        lines.next().map(str::trim),
        Some("Version 3, 19 November 2007")
    );
    assert!(text.contains("How to Apply These Terms to Your New Programs"));

    // `Sha256::digest` rend un tableau d'octets ; on l'écrit en hexadécimal.
    let digest = Sha256::digest(&bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex, AGPL_SHA256,
        "LICENSE-AGPL n'est plus le texte exact de gnu.org"
    );
}
