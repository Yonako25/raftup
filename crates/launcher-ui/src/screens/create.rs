//! Écran « Créer une partie », présenté comme le « Salon de préparation » de la maquette.
//!
//! L'hôte choisit son jeu et un préréglage, puis ajoute l'équipage (pseudo et jeu de
//! chacun) : Raftup écrit lui-même un YAML par joueur (ADR 0044). Des YAML écrits à la
//! main restent possibles dans les options avancées (ADR 0019 : par la boîte de
//! fichiers seulement) ; ils remplacent alors l'équipage.
//! `build_request` vérifie le tout et en fait une demande de génération et des
//! réglages d'hébergement (`docs/contrat-ui-generation.md`).
//!
//! Au clic sur « Générer », si le formulaire est bon, on vérifie que le générateur
//! est installé ; sinon on demande l'accord de l'hôte pour l'installer (étape 2,
//! `generator_setup`). S'il est prêt, la génération part dans un thread (étape 3,
//! `generation`). La seed générée est ensuite hébergée par le worker (étape 4,
//! `hosting`), puis l'écran bascule vers « Partie en cours ».
//!
//! Hébergement sur Rafthub (ADR 0052) : si le joueur est connecté à un hub qui l'annonce
//! (`hosting` dans `GET /v1/info`), le panneau « Hébergement » propose « Héberger sur mon
//! PC » / « Héberger sur Rafthub ». Pour Rafthub, la seed est générée ici de la même
//! façon, puis envoyée au hub (`hub_game::Upload`) au lieu d'être donnée au worker.

use std::path::PathBuf;

use egui::{RichText, Ui};
use launcher_core::crew::{self, CrewMember};
use launcher_core::generation::{
    installed_apworlds, load_native_worlds, rust_support_with_worlds, GenerateRequest,
};
use launcher_core::profile::StepKind;
use launcher_core::PartyHostOptions;

use super::hub_game::{self, HubAccess};
use super::library::{load_catalog, load_paths, AP_VERSION};
use super::prefs;
use super::presets::Preset;
use super::security::{self, PemFile, Security};
use super::{generation, generator_setup, hosting};
use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::{Shared, Worker};

/// Où tourne la partie (ADR 0052).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostMode {
    /// Sur ce PC (self-host) : toujours possible, sans compte.
    #[default]
    ThisPc,
    /// Sur le Rafthub où le joueur est connecté, s'il le propose.
    Rafthub,
}

/// Ce qu'il faut pour envoyer la seed au hub une fois générée, lu au clic.
struct RafthubTarget {
    access: HubAccess,
    /// Nom tapé par l'hôte (peut être vide : `hub_game::game_name` en choisit un).
    title: String,
    my_name: String,
    password: Option<String>,
}

/// Ce que [`background`] vient de voir démarrer.
pub enum Started {
    Nothing,
    /// Le serveur tourne sur ce PC.
    HereOnPc,
    /// Le hub a créé la partie. `Box` : la réponse est grosse à côté des autres cas.
    OnRafthub(Box<launcher_core::hub_client::CreatedGame>),
}

/// Avertissement obligatoire avant d'ajouter un apworld (contrat, « Avertissements »).
pub const APWORLD_WARNING: &str = "Un monde ajouté à la main (fichier .apworld) est du code qui \
     s'exécute sur ton PC, sans bac à sable. N'ajoute que ceux que tu juges sûrs.";

/// Boîte de dialogue à ouvrir après le dessin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Yaml,
    Apworld,
    Cert,
    Key,
}

/// Un jeu du catalogue qu'on peut choisir dans le salon.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GameChoice {
    /// Identifiant du profil (sert aussi à la couleur de la jaquette).
    id: String,
    /// Nom affiché.
    name: String,
    /// Nom du jeu côté Archipelago, écrit dans le YAML.
    ap_game: String,
    /// Version de l'apworld du profil, pour la ligne des versions (au survol).
    world_version: Option<String>,
}

/// Les jeux du catalogue (vide si le catalogue ne se lit pas).
fn game_choices() -> Vec<GameChoice> {
    match load_catalog() {
        Ok(catalog) => game_choices_from(&catalog),
        Err(_) => Vec::new(),
    }
}

/// Les jeux d'un catalogue donné, dans l'ordre du catalogue.
fn game_choices_from(catalog: &launcher_core::Catalog) -> Vec<GameChoice> {
    catalog
        .games()
        .map(|profile| GameChoice {
            id: profile.game.id.clone(),
            name: profile.game.name.clone(),
            ap_game: profile.game.ap_game.clone(),
            world_version: profile
                .steps
                .iter()
                .find(|step| step.kind == StepKind::Apworld)
                .map(|step| step.version.clone()),
        })
        .collect()
}

/// Un ami ajouté à l'équipage : son pseudo et l'identifiant du jeu choisi.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mate {
    name: String,
    game_id: String,
}

/// Ce que l'hôte a saisi.
pub struct CreateForm {
    /// Jeux proposés dans le salon (catalogue de Raftup).
    games: Vec<GameChoice>,
    /// Pseudo de l'hôte (son slot dans la partie).
    my_name: String,
    /// Jeu de l'hôte (identifiant du profil).
    my_game: Option<String>,
    /// Les amis de l'équipage, dans l'ordre d'ajout.
    mates: Vec<Mate>,
    /// Ligne « ajouter un joueur » : pseudo et jeu en cours de saisie.
    new_name: String,
    new_game: Option<String>,
    /// Refus de la dernière tentative d'ajout (pseudo vide, déjà pris…).
    crew_error: Option<String>,
    /// Fichiers YAML importés à la main, dans l'ordre d'ajout (sans doublon).
    /// S'il y en a, ils remplacent l'équipage.
    yamls: Vec<PathBuf>,
    /// Apworlds supplémentaires, dans l'ordre d'ajout (sans doublon).
    apworlds: Vec<PathBuf>,
    /// L'hôte a coché « J'ai compris » sous l'avertissement des apworlds.
    apworld_ack: bool,
    /// Apworlds installés par la Bibliothèque (profils du catalogue, hash vérifié) :
    /// ajoutés tout seuls à la génération, sans l'avertissement des apworlds choisis à la main.
    installed_apworlds: Vec<PathBuf>,
    pending_pick: Option<Pick>,
    /// Preset de génération (F06) : Rapide, Standard ou Long.
    preset: Preset,
    /// L'hôte veut essayer le moteur Rust (ADR 0027, sur option tant que l'équilibrage
    /// de progression n'est pas fait).
    rust_engine: bool,
    /// Équilibrage par temps de jeu (ADR 0030), seulement avec le moteur Rust ou export.
    /// Décoché par défaut.
    time_balance: bool,
    port: String,
    password: String,
    public_host: String,
    /// Adresse du webhook Discord (secrète : champ masqué, jamais écrite dans un journal).
    webhook: String,
    /// Chiffrement choisi (ADR 0043) : par Raftup sans fichier, par défaut.
    security: Security,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    /// Le générateur : absent, en cours d'installation, prêt… (étape 2).
    setup: generator_setup::Setup,
    /// La génération : en cours, finie ou échouée (étape 3).
    generation: generation::Generation,
    /// Réglages du serveur lus au clic sur « Générer » : ce sont eux qui servent à
    /// héberger la seed, même si le formulaire change pendant la génération.
    pending_host: Option<PartyHostOptions>,
    /// L'hébergement de la seed générée (étape 4).
    hosting: hosting::Hosting,
    /// Ouvrent de force un bloc replié à la prochaine image : le formulaire a été refusé
    /// à cause d'un de ses champs (port, adresse, certificat, webhook ; apworlds).
    open_advanced: bool,
    open_advanced_apworlds: bool,
    /// Ouvre de force les options avancées de « Ton monde » (YAML ou apworlds refusés).
    open_advanced_world: bool,
    /// Mode d'hébergement choisi (ADR 0052).
    host_mode: HostMode,
    /// Nom de la partie sur Rafthub (facultatif).
    game_title: String,
    /// Accès au hub pour héberger, recopié à chaque image ; `None` : pas de choix Rafthub.
    rafthub: Option<HubAccess>,
    /// Réglages de l'envoi au hub, lus au clic sur « Générer » (comme `pending_host`).
    pending_rafthub: Option<RafthubTarget>,
    /// L'envoi de la seed générée au hub.
    upload: hub_game::Upload,
}

