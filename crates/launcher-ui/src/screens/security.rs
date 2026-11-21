//! Choix du chiffrement de la partie, commun à « Créer une partie » et « Héberger »
//! (ADR 0043) : chiffré par Raftup (par défaut, aucun fichier), mon certificat PEM,
//! ou en clair.

use std::path::PathBuf;

use egui::{RichText, Ui};
use launcher_core::Encryption;

use super::library::load_paths;
use crate::theme;
use crate::widgets;

/// Le chiffrement choisi par l'hôte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Security {
    /// Certificat généré par Raftup, `wss` et `ws` sur le même port (ADR 0043).
    #[default]
    Raftup,
    /// Certificat et clé de l'hôte (fichiers PEM, ADR 0009) : `wss` seul.
    Pem,
    /// `ws` en clair, sur choix explicite (NF06).
    Plain,
}

/// Fichier PEM demandé par un clic sur « Choisir… ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PemFile {
    Cert,
    Key,
}

impl Security {
    /// Fin du résumé « Port … · adresse … · … » des options avancées.
    pub fn summary(self) -> &'static str {
        match self {
            Security::Raftup => "chiffré par Raftup (wss, sans fichier)",
            Security::Pem => "chiffré (wss, certificat PEM demandé)",
            Security::Plain => "en clair (ws)",
        }
    }

    /// Le schéma que verront les joueurs Raftup.
    pub fn scheme(self) -> &'static str {
        match self {
            Security::Plain => "ws",
            Security::Raftup | Security::Pem => "wss",
        }
    }
}

/// Le réglage de chiffrement du serveur, ou le message à montrer.
pub fn encryption(
    security: Security,
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
) -> Result<Encryption, String> {
    match security {
        Security::Plain => Ok(Encryption::Plain),
        Security::Pem => match (cert, key) {
            (Some(cert), Some(key)) => Ok(Encryption::Wss {
                cert: cert.clone(),
                key: key.clone(),
            }),
            _ => Err(
                "« Mon certificat » demande un certificat et une clé privée (fichiers PEM). \
                 Sans fichier, choisissez « Chiffré par Raftup »."
                    .into(),
            ),
        },
        Security::Raftup => match load_paths() {
            Ok(paths) => Ok(Encryption::Generated {
                dir: paths.tls_dir(),
            }),
            Err(e) => Err(format!(
                "Dossiers de Raftup introuvables pour le certificat de la partie : {e}"
            )),
        },
    }
}

/// Les trois cartes du choix, dans « Options avancées ». `file_row` dessine une ligne
/// « fichier choisi + Choisir… » et rend vrai au clic (propre à chaque écran).
/// Rend le fichier PEM à choisir, si l'hôte a cliqué sur « Choisir… ».
pub fn choice(
    ui: &mut Ui,
    security: &mut Security,
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
    file_row: fn(&mut Ui, &Option<PathBuf>) -> bool,
) -> Option<PemFile> {
    let mut pick = None;
    let raftup = widgets::choice_card(
        ui,
        *security == Security::Raftup,
        "Chiffré par Raftup — recommandé",
        widgets::dim(
            "Aucun fichier à fournir. Chiffré pour les joueurs qui passent par Raftup ; \
             un mod de jeu ordinaire se connecte en clair.",
        )
        .size(13.0),
        |_| {},
    );
    if raftup {
        *security = Security::Raftup;
    }
    ui.add_space(4.0);
    let pem = widgets::choice_card(
        ui,
        *security == Security::Pem,
        "Mon certificat (PEM)",
        widgets::dim("Pour un nom de domaine avec son certificat (Let's Encrypt…).").size(13.0),
        |ui| {
            ui.add_space(4.0);
            ui.label(widgets::dim("Certificat (PEM)").size(13.0));
            if file_row(ui, cert) {
                pick = Some(PemFile::Cert);
            }
            ui.label(widgets::dim("Clé privée (PEM)").size(13.0));
            if file_row(ui, key) {
                pick = Some(PemFile::Key);
            }
        },
    );
    if pem {
        *security = Security::Pem;
    }
    ui.add_space(4.0);
    let plain = widgets::choice_card(
        ui,
        *security == Security::Plain,
        "En clair (ws)",
        RichText::new("Le mot de passe de la salle passe en clair sur le réseau.")
            .color(theme::AMBER)
            .size(13.0),
        |_| {},
    );
    if plain {
        *security = Security::Plain;
    }
    pick
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chiffre_par_raftup_par_defaut_sans_fichier() {
        assert_eq!(Security::default(), Security::Raftup);
        // Rien à fournir : le certificat ira dans le dossier des données de Raftup.
        match encryption(Security::Raftup, &None, &None) {
            Ok(Encryption::Generated { dir }) => assert!(dir.ends_with("tls")),
            // Pas de dossier personnel (CI très restreinte) : le message le dit.
            Err(message) => assert!(message.contains("Dossiers de Raftup")),
            Ok(other) => panic!("{other:?}"),
        }
        assert_eq!(
            encryption(Security::Plain, &None, &None),
            Ok(Encryption::Plain)
        );
        let refused = encryption(Security::Pem, &None, &None).unwrap_err();
        assert!(refused.contains("certificat"));
        let cert = Some(PathBuf::from("c.pem"));
        let key = Some(PathBuf::from("k.pem"));
        assert_eq!(
            encryption(Security::Pem, &cert, &key),
            Ok(Encryption::Wss {
                cert: PathBuf::from("c.pem"),
                key: PathBuf::from("k.pem")
            })
        );
        assert_eq!(Security::Raftup.scheme(), "wss");
        assert_eq!(Security::Plain.scheme(), "ws");
    }
}
