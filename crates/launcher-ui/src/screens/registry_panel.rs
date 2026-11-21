//! Le registre de paquets signés dans la Bibliothèque (ADR 0032).
//!
//! À la première ouverture de la Bibliothèque, un thread à part lit l'index signé
//! (`REGISTRY_INDEX_URL`), vérifie sa signature, puis télécharge les profils des jeux
//! absents du catalogue embarqué. L'écran ajoute ces jeux à son catalogue (le catalogue
//! embarqué gagne toujours) et les marque « Registre ».
//!
//! Une erreur de réseau ou de signature n'empêche rien : la Bibliothèque garde son
//! catalogue embarqué et le dit en une ligne discrète, en bas de l'écran.
//!
//! Les apworlds du registre sont listées dans « Mondes du registre » ; « Installer » les
//! télécharge chez leur auteur, contrôle taille et sha256, puis les pose dans le dossier
//! des mondes (`launcher_core::registry::install_apworld`).
//!
//! En debug, `RAFTUP_DEV_REGISTRE=<dossier>` lit le registre dans un dossier local au lieu
//! du réseau (`index.json`, `index.json.minisig` et les paquets, rangés sous le dernier
//! segment de leur adresse) : de quoi regarder l'écran avant la mise en ligne. La
//! signature est vérifiée pareil, avec la vraie clé, sauf si `RAFTUP_DEV_REGISTRE_CLE`
//! donne une clé publique de test. `RAFTUP_DEV_REGISTRE_CONFIRMER=<nom>` ouvre la boîte
//! de confirmation de ce monde dès la lecture (captures de la macro).
//!
//! Chaque apworld porte le badge de son niveau de confiance (ADR 0045), expliqué au
//! survol. Avant d'installer un monde « Non vérifié » ou « Structure valide », une boîte
//! demande confirmation.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use launcher_core::download::{DownloadError, Fetcher};
use launcher_core::registry::{
    apply_registry, apworld_path, fetch_registry, install_apworld, ApworldInstall, Index,
    PackageKind, RegistryFetch, Trust, TrustLevel, REGISTRY_INDEX_URL, REGISTRY_PUBLIC_KEY,
};
use launcher_core::{Catalog, HttpFetcher};

use crate::{theme, widgets};

/// Où en est la lecture du registre.
enum Phase {
    /// Pas encore lancée.
    NotStarted,
    /// Désactivée (`RAFTUP_NO_REGISTRY`, pratique en développement).
    Disabled,
    /// Le thread lit l'index.
    Loading(Receiver<Result<RegistryFetch, String>>),
    /// Index lu et vérifié.
    Ready,
    /// Réseau, signature ou index refusé : message court, la Bibliothèque continue.
    Failed(String),
}

/// Une installation d'apworld en cours.
struct Install {
    name: String,
    receiver: Receiver<Result<ApworldInstall, String>>,
}

/// L'état du registre, gardé par l'écran Bibliothèque.
pub struct RegistryPanel {
    phase: Phase,
    /// Index vérifié (après `Ready`).
    index: Option<Index>,
    /// Jeux ajoutés au catalogue par le registre.
    added: BTreeSet<String>,
    /// Profils du registre écartés, avec la raison (affichés en survol).
    rejected: Vec<(String, String)>,
    /// Apworlds du registre déjà présentes dans le dossier des mondes.
    installed: BTreeSet<String>,
    install: Option<Install>,
    /// Résultat de la dernière installation : texte, en vert (`Ok`) ou en orange (`Err`).
    message: Option<Result<String, String>>,
    /// Apworld de niveau bas dont l'installation attend une confirmation (ADR 0045).
    confirm: Option<String>,
}

impl RegistryPanel {
    pub fn new() -> RegistryPanel {
        RegistryPanel {
            phase: Phase::NotStarted,
            index: None,
            added: BTreeSet::new(),
            rejected: Vec::new(),
            installed: BTreeSet::new(),
            install: None,
            message: None,
            confirm: None,
        }
    }