impl Default for CreateForm {
    /// Mêmes valeurs par défaut que l'écran « Héberger une partie ».
    fn default() -> CreateForm {
        CreateForm {
            games: Vec::new(),
            my_name: String::new(),
            my_game: None,
            mates: Vec::new(),
            new_name: String::new(),
            new_game: None,
            crew_error: None,
            yamls: Vec::new(),
            apworlds: Vec::new(),
            apworld_ack: false,
            installed_apworlds: Vec::new(),
            pending_pick: None,
            preset: Preset::default(),
            rust_engine: false,
            time_balance: false,
            port: "38281".to_string(),
            password: String::new(),
            public_host: "localhost".to_string(),
            webhook: String::new(),
            security: Security::default(),
            cert: None,
            key: None,
            setup: generator_setup::Setup::default(),
            generation: generation::Generation::default(),
            pending_host: None,
            hosting: hosting::Hosting::default(),
            open_advanced: false,
            open_advanced_apworlds: false,
            open_advanced_world: false,
            host_mode: HostMode::default(),
            game_title: String::new(),
            rafthub: None,
            pending_rafthub: None,
            upload: hub_game::Upload::default(),
        }
    }
}

impl CreateForm {
    /// Relit les apworlds installés par la Bibliothèque (liste vide si les dossiers de
    /// Raftup sont introuvables : la génération dira alors ce qui manque).
    fn refresh_installed(&mut self) {
        self.installed_apworlds = match load_paths() {
            Ok(paths) => installed_apworlds(&paths.worlds_dir(AP_VERSION)),
            Err(_) => Vec::new(),
        };
    }

    /// Formulaire pré-rempli avec le webhook et le préréglage mémorisés.
    pub fn with_prefs(prefs: prefs::Prefs) -> CreateForm {
        let mut form = CreateForm {
            webhook: prefs.webhook,
            preset: prefs.preset,
            ..CreateForm::default()
        };
        form.refresh_installed();
        form.set_games(game_choices());
        form
    }

    /// Tests de l'interface (`ui_tests`) : formulaire par défaut avec les jeux de
    /// `catalog`, sans rien lire sur le disque (ni préférences, ni apworlds installés).
    #[cfg(test)]
    pub(crate) fn for_tests(catalog: &launcher_core::Catalog) -> CreateForm {
        let mut form = CreateForm::default();
        form.set_games(game_choices_from(catalog));
        form
    }

    /// Range les jeux proposés. Un seul jeu au catalogue : il est choisi d'office.
    fn set_games(&mut self, games: Vec<GameChoice>) {
        self.games = games;
        if let [only] = self.games.as_slice() {
            self.my_game = Some(only.id.clone());
            self.new_game = Some(only.id.clone());
        }
    }

    fn game(&self, id: &str) -> Option<&GameChoice> {
        self.games.iter().find(|g| g.id == id)
    }

    /// Nom affiché d'un jeu, ou son identifiant s'il n'est plus au catalogue.
    fn game_name(&self, id: &str) -> String {
        self.game(id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Clic sur « Ajouter » : vérifie le pseudo et ajoute l'ami à l'équipage.
    fn add_mate(&mut self) {
        let Some(game_id) = self.new_game.clone() else {
            self.crew_error = Some("Choisis le jeu de ce joueur.".into());
            return;
        };
        // On vérifie contre tout l'équipage, hôte compris.
        let mut others: Vec<CrewMember> = self
            .mates
            .iter()
            .map(|m| CrewMember {
                name: m.name.clone(),
                ap_game: String::new(),
            })
            .collect();
        if !self.my_name.trim().is_empty() {
            others.push(CrewMember {
                name: self.my_name.trim().to_string(),
                ap_game: String::new(),
            });
        }
        match crew::check_name(&self.new_name, &others) {
            Ok(name) => {
                self.mates.push(Mate { name, game_id });
                self.new_name.clear();
                self.crew_error = None;
            }
            Err(e) => self.crew_error = Some(e.to_string()),
        }
    }

    /// L'équipage complet (hôte en premier), vérifié, prêt à devenir des YAML.
    fn crew_members(&self) -> Result<Vec<CrewMember>, String> {
        let Some(my_game) = self.my_game.as_deref().and_then(|id| self.game(id)) else {
            return Err("Choisis ton jeu dans « Ton monde ».".into());
        };
        let mut members: Vec<CrewMember> = Vec::new();
        for mate in &self.mates {
            let ap_game = self
                .game(&mate.game_id)
                .map(|g| g.ap_game.clone())
                .ok_or_else(|| format!("Le jeu de {} n'est plus au catalogue.", mate.name))?;
            members.push(CrewMember {
                name: mate.name.clone(),
                ap_game,
            });
        }
        // Pseudo vide : « Indique ton pseudo. » plutôt que « Ton pseudo : Indique un pseudo. ».
        let my_name = crew::check_name(&self.my_name, &members).map_err(|e| match e {
            crew::NameError::Empty => "Indique ton pseudo.".to_string(),
            other => format!("Ton pseudo : {other}"),
        })?;
        // `insert(0, …)` : l'hôte passe en tête, il aura le slot 1.
        members.insert(
            0,
            CrewMember {
                name: my_name,
                ap_game: my_game.ap_game.clone(),
            },
        );
        Ok(members)
    }

    /// Vrai si la partie partira sur Rafthub : le mode est choisi et le hub le propose.
    pub fn on_rafthub(&self) -> bool {
        self.host_mode == HostMode::Rafthub && self.rafthub.is_some()
    }

    /// Tests de l'interface : le mode d'hébergement choisi.
    #[cfg(test)]
    pub(crate) fn host_mode(&self) -> HostMode {
        self.host_mode
    }

    /// Tests de l'interface : l'adresse publique du serveur de ce PC.
    #[cfg(test)]
    pub(crate) fn set_public_host_for_tests(&mut self, host: &str) {
        self.public_host = host.to_string();
    }

    /// Nombre de joueurs de la partie : les YAML importés, sinon l'équipage.
    fn player_count(&self) -> usize {
        if self.yamls.is_empty() {
            1 + self.mates.len()
        } else {
            self.yamls.len()
        }
    }
}

/// Fait avancer la génération et l'hébergement, que l'écran soit affiché ou non.
///
/// `App` l'appelle à CHAQUE image, avant de dessiner : si l'hôte va sur un autre écran
/// pendant la génération, la seed est quand même hébergée dès qu'elle est prête, et
/// une erreur de démarrage du serveur arrive dans le panneau d'hébergement (avec
/// « Réessayer ») au lieu de s'afficher, puis de se perdre, sur un autre écran.
///
/// Rend ce qui vient de démarrer : le serveur sur ce PC, ou la partie sur Rafthub.
pub fn background(
    ctx: &egui::Context,
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) -> Started {
    generation::poll(&mut form.generation);
    // Seed tout juste générée : on l'héberge avec les réglages du clic (une fois).
    if let Some(output) = form.generation.output() {
        if let Some(options) = form.pending_host.take() {
            // Une vieille erreur (d'un autre écran) passerait sinon pour un échec
            // du démarrage : `poll` prend la première erreur qu'il trouve.
            *error = None;
            form.hosting = hosting::start(worker, shared, output.clone(), options);
        } else if let Some(target) = form.pending_rafthub.take() {
            form.upload = upload(ctx, &target, output.clone());
        }
    }
    if hosting::poll(&mut form.hosting, shared, error) == hosting::Outcome::Started {
        // Le serveur tourne : l'écran repart propre pour la prochaine partie.
        form.generation = generation::Generation::Idle;
        return Started::HereOnPc;
    }
    if let Some(created) = hub_game::poll_upload(&mut form.upload) {
        form.generation = generation::Generation::Idle;
        return Started::OnRafthub(Box::new(created));
    }
    Started::Nothing
}

/// Envoie la seed au hub, avec le nom et le mot de passe du clic.
fn upload(
    ctx: &egui::Context,
    target: &RafthubTarget,
    output: launcher_core::generation::GenerateOutput,
) -> hub_game::Upload {
    let name = hub_game::game_name(&target.title, &target.my_name, &output.seed_name);
    hub_game::start_upload(ctx, &target.access, output, name, target.password.clone())
}

/// `rafthub` : accès au hub pour y héberger la partie, si le joueur y est connecté et
/// que le hub le propose (sinon, seul « Sur mon PC » existe).
pub fn show(
    ui: &mut Ui,
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
    rafthub: Option<&HubAccess>,
) -> Option<Page> {
    form.rafthub = rafthub.cloned();
    let mut goto = None;
    super::page(ui, |ui| {
        let count = form.player_count();
        let plural = if count > 1 { "s" } else { "" };
        super::header(
            ui,
            &format!("Tu organises · {count} joueur{plural}"),
            "Salon de préparation",
            |ui| {
                if widgets::ghost_button(ui, "Héberger un fichier de partie") {
                    goto = Some(Page::Hosted);
                }
            },
        );
        if let Some(message) = error.as_deref() {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }
        // Consentement, progression ou erreur de l'installation du générateur.
        generator_setup::show(ui, &mut form.setup);
        // Génération en cours, seed prête ou échec.
        if generation::show(ui, &mut form.generation) {
            on_generate(ui.ctx(), form, error);
        }
        super::estimate::show(ui, form.generation.output()); // Durée estimée (ADR 0030).
                                                             // Démarrage du serveur, ou échec avec « Réessayer ».
        if hosting::show(ui, &mut form.hosting) {
            retry_hosting(ui.ctx(), form, error, worker, shared);
        }
        // Envoi sur Rafthub, ou échec avec « Réessayer ».
        if hub_game::show_upload(ui, &mut form.upload) {
            retry_hosting(ui.ctx(), form, error, worker, shared);
        }

        // Deux colonnes comme la maquette : le salon à gauche, la session à droite.
        // Fenêtre étroite : tout l'un sous l'autre.
        if ui.available_width() >= super::WIDE {
            let gap = 16.0;
            let right = (ui.available_width() * 0.34).max(300.0);
            // egui ajoute son propre espacement entre deux éléments d'une rangée : on le
            // retire de la colonne de gauche, sinon celle de droite dépasse la marge.
            let spacing = ui.spacing().item_spacing.x;
            let left = ui.available_width() - right - gap - spacing;
            ui.horizontal_top(|ui| {
                column(ui, left, |ui| {
                    world_panel(ui, form);
                    ui.add_space(gap);
                    crew_panel(ui, form);
                });
                ui.add_space(gap);
                column(ui, right, |ui| {
                    hosting_panel(ui, form);
                    ui.add_space(gap);
                    launch_panel(ui, form, error);
                });
            });
        } else {
            world_panel(ui, form);
            ui.add_space(12.0);
            crew_panel(ui, form);
            ui.add_space(12.0);
            hosting_panel(ui, form);
            ui.add_space(12.0);
            launch_panel(ui, form, error);
        }
    });
    goto
}

/// Une colonne de largeur fixe, remplie de haut en bas.
fn column(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            add_contents(ui);
        },
    );
}

