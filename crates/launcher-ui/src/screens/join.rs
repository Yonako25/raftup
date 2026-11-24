//! Écran « Rejoindre » : coller un lien d'invitation `raftup://`, vérifier où il mène,
//! puis continuer vers la Bibliothèque (ADR 0020, décisions 1, 6 et 7).
//!
//! Rien ne se connecte et rien ne se lance tout seul : l'écran montre l'hôte en clair,
//! et c'est le joueur qui clique sur « Continuer ».

use egui::{RichText, Ui};
use launcher_core::invite::{parse_invite, Invite};
use launcher_core::{Catalog, InstallStatus};

use super::library::{self, LibraryState, Scan};
use crate::app::Page;
use crate::theme;
use crate::widgets;

/// Longueur maximale d'un nom de slot Archipelago, en caractères.
const SLOT_MAX: usize = 16;

/// Ce que l'écran garde d'une image à l'autre.
#[derive(Default)]
pub struct JoinState {
    /// Le texte du champ « Collez le lien d'invitation ».
    link: String,
    /// Le lien une fois lu avec succès.
    invite: Option<Invite>,
    /// Le slot : repris du lien, ou saisi s'il manque.
    slot: String,
    /// Message d'erreur (lien refusé, slot manquant).
    error: Option<String>,
    /// Consignes affichées quand le jeu n'est pas dans le catalogue.
    instructions: Option<String>,
    /// Presse-papiers déjà regardé depuis l'arrivée sur l'écran (une fois par visite).
    clipboard_checked: bool,
    /// Le lien du champ vient du presse-papiers (on le dit au joueur).
    from_clipboard: bool,
}

impl JoinState {
    /// Une invitation lue et pas encore suivie ? Rend le jeu du lien (`None` dedans :
    /// le lien ne dit pas le jeu). L'Accueil la rappelle au joueur.
    pub fn pending_invite(&self) -> Option<Option<&str>> {
        self.invite.as_ref().map(|invite| invite.game.as_deref())
    }

    /// Tests : une invitation déjà lue, comme après un clic sur un lien.
    #[cfg(test)]
    pub(crate) fn with_invite(link: &str) -> JoinState {
        let mut state = JoinState::without_clipboard();
        receive(&mut state, link);
        state
    }

    /// Tests de l'interface : le presse-papiers du système est déjà « regardé »,
    /// donc jamais lu (le résultat ne dépend pas de ce que le PC a copié).
    #[cfg(test)]
    pub(crate) fn without_clipboard() -> JoinState {
        JoinState {
            clipboard_checked: true,
            ..JoinState::default()
        }
    }
}

/// Où mène « Continuer ».
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Un jeu du catalogue : sa fiche dans la Bibliothèque. `installed` : les mods sont posés.
    Library { game_id: String, installed: bool },
    /// Jeu absent du catalogue (ou du lien) : le joueur saisit l'adresse dans son jeu.
    OutsideCatalog,
}

/// Cherche le jeu du catalogue dont `ap_game` est celui du lien. Fonction pure : testée
/// sans interface.
pub fn destination(catalog: &Catalog, scan: Option<&Scan>, invite: &Invite) -> Destination {
    let Some(ap_game) = invite.game.as_deref() else {
        return Destination::OutsideCatalog;
    };
    // `games()` rend les profils dans un ordre stable : le résultat est reproductible.
    let Some(profile) = catalog.games().find(|p| p.game.ap_game == ap_game) else {
        return Destination::OutsideCatalog;
    };
    let game_id = profile.game.id.clone();
    let installed = scan
        .and_then(|s| s.status.get(&game_id))
        .is_some_and(|status| {
            matches!(
                status,
                Ok(InstallStatus::Installed | InstallStatus::UpdateAvailable)
            )
        });
    Destination::Library { game_id, installed }
}

/// Un lien est arrivé de Windows (ou du presse-papiers) : on remplit le champ et on l'analyse.
pub fn receive(state: &mut JoinState, link: &str) {
    state.link = link.trim().to_string();
    state.from_clipboard = false;
    // Un lien vient d'arriver : le presse-papiers ne doit pas le remplacer pendant
    // cette visite de l'écran.
    state.clipboard_checked = true;
    read_link(state);
}