    /// Niveau de confiance d'une apworld de l'index (« non vérifié » si elle est inconnue).
    fn level_of(&self, name: &str) -> TrustLevel {
        self.index
            .as_ref()
            .and_then(|i| i.get(PackageKind::Apworld, name))
            .map_or(TrustLevel::Unverified, |p| p.trust.level)
    }

    /// Clic sur « Installer » : un monde de niveau bas demande d'abord une confirmation.
    fn request_install(&mut self, ctx: &egui::Context, name: &str, worlds_dir: &Path) {
        if needs_confirmation(self.level_of(name)) {
            self.confirm = Some(name.to_string());
        } else {
            self.start_install(ctx, name, worlds_dir.to_path_buf());
        }
    }

    /// Lance la lecture du registre une seule fois, dans un thread à part.
    /// `catalog` : le catalogue embarqué, pour ne télécharger que les jeux qui manquent.
    pub fn start(&mut self, ctx: &egui::Context, catalog: &Catalog) {
        if !matches!(self.phase, Phase::NotStarted) {
            return;
        }
        if std::env::var_os("RAFTUP_NO_REGISTRY").is_some() {
            self.phase = Phase::Disabled;
            return;
        }
        // Copies possédées par le thread (`move`) : il ne peut rien emprunter à l'écran.
        let catalog = catalog.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_registry(
                fetcher().as_ref(),
                REGISTRY_INDEX_URL,
                &public_key(),
                &catalog,
            )
            .map_err(|e| e.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.phase = Phase::Loading(receiver);
    }

    /// Lit les réponses des threads sans attendre. Ajoute au catalogue les jeux du registre
    /// et rend `true` si des jeux ont été ajoutés (il faut alors relancer la détection).
    pub fn poll(&mut self, catalog: &mut Catalog, worlds_dir: Option<&Path>) -> bool {
        let mut added_games = false;
        if let Phase::Loading(receiver) = &self.phase {
            match receiver.try_recv() {
                Ok(Ok(fetched)) => {
                    added_games = self.apply(catalog, fetched, worlds_dir);
                }
                Ok(Err(message)) => {
                    tracing::info!("registre ignoré : {message}");
                    self.phase = Phase::Failed(message);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.phase = Phase::Failed("lecture interrompue".to_string());
                }
            }
        }
        if let Some(install) = &self.install {
            match install.receiver.try_recv() {
                Ok(result) => {
                    let name = install.name.clone();
                    self.message = Some(match result {
                        Ok(ApworldInstall::Installed(path)) => {
                            self.installed.insert(name.clone());
                            Ok(format!("{name} installée : {}", path.display()))
                        }
                        Ok(ApworldInstall::AlreadyThere(_)) => {
                            self.installed.insert(name.clone());
                            Ok(format!("{name} était déjà installée."))
                        }
                        Err(e) => Err(format!("{name} : {e}")),
                    });
                    self.install = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.message = Some(Err("installation interrompue sans réponse".into()));
                    self.install = None;
                }
            }
        }
        added_games
    }

    /// Fusionne un registre lu : profils ajoutés au catalogue, apworlds déjà posées notées.
    /// Rend `true` si des jeux ont été ajoutés.
    fn apply(
        &mut self,
        catalog: &mut Catalog,
        fetched: RegistryFetch,
        worlds_dir: Option<&Path>,
    ) -> bool {
        let report = apply_registry(catalog, &fetched);
        for (name, reason) in &report.rejected {
            tracing::warn!("profil du registre « {name} » écarté : {reason}");
        }
        self.added = report.added.iter().cloned().collect();
        self.rejected = report.rejected;
        self.index = Some(fetched.index);
        if let Some(dir) = worlds_dir {
            self.refresh_installed(dir);
        }
        // Debug : ouvre tout de suite la confirmation d'un monde (captures de la macro).
        if cfg!(debug_assertions) {
            if let Ok(name) = std::env::var("RAFTUP_DEV_REGISTRE_CONFIRMER") {
                self.confirm = Some(name);
            }
        }
        self.phase = Phase::Ready;
        !self.added.is_empty()
    }

    /// Note les apworlds du registre déjà présentes (fichier `<nom>.apworld`).
    fn refresh_installed(&mut self, worlds_dir: &Path) {
        let Some(index) = &self.index else { return };
        self.installed = index
            .of_kind(PackageKind::Apworld)
            .filter(|p| apworld_path(worlds_dir, &p.name).exists())
            .map(|p| p.name.clone())
            .collect();
    }

    /// Le jeu vient-il du registre (et non du catalogue embarqué) ?
    pub fn is_from_registry(&self, game_id: &str) -> bool {
        self.added.contains(game_id)
    }

    /// Ligne discrète sur l'état du registre ; `None` s'il n'y a rien à dire.
    fn status_text(&self) -> Option<String> {
        match &self.phase {
            Phase::NotStarted | Phase::Disabled => None,
            Phase::Loading(_) => Some("Registre : lecture…".to_string()),
            // La raison complète (adresse, code HTTP…) est en survol : voir `hover_text`.
            Phase::Failed(_) => {
                Some("Registre indisponible : catalogue embarqué seul.".to_string())
            }
            Phase::Ready => {
                let total = self.index.as_ref().map_or(0, |i| i.packages.len());
                let mut text = format!("Registre signé : {total} paquet(s) vérifié(s)");
                if !self.added.is_empty() {
                    text.push_str(&format!(", {} jeu(x) ajouté(s)", self.added.len()));
                }
                if !self.rejected.is_empty() {
                    text.push_str(&format!(", {} écarté(s)", self.rejected.len()));
                }
                Some(text)
            }
        }
    }

    /// Détail affiché au survol de la ligne d'état : raison de l'échec, ou profils écartés.
    fn hover_text(&self) -> Option<String> {
        if let Phase::Failed(reason) = &self.phase {
            return Some(reason.clone());
        }
        if self.rejected.is_empty() {
            return None;
        }
        let reasons: Vec<String> = self
            .rejected
            .iter()
            .map(|(name, reason)| format!("{name} : {reason}"))
            .collect();
        Some(reasons.join("\n"))
    }

    /// Lance l'installation d'une apworld du registre dans un thread à part.
    fn start_install(&mut self, ctx: &egui::Context, name: &str, worlds_dir: PathBuf) {
        let (Some(index), None) = (&self.index, &self.install) else {
            return;
        };
        let index = index.clone();
        let owned_name = name.to_string();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = install_apworld(fetcher().as_ref(), &index, &owned_name, &worlds_dir)
                .map_err(|e| e.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        self.message = None;
        self.install = Some(Install {
            name: name.to_string(),
            receiver,
        });
    }
}

/// Clé publique du registre. En debug seulement, `RAFTUP_DEV_REGISTRE_CLE` la remplace,
/// pour regarder un registre local signé par une clé de test (jamais la vraie clé secrète).
fn public_key() -> String {
    if cfg!(debug_assertions) {
        if let Ok(key) = std::env::var("RAFTUP_DEV_REGISTRE_CLE") {
            return key;
        }
    }
    REGISTRY_PUBLIC_KEY.to_string()
}

/// Faut-il confirmer avant d'installer un monde de ce niveau ? Oui tant que la CI ne l'a
/// pas fait générer (ADR 0045).
pub fn needs_confirmation(level: TrustLevel) -> bool {
    level < TrustLevel::Generates
}

/// Texte affiché au survol du badge : ce que prouve le niveau, puis le détail de la CI.
pub fn trust_hover(trust: &Trust) -> String {
    let mut text = format!("{} — {}", trust.level.label(), trust.level.meaning());
    if let Some(detail) = &trust.detail {
        text.push_str("\n\n");
        text.push_str(detail);
    }
    text
}

/// Couleurs du badge (texte, fond), de plus en plus franches avec le niveau.
fn trust_colors(level: TrustLevel) -> (egui::Color32, egui::Color32) {
    match level {
        TrustLevel::Unverified => (theme::TEXT_DIM, theme::FIELD),
        TrustLevel::Valid => (theme::AMBER, egui::Color32::from_rgb(0x33, 0x27, 0x14)),
        TrustLevel::Generates => (theme::ITEM, theme::ACCENT_BG),
        TrustLevel::Fuzzed => (theme::GREEN, theme::GREEN_BG),
    }
}

/// Badge du niveau de confiance, avec son explication au survol.
fn trust_badge(ui: &mut Ui, trust: &Trust) {
    let (fg, bg) = trust_colors(trust.level);
    // Dessiné à la main comme `widgets::pill`, mais en gardant la réponse pour le survol.
    let galley = ui.painter().layout_no_wrap(
        trust.level.label().to_string(),
        egui::FontId::proportional(12.0),
        fg,
    );
    let size = galley.size() + egui::vec2(14.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), bg);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(theme::RADIUS),
        egui::Stroke::new(1.0, fg.gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    response.on_hover_text(trust_hover(trust));
}

/// Boîte de confirmation avant d'installer un monde de niveau bas.
fn confirm_modal(ctx: &egui::Context, panel: &mut RegistryPanel, worlds_dir: &Path) {
    let Some(name) = panel.confirm.clone() else {
        return;
    };
    let trust = panel
        .index
        .as_ref()
        .and_then(|i| i.get(PackageKind::Apworld, &name))
        .map(|p| p.trust.clone())
        .unwrap_or_default();
    let mut install = false;
    let mut cancel = false;
    // `Modal` grise le reste de la fenêtre et bloque les clics derrière la boîte.
    let modal = egui::Modal::new(egui::Id::unique("registre_confirmer")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(widgets::dim("MONDE DU REGISTRE").size(12.0));
        ui.label(
            RichText::new(format!("Installer « {name} » ?"))
                .size(20.0)
                .family(theme::bold_family())
                .color(theme::TEXT),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(widgets::dim("Niveau de confiance :"));
            trust_badge(ui, &trust);
        });
        ui.add_space(4.0);
        ui.label(RichText::new(trust.level.meaning()).color(theme::TEXT));
        ui.add_space(6.0);
        ui.label(widgets::dim(
            "Ce monde n'a pas été généré par la CI de Raftup. Une apworld est du code \
             Python qui s'exécute sur ce PC pendant la génération : ne l'installez que si \
             vous faites confiance à son auteur.",
        ));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            install = widgets::primary_button(ui, "Installer quand même", true);
            cancel = widgets::ghost_button(ui, "Annuler");
        });
    });
    // Échap ou clic à côté de la boîte : comme « Annuler ».
    if install {
        panel.confirm = None;
        panel.start_install(ctx, &name, worlds_dir.to_path_buf());
    } else if cancel || modal.should_close() {
        panel.confirm = None;
    }
}