/// Jaquette en aplat coloré avec le nom du jeu (jamais de visuel officiel, SPEC § 7).
fn cover(ui: &mut Ui, size: egui::Vec2, game: Option<&GameChoice>) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(theme::RADIUS);
    match game {
        Some(game) => {
            painter.rect_filled(rect, radius, theme::cover_color(&game.id));
            let galley = painter.layout(
                game.name.clone(),
                theme::title_font((size.x * 0.13).clamp(9.0, 18.0)),
                theme::TEXT,
                size.x - 16.0,
            );
            painter.galley(
                rect.left_center() + egui::vec2(8.0, -galley.size().y / 2.0),
                galley,
                theme::TEXT,
            );
        }
        None => {
            painter.rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.0, theme::BORDER),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// Liste déroulante des jeux du catalogue. `id_salt` distingue les deux listes de l'écran.
fn game_combo(
    ui: &mut Ui,
    id_salt: &str,
    games: &[GameChoice],
    selected: &mut Option<String>,
    width: f32,
) {
    let current = selected
        .as_deref()
        .and_then(|id| games.iter().find(|g| g.id == id))
        .map(|g| g.name.clone())
        .unwrap_or_else(|| "Choisir un jeu…".to_string());
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(current)
        .width(width)
        .show_ui(ui, |ui| {
            for game in games {
                ui.selectable_value(selected, Some(game.id.clone()), &game.name);
            }
        });
}

/// Panneau « Ton monde » : pseudo, jeu, préréglage, et les options d'expert repliées.
fn world_panel(ui: &mut Ui, form: &mut CreateForm) {
    widgets::panel(ui, "Ton monde", |ui| {
        ui.horizontal_top(|ui| {
            let my_game = form
                .my_game
                .as_deref()
                .and_then(|id| form.game(id))
                .cloned();
            cover(ui, egui::vec2(110.0, 150.0), my_game.as_ref());
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let width = ui.available_width();
                ui.label(widgets::caps("Ton pseudo"));
                ui.add(
                    egui::TextEdit::singleline(&mut form.my_name)
                        .hint_text("Ton nom dans la partie (16 caractères au plus)")
                        .desired_width(width),
                );
                ui.add_space(8.0);
                ui.label(widgets::caps("Jeu"));
                game_combo(ui, "salon_my_game", &form.games, &mut form.my_game, width);
                let hint = if form.games.is_empty() {
                    "Aucun jeu au catalogue : importe des fichiers de réglages dans les options avancées."
                } else {
                    "Les jeux du catalogue de Raftup."
                };
                ui.label(widgets::dim(hint).size(12.0));
                ui.add_space(8.0);
                ui.label(widgets::caps("Réglages"));
                ui.horizontal(|ui| {
                    // Boutons collés, comme un sélecteur à segments de la maquette.
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for preset in Preset::ALL {
                        if widgets::segment(ui, preset.label(), form.preset == preset) {
                            form.preset = preset;
                        }
                    }
                });
                ui.label(widgets::dim(form.preset.description()).size(13.0));
            });
        });
        let mut open_now = form.open_advanced_world;
        widgets::advanced(
            ui,
            "create_world",
            "Options avancées…",
            &mut open_now,
            |ui| advanced_world(ui, form),
        );
        form.open_advanced_world = open_now;
    });
}

/// Options d'expert de « Ton monde » : moteur Rust, YAML importés, apworlds en plus.
fn advanced_world(ui: &mut Ui, form: &mut CreateForm) {
    ui.checkbox(
        &mut form.rust_engine,
        "Préparer sans le générateur classique (expérimental)",
    );
    ui.label(
        widgets::dim(
            "Raftup prépare la partie seul quand tous les joueurs jouent à APQuest ; sinon, le \
             générateur classique construit les mondes et Raftup place les objets.",
        )
        .size(13.0),
    );
    // `add_enabled` : la case est grisée (et ignorée) sans le moteur Rust.
    ui.add_enabled(
        form.rust_engine,
        egui::Checkbox::new(&mut form.time_balance, "Équilibrer par temps de jeu"),
    );
    ui.label(
        widgets::dim(
            "Avance les objets dont un joueur a besoin pour qu'il attende moins, d'après le \
             temps moyen d'un lieu de chaque jeu (estimé, pas encore mesuré).",
        )
        .size(13.0),
    );
    ui.add_space(8.0);
    labelled(
        ui,
        "Fichiers de réglages écrits à la main (.yaml, remplacent l'équipage)",
    );
    file_list(ui, &mut form.yamls, "Aucun fichier de réglages importé.");
    if widgets::ghost_button(ui, "Importer des fichiers de réglages…") {
        form.pending_pick = Some(Pick::Yaml);
    }
    if !form.installed_apworlds.is_empty() {
        let names: Vec<String> = form
            .installed_apworlds
            .iter()
            .filter_map(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        ui.add_space(6.0);
        ui.label(
            widgets::dim("Mondes pris automatiquement (installés par la Bibliothèque) :")
                .size(13.0),
        );
        ui.label(
            RichText::new(names.join(", "))
                .color(theme::TEXT)
                .size(13.0),
        );
    }
    // Apworlds choisis à la main : réservé à ceux qui savent ce qu'ils font.
    widgets::advanced(
        ui,
        "create_apworlds",
        "Mondes en plus (fichiers .apworld)",
        &mut form.open_advanced_apworlds,
        |ui| {
            ui.label(
                RichText::new(APWORLD_WARNING)
                    .color(theme::AMBER)
                    .size(13.0),
            );
            ui.checkbox(&mut form.apworld_ack, "J'ai compris");
            ui.add_space(6.0);
            file_list(ui, &mut form.apworlds, "Aucun monde en plus.");
            ui.add_space(6.0);
            // `add_enabled` grise le bouton tant que l'avertissement n'est pas coché.
            let add = ui.add_enabled(
                form.apworld_ack,
                egui::Button::new(RichText::new("Ajouter des mondes…").color(theme::TEXT)),
            );
            if add.clicked() {
                form.pending_pick = Some(Pick::Apworld);
            }
        },
    );
}

/// Carré coloré avec l'initiale du joueur, comme les avatars de la maquette.
fn avatar(ui: &mut Ui, name: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
    let initial: String = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS),
        theme::cover_color(name),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        theme::title_font(17.0),
        theme::TEXT,
    );
}

/// Une ligne de l'équipage. Rend vrai si l'on a cliqué « Retirer ».
fn crew_row(ui: &mut Ui, name: &str, detail: &str, removable: bool) -> bool {
    let mut removed = false;
    egui::Frame::new()
        .fill(theme::FIELD)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                avatar(ui, name);
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(name)
                            .color(theme::TEXT)
                            .strong()
                            .family(theme::bold_family()),
                    );
                    ui.label(widgets::dim(detail).size(13.0));
                });
                if removable {
                    // Mise en page de droite à gauche : le bouton se colle au bord droit.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        removed = widgets::ghost_button(ui, "Retirer");
                    });
                }
            });
        });
    removed
}