/// Le joueur quitte l'écran : à son retour, le presse-papiers sera relu.
pub fn on_leave(state: &mut JoinState) {
    state.clipboard_checked = false;
}

/// Le lien d'invitation à prendre dans le presse-papiers, s'il y en a un : texte qui
/// commence par `raftup://`, différent du champ actuel. Un lien `raftup://hub/…` n'est
/// pas une invitation (il demande une confirmation à part) : il est laissé de côté.
/// Fonction pure : testée sans presse-papiers.
fn invite_in_clipboard<'a>(clipboard: &'a str, current: &str) -> Option<&'a str> {
    let text = clipboard.trim();
    // Un lien tient sur une ligne : un long texte copié n'est pas lu.
    let looks_like_link = text.len() <= 2048 && !text.contains(['\n', '\r']);
    let is_invite = text.starts_with("raftup://") && !launcher_core::rafthub::is_hub_link(text);
    (looks_like_link && is_invite && text != current.trim()).then_some(text)
}

/// Texte du presse-papiers du système, ou `None` (vide, image, accès refusé…).
/// Rien n'est écrit au journal : le presse-papiers peut contenir un secret.
fn read_clipboard() -> Option<String> {
    // `arboard::Clipboard` s'ouvre à chaque lecture : rare (une fois par visite).
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Arrivée sur l'écran : si le presse-papiers contient un lien d'invitation, il est lu
/// tout seul (plan de l'interface, étape 1). Rien ne se connecte : le joueur vérifie
/// l'hôte puis clique « Continuer », comme pour un lien collé à la main.
fn check_clipboard(state: &mut JoinState) {
    if state.clipboard_checked {
        return;
    }
    state.clipboard_checked = true;
    let Some(text) = read_clipboard() else {
        return;
    };
    if let Some(link) = invite_in_clipboard(&text, &state.link) {
        let link = link.to_string();
        receive(state, &link);
        state.from_clipboard = true;
    }
}

/// Analyse le texte du champ. Les erreurs de `InviteError` sont montrées telles quelles.
fn read_link(state: &mut JoinState) {
    state.instructions = None;
    match parse_invite(state.link.trim()) {
        Ok(invite) => {
            state.slot = invite.slot.clone().unwrap_or_default();
            state.invite = Some(invite);
            state.error = None;
        }
        Err(e) => {
            state.invite = None;
            state.error = Some(e.to_string());
        }
    }
}

/// Vrai pour un lien qui a l'air entier (`raftup://…` avec une adresse) : c'est ce
/// qu'on obtient en collant une invitation, pas en la tapant lettre par lettre.
fn looks_complete(link: &str) -> bool {
    let link = link.trim().to_ascii_lowercase();
    link.starts_with("raftup://") && link.contains("addr=")
}

/// « hôte:port », en clair.
fn host_text(invite: &Invite) -> String {
    format!("{}:{}", invite.address.host, invite.address.port)
}

pub fn show(ui: &mut Ui, state: &mut JoinState, library: &mut LibraryState) -> Option<Page> {
    let mut goto = None;
    let ctx = ui.ctx().clone();
    check_clipboard(state);
    super::page(ui, |ui| {
        super::header(ui, "Parties › Rejoindre", "Rejoindre une partie", |ui| {
            if widgets::ghost_button(ui, "Retour") {
                goto = Some(Page::Hosted);
            }
        });
        if let Some(message) = &state.error {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }

        widgets::panel(ui, "Lien d'invitation", |ui| {
            ui.label(widgets::dim("Colle le lien d'invitation").size(13.0));
            let edit = ui.add(
                egui::TextEdit::singleline(&mut state.link)
                    .hint_text("raftup://join?v=1&addr=…")
                    .desired_width(f32::INFINITY),
            );
            // Modifier le texte annule la lecture précédente : on ne continue jamais
            // avec un lien différent de celui qu'on a montré.
            if edit.changed() {
                state.invite = None;
                state.instructions = None;
                state.from_clipboard = false;
                // Un lien complet collé est lu tout de suite : pas besoin de cliquer
                // « Lire le lien » (le bouton reste pour un lien tapé à la main).
                if looks_complete(&state.link) {
                    read_link(state);
                }
            }
            if state.from_clipboard {
                ui.label(widgets::dim("Lien trouvé dans le presse-papiers.").size(12.0));
            }
            ui.add_space(6.0);
            if widgets::primary_button(ui, "Lire le lien", !state.link.trim().is_empty()) {
                read_link(state);
            }
        });

        let Some(invite) = state.invite.clone() else {
            return;
        };
        ui.add_space(12.0);
        widgets::panel(ui, "Vérifie avant de continuer", |ui| {
            ui.label(widgets::dim("Serveur").size(13.0));
            // L'hôte est montré en clair et en grand : c'est lui que le joueur doit reconnaître.
            ui.label(
                RichText::new(host_text(&invite))
                    .size(26.0)
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            if invite.fingerprint.is_some() {
                ui.label(widgets::dim("Partie chiffrée par Raftup.").size(13.0))
                    .on_hover_text(
                        "Les outils de Raftup vérifient le certificat du lien ; le mod du jeu \
                         se connecte en clair.",
                    );
            } else if !invite.address.secure {
                ui.label(
                    RichText::new(
                        "Connexion non chiffrée : le mot de passe de la salle passera en clair.",
                    )
                    .color(theme::AMBER),
                );
            }
            ui.add_space(8.0);
            ui.label(widgets::dim("Jeu").size(13.0));
            ui.label(
                RichText::new(invite.game.as_deref().unwrap_or("Non précisé"))
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            ui.add_space(8.0);
            ui.label(widgets::dim("Joueur").size(13.0));
            if invite.slot.is_some() {
                ui.label(
                    RichText::new(&state.slot)
                        .strong()
                        .family(theme::bold_family())
                        .color(theme::TEXT),
                );
            } else {
                ui.add(
                    egui::TextEdit::singleline(&mut state.slot)
                        .hint_text("Ton nom de joueur")
                        .char_limit(SLOT_MAX)
                        .desired_width(220.0),
                );
            }
            ui.add_space(10.0);
            if widgets::primary_button(ui, "Continuer", true) {
                continue_to(state, library, &invite, &ctx, &mut goto);
            }
        });

        if let Some(text) = &state.instructions {
            ui.add_space(12.0);
            widgets::panel(ui, "Jeu hors catalogue", |ui| {
                ui.label(RichText::new(text).color(theme::TEXT));
            });
        }
    });
    goto
}

/// Clic sur « Continuer » : Bibliothèque pré-remplie, ou copie dans le presse-papiers.
fn continue_to(
    state: &mut JoinState,
    library: &mut LibraryState,
    invite: &Invite,
    ctx: &egui::Context,
    goto: &mut Option<Page>,
) {
    let slot = state.slot.trim().to_string();
    if slot.is_empty() {
        state.error = Some("Indique ton nom de joueur.".to_string());
        return;
    }
    state.error = None;
    // Adresse donnée au mod du jeu : sans schéma pour un certificat généré par Raftup
    // (le mod essaie wss puis retombe sur ws, ADR 0043).
    let url = invite.address_for_game();
    let dest = library
        .catalog()
        .map(|catalog| destination(catalog, library.scan_result(), invite))
        .unwrap_or(Destination::OutsideCatalog);
    match dest {
        Destination::Library { game_id, .. } => {
            library::open_with_invite(library, &game_id, &url, &slot);
            *goto = Some(Page::Library);
        }
        Destination::OutsideCatalog => {
            // Même format que le lancement manuel : adresse, retour à la ligne, slot.
            ctx.copy_text(format!("{url}\n{slot}"));
            state.instructions = Some(
                "Ouvre ton jeu et saisis l'adresse et le nom de joueur copiés. \
                 (Adresse et nom de joueur copiés dans le presse-papiers.)"
                    .to_string(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use launcher_core::invite::ServerAddress;

    /// Profil de démonstration : il n'existe que dans les tests.
    const DEMO: &str = r#"
format_version = 1

[game]
id = "demo"
name = "Jeu de démo"
ap_game = "Demo Game"
status = "available"

[game.detect]
expected_exe = "Demo.exe"

[credits]
authors = ["Quelqu'un"]
license = "MIT"

[[steps]]
id = "le-mod"
kind = "mod"
name = "Le mod"
version = "1.0.0"
source = { url = "https://github.com/x/y/releases/download/v1/m.zip" }
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
install = { copy_to = "{game_dir}/Mods/m.dll" }

[launch]
exe = "{game_dir}/Demo.exe"
connect = { method = "cli_args", args = ["--connect", "{address}", "--name", "{slot}"] }
"#;

    fn catalog() -> Catalog {
        Catalog::from_texts(&[("demo.toml", DEMO)]).unwrap()
    }

    fn invite(game: Option<&str>) -> Invite {
        Invite {
            address: ServerAddress {
                secure: true,
                host: "h.example".into(),
                port: 38281,
            },
            slot: Some("Alice".into()),
            game: game.map(str::to_string),
            fingerprint: None,
        }
    }

    fn scan_with(status: InstallStatus) -> Scan {
        let mut scan = Scan::default();
        scan.status.insert("demo".into(), Ok(status));
        scan
    }

    #[test]
    fn jeu_installe() {
        let scan = scan_with(InstallStatus::Installed);
        assert_eq!(
            destination(&catalog(), Some(&scan), &invite(Some("Demo Game"))),
            Destination::Library {
                game_id: "demo".into(),
                installed: true
            }
        );
    }

    #[test]
    fn jeu_du_catalogue_non_installe() {
        let scan = scan_with(InstallStatus::NotInstalled);
        for scan in [Some(&scan), None] {
            assert_eq!(
                destination(&catalog(), scan, &invite(Some("Demo Game"))),
                Destination::Library {
                    game_id: "demo".into(),
                    installed: false
                }
            );
        }
    }

    #[test]
    fn jeu_hors_catalogue() {
        assert_eq!(
            destination(&catalog(), None, &invite(Some("Autre Jeu"))),
            Destination::OutsideCatalog
        );
    }

    #[test]
    fn jeu_absent_du_lien() {
        assert_eq!(
            destination(&catalog(), None, &invite(None)),
            Destination::OutsideCatalog
        );
    }

    #[test]
    fn recevoir_un_lien_valide_remplit_et_analyse() {
        let mut state = JoinState::default();
        let link = launcher_core::invite::make_invite(&invite(Some("Demo Game"))).unwrap();
        receive(&mut state, &format!(" {link} "));
        // Les espaces autour sont retirés du champ.
        assert_eq!(state.link, link);
        assert_eq!(state.invite, Some(invite(Some("Demo Game"))));
        assert_eq!(state.slot, "Alice");
        assert!(state.error.is_none());
        // Le lien reçu n'est pas remplacé par le presse-papiers pendant cette visite ;
        // en quittant l'écran, le presse-papiers sera relu au retour.
        assert!(state.clipboard_checked);
        on_leave(&mut state);
        assert!(!state.clipboard_checked);
    }

    #[test]
    fn lien_du_presse_papiers() {
        let link = "raftup://join?v=1&addr=wss%3A%2F%2Fh.example%3A38281";
        // Un lien d'invitation, espaces autour : pris, sans les espaces.
        assert_eq!(invite_in_clipboard(&format!("  {link}\n"), ""), Some(link));
        // Déjà dans le champ : rien à faire.
        assert_eq!(invite_in_clipboard(link, link), None);
        // Autre texte, lien du hub, texte sur plusieurs lignes : laissés de côté.
        assert_eq!(invite_in_clipboard("mon mot de passe", ""), None);
        assert_eq!(invite_in_clipboard("raftup://hub/hub.example", ""), None);
        assert_eq!(invite_in_clipboard(&format!("{link}\nsuite"), ""), None);
        assert_eq!(invite_in_clipboard("", ""), None);
    }

    #[test]
    fn recevoir_un_lien_invalide_montre_l_erreur() {
        let mut state = JoinState::default();
        receive(&mut state, "https://exemple.org");
        assert!(state.invite.is_none());
        assert_eq!(
            state.error.as_deref(),
            Some("ce n'est pas un lien d'invitation Raftup")
        );
        assert_eq!(state.link, "https://exemple.org");
    }
}
