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
//! signature est vérifiée pareil, avec la vraie clé.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use launcher_core::download::{DownloadError, Fetcher};
use launcher_core::registry::{
    apply_registry, apworld_path, fetch_registry, install_apworld, ApworldInstall, Index,
    PackageKind, RegistryFetch, REGISTRY_INDEX_URL, REGISTRY_PUBLIC_KEY,
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
                REGISTRY_PUBLIC_KEY,
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
                        ui.label(RichText::new(title).color(theme::TEXT).strong());
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
            panel.start_install(&ctx, &name, dir.to_path_buf());
        }
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