/// Panneau « Équipage » : l'hôte, ses amis, et la ligne pour en ajouter un.
fn crew_panel(ui: &mut Ui, form: &mut CreateForm) {
    ui.horizontal(|ui| {
        ui.label(widgets::title("Équipage", 20.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let count = form.player_count();
            let plural = if count > 1 { "s" } else { "" };
            ui.label(widgets::dim(format!("{count} joueur{plural}")));
        });
    });
    ui.add_space(6.0);
    if !form.yamls.is_empty() {
        widgets::panel(ui, "", |ui| {
            ui.label(
                RichText::new(format!(
                    "{} fichiers de réglages importés remplacent l'équipage (options avancées de « Ton monde »).",
                    form.yamls.len()
                ))
                .color(theme::AMBER),
            );
        });
        return;
    }
    // Ce que voit l'hôte pour sa propre ligne.
    let preset = form.preset.label();
    let my_name = match form.my_name.trim() {
        "" => "Toi".to_string(),
        name => format!("{name} (toi)"),
    };
    let my_game = form
        .my_game
        .as_deref()
        .map(|id| form.game_name(id))
        .unwrap_or_else(|| "Aucun jeu choisi".to_string());
    // `spacing_mut` : les lignes se touchent, comme une liste de la maquette.
    ui.spacing_mut().item_spacing.y = 0.0;
    crew_row(ui, &my_name, &format!("{my_game} · {preset}"), false);
    let mut remove = None;
    for (index, mate) in form.mates.iter().enumerate() {
        let detail = format!("{} · {preset}", form.game_name(&mate.game_id));
        if crew_row(ui, &mate.name, &detail, true) {
            remove = Some(index);
        }
    }
    if let Some(index) = remove {
        form.mates.remove(index);
    }
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.add_space(10.0);
    widgets::panel(ui, "", |ui| {
        ui.label(widgets::caps("Ajouter un joueur"));
        ui.horizontal(|ui| {
            let name_width = (ui.available_width() * 0.4).max(120.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut form.new_name)
                    .hint_text("Pseudo")
                    .desired_width(name_width),
            );
            // Entrée dans le champ : comme un clic sur « Ajouter ».
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let combo_width = (ui.available_width() - 110.0).max(120.0);
            game_combo(
                ui,
                "salon_new_game",
                &form.games,
                &mut form.new_game,
                combo_width,
            );
            if widgets::ghost_button(ui, "Ajouter") || enter {
                form.add_mate();
            }
        });
        if let Some(message) = &form.crew_error {
            ui.label(RichText::new(message).color(theme::AMBER).size(13.0));
        }
        ui.label(
            widgets::dim("Chaque joueur reçoit son lien d'invitation une fois la partie hébergée.")
                .size(12.0),
        );
    });
}

/// Versions figées pour toute la partie, en une ligne discrète : « Générateur 0.6.8 ·
/// Raftup 0.1.0 ». Le détail (version du monde de chaque jeu) est au survol.
fn versions_line(ui: &mut Ui, form: &CreateForm) {
    let mut details = vec![
        format!("Générateur classique {AP_VERSION}"),
        format!("Raftup {}", env!("CARGO_PKG_VERSION")),
    ];
    // Un monde par jeu de l'équipage, sans doublon, dans l'ordre de l'équipage.
    let mut ids: Vec<&str> = form.my_game.as_deref().into_iter().collect();
    for mate in &form.mates {
        if !ids.contains(&mate.game_id.as_str()) {
            ids.push(&mate.game_id);
        }
    }
    for id in ids {
        if let Some(game) = form.game(id) {
            let version = game.world_version.as_deref().unwrap_or("—");
            details.push(format!("Monde {} {version}", game.name));
        }
    }
    ui.label(
        widgets::dim(format!(
            "Générateur {AP_VERSION} · Raftup {}",
            env!("CARGO_PKG_VERSION")
        ))
        .size(12.0),
    )
    .on_hover_text(format!(
        "Versions figées pour toute l'équipe :\n{}",
        details.join("\n")
    ));
}

/// « Tes amis se connecteront à exemple.fr:38281. », ou un avertissement si l'adresse ne
/// marche que sur ce PC. Partagé avec « Héberger un fichier ».
pub fn address_line(ui: &mut Ui, port: &str, public_host: &str) {
    if format::is_local_address(public_host) || public_host.trim().is_empty() {
        ui.label(
            RichText::new(
                "Adresse locale : seuls les joueurs de ce PC pourront se connecter. \
                 Indique ton adresse publique dans les options avancées.",
            )
            .color(theme::AMBER)
            .size(12.0),
        );
    } else {
        ui.label(widgets::dim(format::server_summary(port, public_host)).size(12.0));
    }
}

/// Panneau à petit titre en capitales (« HÉBERGEMENT »), comme la colonne de droite
/// de la maquette.
fn caps_panel(ui: &mut Ui, heading: &str, add_contents: impl FnOnce(&mut Ui)) {
    widgets::panel(ui, "", |ui| {
        ui.label(widgets::caps(heading));
        ui.add_space(8.0);
        add_contents(ui);
    });
}

/// Panneau « Hébergement » : choix du mode (si Rafthub le propose), mot de passe,
/// résumé du serveur, options d'expert repliées.
fn hosting_panel(ui: &mut Ui, form: &mut CreateForm) {
    caps_panel(ui, "Hébergement", |ui| {
        if let Some(hub) = form.rafthub.as_ref().map(|a| a.host().to_string()) {
            // Sélecteur à segments sur une ligne (libellés courts pour tenir dans la colonne).
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let here = form.host_mode == HostMode::ThisPc;
                if widgets::segment(ui, "Sur mon PC", here) {
                    form.host_mode = HostMode::ThisPc;
                }
                if widgets::segment(ui, "Sur Rafthub", !here) {
                    form.host_mode = HostMode::Rafthub;
                }
            });
            ui.add_space(6.0);
            if form.host_mode == HostMode::Rafthub {
                rafthub_panel(ui, form, &hub);
                return;
            }
        }
        ui.label(
            widgets::dim("La partie tourne sur ton PC : il doit rester allumé pendant la partie.")
                .size(13.0),
        );
        labelled(ui, "Mot de passe de la salle (facultatif)");
        ui.add(
            egui::TextEdit::singleline(&mut form.password)
                .password(true)
                .desired_width(f32::INFINITY),
        );
        address_line(ui, &form.port, &form.public_host);
        // `open_advanced` est copié à part : `form` est déjà emprunté par la fermeture.
        let mut open_now = form.open_advanced;
        widgets::advanced(
            ui,
            "create_server",
            "Options avancées",
            &mut open_now,
            |ui| advanced_server(ui, form),
        );
        form.open_advanced = open_now;
    });
}

/// Réglages d'une partie hébergée par Rafthub : nom et mot de passe, sans port ni
/// certificat (le hub les choisit).
fn rafthub_panel(ui: &mut Ui, form: &mut CreateForm, hub: &str) {
    ui.label(
        widgets::dim(format!(
            "La partie tourne sur Rafthub ({hub}) : elle continue quand ton PC est éteint."
        ))
        .size(13.0),
    );
    ui.add_space(6.0);
    labelled(ui, "Nom de la partie (facultatif)");
    ui.add(
        egui::TextEdit::singleline(&mut form.game_title)
            .hint_text("Partie de …")
            .char_limit(64)
            .desired_width(f32::INFINITY),
    );
    labelled(ui, "Mot de passe de la salle (facultatif)");
    ui.add(
        egui::TextEdit::singleline(&mut form.password)
            .password(true)
            .desired_width(f32::INFINITY),
    );
}