/// Le client qui lit le registre : le vrai réseau, ou en debug le dossier de
/// `RAFTUP_DEV_REGISTRE`. `Box<dyn Fetcher>` : les deux types n'ont que le trait en commun.
fn fetcher() -> Box<dyn Fetcher> {
    if cfg!(debug_assertions) {
        if let Some(dir) = std::env::var_os("RAFTUP_DEV_REGISTRE") {
            return Box::new(LocalRegistry(PathBuf::from(dir)));
        }
    }
    Box::new(HttpFetcher::new())
}

/// Registre lu dans un dossier (debug) : une adresse donne le fichier nommé comme son
/// dernier segment (`…/registre/index.json` → `<dossier>/index.json`).
struct LocalRegistry(PathBuf);

impl Fetcher for LocalRegistry {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, DownloadError> {
        let name = url.rsplit('/').next().unwrap_or_default();
        let path = self.0.join(name);
        let file = std::fs::File::open(&path).map_err(|source| DownloadError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(Box::new(file))
    }
}

impl Default for RegistryPanel {
    fn default() -> Self {
        RegistryPanel::new()
    }
}

/// Bas de la Bibliothèque : les apworlds du registre (s'il y en a), puis la ligne d'état.
pub fn show(ui: &mut Ui, panel: &mut RegistryPanel, worlds_dir: Option<&Path>) {
    let ctx = ui.ctx().clone();
    let apworlds: Vec<_> = panel
        .index
        .as_ref()
        .map(|i| i.of_kind(PackageKind::Apworld).cloned().collect())
        .unwrap_or_default();
    if let (false, Some(dir)) = (apworlds.is_empty(), worlds_dir) {
        ui.add_space(14.0);
        let mut clicked = None;
        widgets::panel(ui, "Mondes du registre", |ui| {
            ui.label(widgets::dim(
                "Apworlds référencées par le registre signé de Raftup. « Installer » les \
                 télécharge chez leur auteur et vérifie leur empreinte avant de les poser.",
            ));
            ui.add_space(6.0);
            for package in &apworlds {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        let title = match &package.game {
                            Some(game) => format!("{game} · {}", package.name),
                            None => package.name.clone(),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(title).color(theme::TEXT).strong());
                            trust_badge(ui, &package.trust);
                        });
                        ui.label(
                            widgets::dim(format!(
                                "v{} · {} · licence {}",
                                package.version,
                                package.authors.join(", "),
                                package.license
                            ))
                            .size(12.0),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let busy = panel.install.as_ref();
                        if busy.is_some_and(|i| i.name == package.name) {
                            ui.spinner();
                        } else if panel.installed.contains(&package.name) {
                            ui.label(RichText::new("Installée").color(theme::GREEN));
                        } else if widgets::primary_button(ui, "Installer", busy.is_none()) {
                            clicked = Some(package.name.clone());
                        }
                    });
                });
            }
            match &panel.message {
                Some(Ok(text)) => {
                    ui.label(RichText::new(text).color(theme::GREEN).size(12.0));
                }
                Some(Err(text)) => {
                    ui.label(RichText::new(text).color(theme::AMBER).size(12.0));
                }
                None => {}
            }
        });
        if let Some(name) = clicked {
            panel.request_install(&ctx, &name, dir);
        }
        confirm_modal(&ctx, panel, dir);
    }
    if let Some(text) = panel.status_text() {
        ui.add_space(10.0);
        let label = ui.label(widgets::dim(text).size(12.0));
        if let Some(detail) = panel.hover_text() {
            label.on_hover_text(detail);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use launcher_core::registry::Package;
    use std::collections::BTreeMap;

    fn package(kind: PackageKind, name: &str) -> Package {
        Package {
            kind,
            name: name.to_string(),
            version: "1.0".to_string(),
            game: None,
            url: format!("https://github.com/a/b/releases/download/v1/{name}"),
            sha256: "0".repeat(64),
            size: 1,
            license: "MIT".to_string(),
            authors: vec!["Auteur".to_string()],
            trust: Default::default(),
        }
    }

    fn fetched(packages: Vec<Package>, failed: Vec<(String, String)>) -> RegistryFetch {
        let mut index = Index::new("raftup");
        index.packages = packages;
        RegistryFetch {
            index,
            files: BTreeMap::new(),
            failed,
        }
    }

    #[test]
    fn confirmation_seulement_pour_les_niveaux_bas() {
        assert!(needs_confirmation(TrustLevel::Unverified));
        assert!(needs_confirmation(TrustLevel::Valid));
        assert!(!needs_confirmation(TrustLevel::Generates));
        assert!(!needs_confirmation(TrustLevel::Fuzzed));
    }

    #[test]
    fn survol_du_badge() {
        let sans_detail = Trust::default();
        assert!(trust_hover(&sans_detail).starts_with("Non vérifié — "));
        let avec = Trust {
            level: TrustLevel::Fuzzed,
            detail: Some("Archipelago 0.6.8 · 5 graine(s)".into()),
        };
        let texte = trust_hover(&avec);
        assert!(texte.starts_with("Testé en profondeur — "));
        assert!(texte.ends_with("\n\nArchipelago 0.6.8 · 5 graine(s)"));
    }

    #[test]
    fn installer_un_monde_bas_demande_confirmation() {
        let ctx = egui::Context::default();
        let mondes = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::embedded().unwrap();
        let mut bas = package(PackageKind::Apworld, "bas");
        bas.trust.level = TrustLevel::Valid;
        let mut sur = package(PackageKind::Apworld, "sur");
        sur.trust.level = TrustLevel::Generates;
        let mut panel = RegistryPanel::new();
        panel.apply(&mut catalog, fetched(vec![bas, sur], Vec::new()), None);
        // Niveau bas : rien ne part, la boîte de confirmation attend.
        panel.request_install(&ctx, "bas", mondes.path());
        assert_eq!(panel.confirm.as_deref(), Some("bas"));
        assert!(panel.install.is_none());
        // Niveau « générable » : l'installation partirait directement (pas lancée ici :
        // elle téléchargerait pour de vrai). Un nom inconnu compte comme « non vérifié ».
        assert_eq!(panel.level_of("sur"), TrustLevel::Generates);
        assert!(!needs_confirmation(panel.level_of("sur")));
        assert_eq!(panel.level_of("absent"), TrustLevel::Unverified);
    }

    #[test]
    fn rien_a_dire_avant_la_lecture() {
        let panel = RegistryPanel::new();
        assert_eq!(panel.status_text(), None);
        assert!(!panel.is_from_registry("slime-rancher"));
    }

    #[test]
    fn echec_discret_sans_toucher_au_catalogue() {
        let mut catalog = Catalog::embedded().unwrap();
        let before = catalog.profiles.len();
        let mut panel = RegistryPanel::new();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Err("signature du registre refusée".to_string()))
            .unwrap();
        panel.phase = Phase::Loading(receiver);
        assert!(!panel.poll(&mut catalog, None));
        assert_eq!(catalog.profiles.len(), before);
        let text = panel.status_text().unwrap();
        assert!(text.contains("indisponible") && text.contains("catalogue embarqué seul"));
        assert_eq!(
            panel.hover_text().as_deref(),
            Some("signature du registre refusée")
        );
    }

    #[test]
    fn registre_lu_compte_les_paquets_et_les_apworlds_posees() {
        let mut catalog = Catalog::embedded().unwrap();
        let mondes = tempfile::tempdir().unwrap();
        std::fs::write(apworld_path(mondes.path(), "deja"), b"x").unwrap();
        let mut panel = RegistryPanel::new();
        let lu = fetched(
            vec![
                package(PackageKind::Apworld, "deja"),
                package(PackageKind::Apworld, "nouvelle"),
                package(PackageKind::Profile, "jeu-casse"),
            ],
            vec![("jeu-casse".to_string(), "404".to_string())],
        );
        // Aucun jeu ajouté : `jeu-casse` n'a pas pu être téléchargé.
        assert!(!panel.apply(&mut catalog, lu, Some(mondes.path())));
        assert!(matches!(panel.phase, Phase::Ready));
        assert_eq!(panel.installed, BTreeSet::from(["deja".to_string()]));
        assert_eq!(
            panel.status_text().unwrap(),
            "Registre signé : 3 paquet(s) vérifié(s), 1 écarté(s)"
        );
        assert!(!panel.is_from_registry("jeu-casse"));
        assert_eq!(panel.hover_text().as_deref(), Some("jeu-casse : 404"));
    }
}