/// Panneau « Lancer la partie » : ce qui manque, puis « Générer et héberger ».
fn launch_panel(ui: &mut Ui, form: &mut CreateForm, error: &mut Option<String>) {
    caps_panel(ui, "Lancer la partie", |ui| {
        // Le même contrôle que le clic : on annonce ici ce qui serait refusé.
        let (ready, status) = match check_form(form) {
            Ok(_) if !form.yamls.is_empty() => (
                true,
                format!(
                    "{} fichiers de réglages importés, prêts à générer.",
                    form.yamls.len()
                ),
            ),
            Ok(_) => {
                let n = form.player_count();
                let plural = if n > 1 { "s" } else { "" };
                (
                    true,
                    format!("{n} joueur{plural} dans l'équipage. Tu peux en ajouter, ou lancer maintenant."),
                )
            }
            Err(message) => (false, message),
        };
        let color = if ready { theme::TEXT } else { theme::AMBER };
        ui.label(RichText::new(status).color(color));
        ui.add_space(10.0);
        // Grisé pendant l'installation du générateur et pendant la génération.
        let enabled = !form.setup.is_busy()
            && !form.generation.is_busy()
            && !form.hosting.is_busy()
            && !form.upload.is_busy();
        let button = egui::Button::new(
            RichText::new("Générer et héberger")
                .font(theme::title_font(17.0))
                .color(egui::Color32::from_rgb(0x08, 0x12, 0x22)),
        )
        .fill(theme::ACCENT)
        .min_size(egui::vec2(ui.available_width(), 46.0));
        if ui.add_enabled(enabled, button).clicked() {
            on_generate(ui.ctx(), form, error);
        }
        ui.add_space(8.0);
        versions_line(ui, form);
    });
}

/// Écrit un YAML par joueur de l'équipage dans `generations/salon/` (ADR 0044).
fn write_crew(form: &CreateForm) -> Result<Vec<PathBuf>, String> {
    let members = form.crew_members()?;
    let dir = load_paths()?.generations_dir().join("salon");
    crew::write_yamls(&dir, &members).map_err(|e| {
        format!(
            "Impossible d'écrire les fichiers de l'équipage dans {} : {e}",
            dir.display()
        )
    })
}

/// Libellé au-dessus d'un champ.
fn labelled(ui: &mut Ui, label: &str) {
    ui.add_space(4.0);
    ui.label(widgets::dim(label).size(13.0));
}

/// Liste de fichiers avec un bouton « Retirer » par ligne.
fn file_list(ui: &mut Ui, files: &mut Vec<PathBuf>, empty: &str) {
    if files.is_empty() {
        ui.label(widgets::dim(empty).size(13.0));
        return;
    }
    // On note l'index à retirer, puis on retire APRÈS la boucle : on ne peut pas
    // modifier un `Vec` pendant qu'on le parcourt (le compilateur le refuse).
    let mut remove = None;
    for (index, path) in files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(file_name(path))
                    .strong()
                    .family(theme::bold_family()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, "Retirer") {
                    remove = Some(index);
                }
            });
        })
        .response
        .on_hover_text(path.display().to_string());
    }
    if let Some(index) = remove {
        files.remove(index);
    }
}

/// Nom du fichier seul, pour l'affichage.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Ligne « chemin en lecture seule + Parcourir… ». Renvoie vrai si on a cliqué.
fn file_row(ui: &mut Ui, path: &Option<PathBuf>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let mut shown = path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        ui.add(
            egui::TextEdit::singleline(&mut shown)
                .hint_text("Aucun fichier choisi")
                .interactive(false)
                .desired_width((ui.available_width() - 120.0).max(80.0)),
        );
        clicked = widgets::ghost_button(ui, "Parcourir…");
    });
    clicked
}

/// Contenu du bloc « Options avancées » du serveur : port, adresse publique, webhook,
/// chiffrement et certificats PEM.
fn advanced_server(ui: &mut Ui, form: &mut CreateForm) {
    labelled(ui, "Port");
    ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(120.0));

    labelled(ui, "Adresse publique (nom de domaine ou IP)");
    ui.add(egui::TextEdit::singleline(&mut form.public_host).desired_width(f32::INFINITY));

    labelled(ui, "Webhook Discord (facultatif)");
    ui.add(
        egui::TextEdit::singleline(&mut form.webhook)
            .password(true)
            .desired_width(f32::INFINITY),
    );

    labelled(ui, "Chiffrement");
    // Copies des chemins : `form.security` est modifié pendant le dessin des cartes.
    let (cert, key) = (form.cert.clone(), form.key.clone());
    match security::choice(ui, &mut form.security, &cert, &key, file_row) {
        Some(PemFile::Cert) => form.pending_pick = Some(Pick::Cert),
        Some(PemFile::Key) => form.pending_pick = Some(Pick::Key),
        None => {}
    }
}

/// Le bloc replié qui contient le champ refusé par `build_request`, à ouvrir pour que
/// l'hôte voie quoi corriger. Suit l'ordre des vérifications de `build_request`.
#[derive(Debug, PartialEq, Eq)]
enum Refused {
    /// Champ toujours visible (les YAML) : rien à ouvrir.
    Visible,
    Apworlds,
    Server,
}

fn refused_block(form: &CreateForm) -> Refused {
    if form.yamls.is_empty() && form.crew_members().is_err() {
        Refused::Visible
    } else if !form.apworlds.is_empty() && !form.apworld_ack {
        Refused::Apworlds
    } else {
        // Toutes les vérifications suivantes portent sur les options avancées du serveur.
        Refused::Server
    }
}

/// Ouvre le bloc du champ refusé.
fn open_refused_block(form: &mut CreateForm) {
    match refused_block(form) {
        Refused::Visible => {}
        Refused::Apworlds => {
            // Le bloc des apworlds est rangé dans les options avancées de « Ton monde ».
            form.open_advanced_world = true;
            form.open_advanced_apworlds = true;
        }
        Refused::Server => form.open_advanced = true,
    }
}

/// Clic sur « Générer la partie » (ou « Réessayer ») : vérifie le formulaire, puis
/// le générateur, puis lance la génération.
fn on_generate(ctx: &egui::Context, form: &mut CreateForm, error: &mut Option<String>) {
    // Relu à chaque clic : un jeu a pu être installé depuis l'ouverture de l'écran.
    form.refresh_installed();
    let built = if form.on_rafthub() {
        build_rafthub(form).map(|(request, target)| (request, Pending::Rafthub(target)))
    } else {
        build_request(form).map(|(request, host)| (request, Pending::Local(host)))
    };
    match built {
        Ok((mut request, pending)) => {
            if request.player_files.is_empty() {
                match write_crew(form) {
                    Ok(files) => request.player_files = files,
                    Err(message) => {
                        form.generation = generation::Generation::Idle;
                        *error = Some(message);
                        return;
                    }
                }
            }
            *error = None;
            // Mémorise le webhook (ou l'efface s'il est vide) et le préréglage. Le webhook
            // est un réglage du serveur de ce PC : en mode Rafthub, on n'y touche pas.
            if let Pending::Local(host) = &pending {
                prefs::remember(host.webhook.as_ref(), Some(form.preset));
            }
            form.hosting = hosting::Hosting::Idle;
            form.upload = hub_game::Upload::Idle;
            let (engine, note) = choose_engine(
                form.rust_engine,
                &request.player_files,
                request.meta_yaml.as_deref(),
            );
            // Sans le moteur Rust, la case est grisée : on ne la suit pas.
            let time_balance = form.rust_engine && form.time_balance;
            if engine == generation::Engine::Rust {
                // Pas de Python : rien à installer.
                form.setup = generator_setup::Setup::Ready;
                keep_pending(form, pending);
                form.generation = generation::start(ctx, request, engine, note, time_balance);
                return;
            }
            // Formulaire bon : on regarde ensuite si le générateur est là.
            // S'il manque, le panneau de consentement s'affiche en haut.
            form.setup = generator_setup::check();
            form.generation = match form.setup {
                generator_setup::Setup::Ready => {
                    // Gardés pour héberger la seed à la fin de la génération.
                    keep_pending(form, pending);
                    generation::start(ctx, request, engine, note, time_balance)
                }
                _ => generation::Generation::Idle,
            };
        }
        Err(message) => {
            form.generation = generation::Generation::Idle;
            open_refused_block(form);
            *error = Some(message);
        }
    }
}

/// Où héberger la seed une fois générée, lu au clic.
enum Pending {
    Local(PartyHostOptions),
    Rafthub(RafthubTarget),
}

/// Range les réglages du clic pour la fin de la génération (un seul des deux).
fn keep_pending(form: &mut CreateForm, pending: Pending) {
    match pending {
        Pending::Local(host) => {
            form.pending_host = Some(host);
            form.pending_rafthub = None;
        }
        Pending::Rafthub(target) => {
            form.pending_rafthub = Some(target);
            form.pending_host = None;
        }
    }
}

/// Moteur de la génération, quand l'hôte a coché « Moteur Rust » : le moteur Rust seul
/// s'il comprend tous les fichiers joueurs (aucun Python) ; sinon le moteur « export »
/// (ADR 0028 : Archipelago construit les mondes, Raftup place les objets, et repli sur
/// Archipelago seul si un monde ne s'y prête pas), avec la raison en tête du journal.
/// Case décochée : Archipelago seul.
fn choose_engine(
    wanted: bool,
    player_files: &[PathBuf],
    meta: Option<&str>,
) -> (generation::Engine, Option<String>) {
    if !wanted {
        return (generation::Engine::Python, None);
    }
    // Mondes natifs installés (ADR 0041) : un monde illisible est seulement écarté.
    let natives = match load_paths() {
        Ok(paths) => load_native_worlds(&paths.native_worlds_dir()),
        Err(_) => Default::default(),
    };
    for (path, reason) in &natives.rejected {
        tracing::warn!("monde natif écarté : {} ({reason})", path.display());
    }
    match rust_support_with_worlds(player_files, meta, &natives.worlds) {
        Ok(()) => (generation::Engine::Rust, None),
        Err(reason) => (
            generation::Engine::Export,
            Some(format!(
                "Raftup ne peut pas tout préparer seul ({reason}) : le générateur classique construit les mondes, Raftup place les objets."
            )),
        ),
    }
}

/// Ce que la macro de développement (`devscript.rs`) a besoin de savoir.
#[cfg(debug_assertions)]
pub struct DevState {
    pub generation_failed: bool,
    pub hosting_failed: bool,
}

/// Macro : remplit le formulaire (YAML donnés, en clair, port donné). Avec
/// `RAFTUP_DEV_RUST=1`, la case « Moteur Rust » est cochée ; avec `RAFTUP_DEV_TEMPS=1`,
/// « Équilibrer par temps de jeu » aussi.
#[cfg(debug_assertions)]
pub fn dev_prefill(form: &mut CreateForm, yamls: Vec<PathBuf>, port: u16) {
    form.yamls = yamls;
    form.security = Security::Plain;
    form.port = port.to_string();
    form.rust_engine = std::env::var("RAFTUP_DEV_RUST").is_ok_and(|v| v == "1");
    form.time_balance = std::env::var("RAFTUP_DEV_TEMPS").is_ok_and(|v| v == "1");
}

/// Macro : un équipage de démonstration (l'hôte et deux amis, sur le premier jeu du
/// catalogue), pour la capture du salon de préparation.
#[cfg(debug_assertions)]
pub fn dev_crew(form: &mut CreateForm) {
    let Some(first) = form.games.first().map(|g| g.id.clone()) else {
        return;
    };
    form.my_name = "Marvin".into();
    form.my_game = Some(first.clone());
    form.new_game = Some(first);
    for name in ["Léa", "Tom"] {
        form.new_name = name.into();
        form.add_mate();
    }
}

/// Macro : comme un clic sur « Générer la partie ».
#[cfg(debug_assertions)]
pub fn dev_generate(ctx: &egui::Context, form: &mut CreateForm, error: &mut Option<String>) {
    on_generate(ctx, form, error);
}

#[cfg(debug_assertions)]
pub fn dev_state(form: &CreateForm) -> DevState {
    DevState {
        generation_failed: matches!(form.generation, generation::Generation::Failed { .. }),
        hosting_failed: form.hosting.failed_output().is_some(),
    }
}

/// Macro : change le port, puis comme un clic sur « Réessayer ».
#[cfg(debug_assertions)]
pub fn dev_retry(
    ctx: &egui::Context,
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    port: u16,
) {
    form.port = port.to_string();
    let shared = worker.lock();
    retry_hosting(ctx, form, error, worker, &shared);
}

/// « Réessayer » après un démarrage (ou un envoi) raté : relit les réglages dans le
/// formulaire (l'hôte a pu changer le port, ou passer de Rafthub à son PC) et relance
/// avec la MÊME seed.
fn retry_hosting(
    ctx: &egui::Context,
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) {
    let Some(output) = form
        .hosting
        .failed_output()
        .or(form.upload.failed_output())
        .cloned()
    else {
        return;
    };
    if form.on_rafthub() {
        match build_rafthub(form) {
            Ok((_, target)) => {
                *error = None;
                form.hosting = hosting::Hosting::Idle;
                form.upload = upload(ctx, &target, output);
            }
            Err(message) => *error = Some(message),
        }
        return;
    }
    form.upload = hub_game::Upload::Idle;
    match build_request(form) {
        Ok((_, host)) => {
            *error = None;
            form.hosting = hosting::start(worker, shared, output, host);
        }
        // Formulaire devenu invalide : message en haut, la seed reste gardée.
        Err(message) => {
            open_refused_block(form);
            *error = Some(message);
        }
    }
}

/// Ouvre la boîte de dialogue demandée pendant le dessin.
///
/// Appelée par `App` après avoir relâché le verrou de l'état partagé : la boîte
/// bloque le fil de l'interface tant qu'elle est ouverte.
pub fn open_pending_dialog(form: &mut CreateForm) {
    let Some(pick) = form.pending_pick.take() else {
        return;
    };
    match pick {
        Pick::Yaml => {
            let picked = rfd::FileDialog::new()
                .set_title("Choisir les fichiers de réglages des joueurs (.yaml)")
                .add_filter("Réglages de joueur", &["yaml", "yml"])
                .pick_files();
            add_files(&mut form.yamls, picked.unwrap_or_default());
        }
        Pick::Apworld => {
            let picked = rfd::FileDialog::new()
                .set_title("Choisir des mondes (.apworld)")
                .add_filter("Monde de jeu", &["apworld"])
                .pick_files();
            add_files(&mut form.apworlds, picked.unwrap_or_default());
        }
        Pick::Cert => {
            form.cert = pick_pem("Certificat PEM", &["pem", "crt"]).or(form.cert.take());
        }
        Pick::Key => {
            form.key = pick_pem("Clé privée PEM", &["pem", "key"]).or(form.key.take());
        }
    }
}

fn pick_pem(name: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(name, extensions)
        .add_filter("Tous les fichiers", &["*"])
        .pick_file()
}

/// Ajoute des fichiers à la fin de la liste, sans doublon, dans l'ordre reçu.
fn add_files(list: &mut Vec<PathBuf>, picked: Vec<PathBuf>) {
    for path in picked {
        if !list.contains(&path) {
            list.push(path);
        }
    }
}

/// Vérifie le formulaire et en fait la demande de génération et les réglages
/// d'hébergement, ou un message en français (contrat, « Messages d'erreur »).
///
/// Tout est vérifié AVANT de générer : une génération dure des minutes, on ne veut
/// pas la perdre sur un port mal tapé.
/// Apworlds choisis à la main, puis ceux de la Bibliothèque. Un apworld installé dont le
/// nom de fichier est déjà choisi à la main est sauté : deux fichiers de même nom
/// s'écraseraient dans `custom_worlds`, et le choix de l'hôte passe avant.
fn all_apworlds(form: &CreateForm) -> Vec<PathBuf> {
    let mut all = form.apworlds.clone();
    for installed in &form.installed_apworlds {
        let same_name = all.iter().any(|p| p.file_name() == installed.file_name());
        if !same_name {
            all.push(installed.clone());
        }
    }
    all
}

/// La demande de génération seule (équipage ou YAML, apworlds, préréglage), commune
/// aux deux modes d'hébergement.
fn build_generation(form: &CreateForm) -> Result<GenerateRequest, String> {
    // Sans YAML importés, c'est l'équipage qui fera les joueurs : on le vérifie ici,
    // ses YAML ne sont écrits qu'au moment de générer (`on_generate`).
    if form.yamls.is_empty() {
        form.crew_members()?;
    }
    if !form.apworlds.is_empty() && !form.apworld_ack {
        return Err(format!("Coche « J'ai compris » : {APWORLD_WARNING}"));
    }
    // `..GenerateRequest::default()` : les autres champs (seed, spoiler, délai)
    // gardent leur valeur par défaut, comme un constructeur avec valeurs par défaut.
    let mut request = GenerateRequest {
        // Vide sans YAML importés : `on_generate` y met ceux de l'équipage.
        player_files: form.yamls.clone(),
        apworlds: all_apworlds(form),
        ..GenerateRequest::default()
    };
    // Le preset ne règle que des champs génériques de la demande (F06, ADR 0023).
    form.preset.apply(&mut request);
    Ok(request)
}

/// Comme [`build_request`], pour une partie hébergée par Rafthub : ni port, ni adresse,
/// ni certificat à vérifier (le hub les choisit).
fn build_rafthub(form: &CreateForm) -> Result<(GenerateRequest, RafthubTarget), String> {
    let request = build_generation(form)?;
    let access = form
        .rafthub
        .clone()
        .ok_or("Connecte-toi à Rafthub (écran « Amis ») pour y héberger la partie.")?;
    let target = RafthubTarget {
        access,
        title: form.game_title.clone(),
        my_name: form.my_name.clone(),
        password: Some(form.password.clone()).filter(|p| !p.is_empty()),
    };
    Ok((request, target))
}

/// Ce que le clic refuserait, selon le mode choisi (texte du panneau « Lancer »).
/// `pub(crate)` : les tests de l'interface s'en servent.
pub(crate) fn check_form(form: &CreateForm) -> Result<GenerateRequest, String> {
    if form.on_rafthub() {
        build_rafthub(form).map(|(request, _)| request)
    } else {
        build_request(form).map(|(request, _)| request)
    }
}

/// `pub(crate)` : les tests de l'interface comparent ce refus au texte affiché.
pub(crate) fn build_request(
    form: &CreateForm,
) -> Result<(GenerateRequest, PartyHostOptions), String> {
    let request = build_generation(form)?;
    let port = format::parse_port(&form.port)?;
    let public_host = form.public_host.trim().to_string();
    if public_host.is_empty() {
        return Err("Indique l'adresse que les autres joueurs utiliseront.".into());
    }
    let encryption = security::encryption(form.security, &form.cert, &form.key)?;
    let password = Some(form.password.clone()).filter(|p| !p.is_empty());
    let webhook = format::parse_webhook(&form.webhook)?;
    let host = PartyHostOptions {
        port,
        password,
        encryption,
        public_host,
        disable_save: false,
        webhook,
    };
    Ok((request, host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::Encryption;

    #[test]
    fn equilibrage_par_temps_decoche_par_defaut() {
        let form = CreateForm::default();
        assert!(!form.time_balance);
        assert!(!form.rust_engine);
    }

    #[test]
    fn moteur_rust_seulement_si_demande_et_compris() {
        let tmp = tempfile::tempdir().unwrap();
        let apquest = tmp.path().join("a.yaml");
        std::fs::write(
            &apquest,
            "name: A
game: APQuest
",
        )
        .unwrap();
        let other = tmp.path().join("b.yaml");
        std::fs::write(
            &other,
            "name: B
game: Slime Rancher
",
        )
        .unwrap();

        assert_eq!(
            choose_engine(false, std::slice::from_ref(&apquest), None),
            (generation::Engine::Python, None)
        );
        assert_eq!(
            choose_engine(true, std::slice::from_ref(&apquest), None),
            (generation::Engine::Rust, None)
        );
        // Préréglage « Long » (équilibrage 20, accès complet) : le moteur Rust le comprend.
        let long = super::super::presets::meta_yaml(Preset::Long);
        assert_eq!(
            choose_engine(true, std::slice::from_ref(&apquest), long.as_deref()),
            (generation::Engine::Rust, None)
        );
        // Préréglage « Rapide » (accès minimal) : pas pour le moteur Rust seul, export.
        let rapide = super::super::presets::meta_yaml(Preset::Rapide);
        let (engine, note) = choose_engine(true, std::slice::from_ref(&apquest), rapide.as_deref());
        assert_eq!(engine, generation::Engine::Export);
        assert!(note.unwrap().contains("accessibility"));
        // Un jeu non porté en Rust : export (Archipelago construit, Raftup place).
        let (engine, note) = choose_engine(true, &[apquest, other], None);
        assert_eq!(engine, generation::Engine::Export);
        assert!(note.unwrap().contains("Slime Rancher"));
    }

    /// Un formulaire déjà rempli et valide (en clair, un joueur).
    fn valid() -> CreateForm {
        CreateForm {
            yamls: vec![PathBuf::from("alice.yaml")],
            security: Security::Plain,
            ..CreateForm::default()
        }
    }

    #[test]
    fn formulaire_valide() {
        let mut form = valid();
        form.password = "secret".into();
        form.public_host = "  exemple.fr ".into();
        let (request, host) = build_request(&form).unwrap();
        assert_eq!(request.player_files, vec![PathBuf::from("alice.yaml")]);
        assert!(request.apworlds.is_empty());
        assert_eq!(host.port, 38281);
        assert_eq!(host.password.as_deref(), Some("secret"));
        assert_eq!(host.public_host, "exemple.fr");
        assert_eq!(host.encryption, Encryption::Plain);
        assert!(!host.disable_save);
    }

    #[test]
    fn webhook_vide_ou_valide_passe_dans_les_reglages() {
        let mut form = valid();
        assert_eq!(build_request(&form).unwrap().1.webhook, None);
        form.webhook = " https://discord.com/api/webhooks/1/abc ".into();
        let (_, host) = build_request(&form).unwrap();
        assert_eq!(
            host.webhook.unwrap().expose(),
            "https://discord.com/api/webhooks/1/abc"
        );
    }

    #[test]
    fn webhook_refuse_sans_recopier_l_adresse() {
        let mut form = valid();
        form.webhook = "https://evil.org/api/webhooks/SECRETTOKEN".into();
        let message = build_request(&form).unwrap_err();
        assert!(!message.contains("SECRETTOKEN"));
    }

    #[test]
    fn le_formulaire_est_prerempli_par_les_reglages_memorises() {
        let form = CreateForm::with_prefs(prefs::Prefs {
            webhook: "https://discord.com/api/webhooks/1/abc".into(),
            preset: Preset::Long,
        });
        assert_eq!(form.webhook, "https://discord.com/api/webhooks/1/abc");
        assert_eq!(form.preset, Preset::Long);
    }

    #[test]
    fn preset_applique_au_delai() {
        let mut form = valid();
        form.preset = Preset::Rapide;
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(request.timeout, Preset::Rapide.timeout());
    }

    /// Formulaire avec deux jeux au catalogue, sans YAML importé.
    fn salon() -> CreateForm {
        let game = |id: &str, ap: &str| GameChoice {
            id: id.into(),
            name: ap.into(),
            ap_game: ap.into(),
            world_version: Some("1.0".into()),
        };
        CreateForm {
            games: vec![game("sr", "Slime Rancher"), game("hk", "Hollow Knight")],
            security: Security::Plain,
            ..CreateForm::default()
        }
    }

    #[test]
    fn equipage_hote_en_tete_puis_amis() {
        let mut form = salon();
        form.my_game = Some("sr".into());
        form.my_name = " Marvin ".into();
        form.new_game = Some("hk".into());
        form.new_name = "Léa".into();
        form.add_mate();
        assert_eq!(form.crew_error, None);
        assert!(form.new_name.is_empty());
        let crew = form.crew_members().unwrap();
        assert_eq!(
            crew,
            vec![
                CrewMember {
                    name: "Marvin".into(),
                    ap_game: "Slime Rancher".into()
                },
                CrewMember {
                    name: "Léa".into(),
                    ap_game: "Hollow Knight".into()
                },
            ]
        );
        // Sans YAML importé, la demande attend les YAML de l'équipage.
        let (request, _) = build_request(&form).unwrap();
        assert!(request.player_files.is_empty());
        assert_eq!(form.player_count(), 2);
    }

    #[test]
    fn equipage_refuse_doublon_et_jeu_manquant() {
        let mut form = salon();
        form.my_name = "Tom".into();
        form.new_name = "tom".into();
        form.add_mate();
        assert_eq!(
            form.crew_error.as_deref(),
            Some("Choisis le jeu de ce joueur.")
        );
        form.new_game = Some("sr".into());
        form.add_mate();
        assert_eq!(
            form.crew_error.as_deref(),
            Some("« tom » est déjà dans l'équipage.")
        );
        assert!(form.mates.is_empty());
        // Pseudo de l'hôte vide : refus clair, jeu choisi.
        form.my_game = Some("sr".into());
        form.my_name.clear();
        assert_eq!(build_request(&form).unwrap_err(), "Indique ton pseudo.");
    }

    #[test]
    fn yaml_importes_remplacent_l_equipage() {
        let mut form = salon();
        form.yamls = vec![
            PathBuf::from("a.yaml"),
            PathBuf::from("b.yaml"),
            PathBuf::from("c.yaml"),
        ];
        assert_eq!(form.player_count(), 3);
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(request.player_files.len(), 3);
    }

    #[test]
    fn sans_yaml() {
        let mut form = valid();
        form.yamls.clear();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Choisis ton jeu dans « Ton monde »."
        );
    }

    #[test]
    fn adresse_vide() {
        let mut form = valid();
        form.public_host = "   ".into();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Indique l'adresse que les autres joueurs utiliseront."
        );
    }

    #[test]
    fn un_refus_ouvre_le_bloc_replie_du_champ_fautif() {
        // Sans YAML : le champ est visible, aucun bloc à ouvrir.
        let mut form = CreateForm::default();
        assert!(build_request(&form).is_err());
        assert_eq!(refused_block(&form), Refused::Visible);
        // Apworld sans « J'ai compris » : le bloc des apworlds.
        form = valid();
        form.apworlds = vec![PathBuf::from("a.apworld")];
        assert!(build_request(&form).is_err());
        assert_eq!(refused_block(&form), Refused::Apworlds);
        open_refused_block(&mut form);
        assert!(form.open_advanced_apworlds && !form.open_advanced);
        // Port, adresse, certificat : les options avancées du serveur.
        form = valid();
        form.port = "abc".into();
        assert!(build_request(&form).is_err());
        open_refused_block(&mut form);
        assert!(form.open_advanced && !form.open_advanced_apworlds);
        form = valid();
        form.security = Security::Pem;
        assert!(build_request(&form).is_err());
        assert_eq!(refused_block(&form), Refused::Server);
    }

    #[test]
    fn port_invalide() {
        let mut form = valid();
        form.port = "0".into();
        assert!(build_request(&form).unwrap_err().contains("port"));
    }

    #[test]
    fn wss_sans_certificat() {
        let mut form = valid();
        form.security = Security::Pem;
        assert!(build_request(&form).unwrap_err().contains("certificat"));
    }

    #[test]
    fn mot_de_passe_vide_absent() {
        let (_, host) = build_request(&valid()).unwrap();
        assert_eq!(host.password, None);
    }

    #[test]
    fn apworlds_installes_ajoutes_sans_avertissement() {
        let mut form = valid();
        form.installed_apworlds = vec![
            PathBuf::from("worlds").join("slime_rancher.apworld"),
            PathBuf::from("worlds").join("autre.apworld"),
        ];
        // Pas besoin de « J'ai compris » : ils viennent du catalogue, hash vérifié.
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(request.apworlds, form.installed_apworlds);

        // Même nom choisi à la main : celui de l'hôte passe avant, sans doublon.
        form.apworlds = vec![PathBuf::from("perso").join("slime_rancher.apworld")];
        form.apworld_ack = true;
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(
            request.apworlds,
            vec![
                PathBuf::from("perso").join("slime_rancher.apworld"),
                PathBuf::from("worlds").join("autre.apworld"),
            ]
        );
    }

    #[test]
    fn apworld_sans_accord_refuse() {
        let mut form = valid();
        form.apworlds = vec![PathBuf::from("monde.apworld")];
        assert!(build_request(&form).unwrap_err().contains("J'ai compris"));
        form.apworld_ack = true;
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(request.apworlds, vec![PathBuf::from("monde.apworld")]);
    }

    #[test]
    fn ajout_sans_doublon_dans_l_ordre() {
        let mut list = vec![PathBuf::from("b.yaml")];
        add_files(
            &mut list,
            vec![
                PathBuf::from("a.yaml"),
                PathBuf::from("b.yaml"),
                PathBuf::from("c.yaml"),
            ],
        );
        assert_eq!(
            list,
            vec![
                PathBuf::from("b.yaml"),
                PathBuf::from("a.yaml"),
                PathBuf::from("c.yaml"),
            ]
        );
    }

    /// Attend que `ready` rende `Some`, en la rappelant toutes les 100 ms, au plus `limit`.
    fn wait_for<T>(limit: std::time::Duration, mut ready: impl FnMut() -> Option<T>) -> T {
        let start = std::time::Instant::now();
        loop {
            if let Some(value) = ready() {
                return value;
            }
            assert!(start.elapsed() < limit, "délai dépassé");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Ce que fait `App` à chaque image : l'erreur du worker passe dans celle de l'écran.
    fn take_worker_error(worker: &crate::worker::Worker, error: &mut Option<String>) {
        if let Some(message) = worker.take_error() {
            *error = Some(message);
        }
    }

    /// Test RÉEL du chemin de l'écran, sans fenêtre : formulaire, générateur installé,
    /// génération dans son thread, hébergement par le worker sur un port occupé, puis
    /// « Réessayer » sur un port libre SANS régénérer.
    ///
    /// Il ne télécharge rien : il lui faut un runtime installé par
    /// `cargo run -p launcher-core --example setup_generator -- <dossier>`, donné par
    /// `RAFTUP_REAL_RUNTIME`. Ports utilisés : 38296 (occupé exprès) et 38297.
    ///
    /// `cargo test -p launcher-ui -- --ignored --nocapture`
    #[test]
    #[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
    fn reel_generer_puis_heberger() {
        use crate::worker::{Command, Worker};
        use std::time::Duration;

        let Some(root) = std::env::var_os("RAFTUP_REAL_RUNTIME") else {
            eprintln!("RAFTUP_REAL_RUNTIME absente : test réel sauté");
            return;
        };
        // L'écran trouve ses dossiers par `load_paths`, qui lit cette variable en debug.
        std::env::set_var("RAFTUP_DEV_RACINE", &root);
        let yamls = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for name in ["Alice", "Bob"] {
            let file = yamls.path().join(format!("{name}.yaml"));
            std::fs::write(
                &file,
                format!("name: {name}\ngame: APQuest\nAPQuest:\n  hard_mode: false\n"),
            )
            .unwrap();
            files.push(file);
        }

        // Le worker, comme dans `main.rs` : un runtime tokio et un contexte egui sans fenêtre.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let ctx = egui::Context::default();
        let (done, done_receiver) = tokio::sync::oneshot::channel();
        let worker = Worker::spawn(
            runtime.handle(),
            ctx.clone(),
            launcher_core::LogBuffer::new(),
            done,
        );

        // Port 38296 occupé exprès : le premier démarrage doit échouer.
        let busy = std::net::TcpListener::bind(("127.0.0.1", 38296)).unwrap();
        let mut form = CreateForm {
            yamls: files,
            security: Security::Plain,
            port: "38296".into(),
            ..CreateForm::default()
        };
        let mut error = None;

        // 1. Clic sur « Générer la partie ».
        on_generate(&ctx, &mut form, &mut error);
        assert_eq!(error, None);
        assert!(
            matches!(form.setup, generator_setup::Setup::Ready),
            "le générateur doit être installé dans RAFTUP_REAL_RUNTIME"
        );
        assert!(form.generation.is_busy());
        // Le formulaire change pendant la génération : ce n'est pas ce port qui sert.
        form.port = "1".into();

        // 2 et 3. Comme `App` à chaque image, mais SANS jamais dessiner l'écran (l'hôte
        // regarde ailleurs) : la génération va au bout, puis l'hébergement part tout seul
        // avec les réglages du clic (port 38296, occupé) et échoue.
        let (output, message) = wait_for(Duration::from_secs(300), || {
            take_worker_error(&worker, &mut error);
            let shared = worker.lock();
            assert!(matches!(
                background(&ctx, &mut form, &mut error, &worker, &shared),
                Started::Nothing
            ));
            if let generation::Generation::Failed { failure, log, .. } = &form.generation {
                panic!("génération ratée : {failure:?}\n{log:?}")
            }
            match &form.hosting {
                hosting::Hosting::Failed { output, message } => {
                    Some((output.clone(), message.clone()))
                }
                _ => None,
            }
        });
        eprintln!(
            "seed {} : {}",
            output.seed_name,
            output.archipelago_file.display()
        );
        assert_eq!(output.players.len(), 2);
        // Les réglages du clic ont servi, une seule fois.
        assert!(form.pending_host.is_none());
        eprintln!("{message}");
        assert!(
            message
                .starts_with("Impossible de démarrer le serveur : le port 38296 est déjà utilisé"),
            "{message}"
        );

        // 4. « Réessayer » sur un port libre : même seed, pas de nouvelle génération.
        form.port = "38297".into();
        retry_hosting(&ctx, &mut form, &mut error, &worker, &worker.lock());
        assert_eq!(error, None);
        wait_for(Duration::from_secs(10), || {
            take_worker_error(&worker, &mut error);
            let shared = worker.lock();
            let started = background(&ctx, &mut form, &mut error, &worker, &shared);
            assert_eq!(error, None);
            matches!(started, Started::HereOnPc).then_some(())
        });
        // L'écran repart propre pour la prochaine partie.
        assert!(matches!(form.generation, generation::Generation::Idle));
        {
            let shared = worker.lock();
            let status = shared.status.as_ref().unwrap();
            assert_eq!(status.seed.seed_name, output.summary.seed_name);
            assert_eq!(status.seed.players, 2);
            eprintln!("partie en ligne : {:?}", status.seed.seed_name);
        }

        // 5. Une partie tourne déjà : refus clair, rien n'est envoyé.
        let again = hosting::start(
            &worker,
            &worker.lock(),
            (*output).clone(),
            build_request(&form).unwrap().1,
        );
        match again {
            hosting::Hosting::Failed { message, .. } => {
                assert_eq!(message, hosting::ALREADY_HOSTING)
            }
            _ => panic!("un deuxième hébergement devait être refusé"),
        }

        // Arrêt propre, comme à la fermeture de la fenêtre.
        drop(busy);
        worker.send(Command::Shutdown);
        runtime.block_on(done_receiver).unwrap();
    }
}
