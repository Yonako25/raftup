//! Écran « Créer une partie », étape 1 : la préparation.
//!
//! L'hôte choisit les fichiers YAML des joueurs (un par joueur, ADR 0019 : par la
//! boîte de fichiers seulement), d'éventuels apworlds, puis les réglages du serveur.
//! `build_request` vérifie le tout et en fait une demande de génération et des
//! réglages d'hébergement (`docs/contrat-ui-generation.md`).
//!
//! Au clic sur « Générer », si le formulaire est bon, on vérifie que le générateur
//! est installé ; sinon on demande l'accord de l'hôte pour l'installer (étape 2,
//! `generator_setup`). S'il est prêt, la génération part dans un thread (étape 3,
//! `generation`). La seed générée est ensuite hébergée par le worker (étape 4,
//! `hosting`), puis l'écran bascule vers « Partie en cours ».

use std::path::PathBuf;

use egui::{RichText, Ui};
use launcher_core::generation::{
    installed_apworlds, load_native_worlds, rust_support_with_worlds, GenerateRequest,
};
use launcher_core::{Encryption, PartyHostOptions};

use super::library::{load_paths, AP_VERSION};
use super::prefs;
use super::presets::Preset;
use super::{generation, generator_setup, hosting};
use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::{Shared, Worker};

/// Avertissement obligatoire avant d'ajouter un apworld (contrat, « Avertissements »).
pub const APWORLD_WARNING: &str = "Un apworld est du code qui s'exécute sur votre PC, sans bac \
     à sable. N'ajoutez que ceux que vous jugez sûrs.";

/// Boîte de dialogue à ouvrir après le dessin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Yaml,
    Apworld,
    Cert,
    Key,
}

/// Ce que l'hôte a saisi.
pub struct CreateForm {
    /// Fichiers YAML, dans l'ordre d'ajout (sans doublon).
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
    encrypted: bool,
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
}

impl Default for CreateForm {
    /// Mêmes valeurs par défaut que l'écran « Héberger une partie ».
    fn default() -> CreateForm {
        CreateForm {
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
            encrypted: true,
            cert: None,
            key: None,
            setup: generator_setup::Setup::default(),
            generation: generation::Generation::default(),
            pending_host: None,
            hosting: hosting::Hosting::default(),
            open_advanced: false,
            open_advanced_apworlds: false,
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
        form
    }
}

/// Fait avancer la génération et l'hébergement, que l'écran soit affiché ou non.
///
/// `App` l'appelle à CHAQUE image, avant de dessiner : si l'hôte va sur un autre écran
/// pendant la génération, la seed est quand même hébergée dès qu'elle est prête, et
/// une erreur de démarrage du serveur arrive dans le panneau d'hébergement (avec
/// « Réessayer ») au lieu de s'afficher, puis de se perdre, sur un autre écran.
///
/// Rend vrai quand le serveur vient de démarrer.
pub fn background(
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) -> bool {
    generation::poll(&mut form.generation);
    // Seed tout juste générée : on l'héberge avec les réglages du clic (une fois).
    if let Some(output) = form.generation.output() {
        if let Some(options) = form.pending_host.take() {
            // Une vieille erreur (d'un autre écran) passerait sinon pour un échec
            // du démarrage : `poll` prend la première erreur qu'il trouve.
            *error = None;
            form.hosting = hosting::start(worker, shared, output.clone(), options);
        }
    }
    if hosting::poll(&mut form.hosting, shared, error) == hosting::Outcome::Started {
        // Le serveur tourne : l'écran repart propre pour la prochaine partie.
        form.generation = generation::Generation::Idle;
        return true;
    }
    false
}

pub fn show(
    ui: &mut Ui,
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) -> Option<Page> {
    let mut goto = None;
    super::page(ui, |ui| {
        super::header(
            ui,
            "Parties › Nouvelle partie",
            "Créer une partie",
            |ui| {
                if widgets::ghost_button(ui, "Héberger un fichier .archipelago") {
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
            retry_hosting(form, error, worker, shared);
        }

        if ui.available_width() >= super::WIDE {
            ui.columns(2, |columns| {
                players_panel(&mut columns[0], form);
                server_panel(&mut columns[1], form, error);
            });
        } else {
            players_panel(ui, form);
            ui.add_space(12.0);
            server_panel(ui, form, error);
        }
    });
    goto
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

fn players_panel(ui: &mut Ui, form: &mut CreateForm) {
    widgets::panel(ui, "Joueurs", |ui| {
        ui.label(widgets::dim("Un fichier YAML par joueur.").size(13.0));
        ui.add_space(6.0);
        file_list(ui, &mut form.yamls, "Aucun fichier YAML pour l'instant.");
        ui.add_space(6.0);
        if widgets::ghost_button(ui, "Ajouter des YAML…") {
            form.pending_pick = Some(Pick::Yaml);
        }
        let count = form.yamls.len();
        if count > 0 {
            let plural = if count > 1 { "s" } else { "" };
            ui.label(widgets::dim(format!("{count} joueur{plural}")).size(13.0));
        }
        if !form.installed_apworlds.is_empty() {
            let names: Vec<String> = form
                .installed_apworlds
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
            // Deux lignes courtes : le panneau justifie un texte qui passe à la ligne,
            // ce qui étirait les espaces entre les lettres.
            ui.add_space(6.0);
            ui.label(
                widgets::dim("Pris automatiquement (installés par la Bibliothèque) :").size(13.0),
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
            "Apworlds en plus",
            &mut form.open_advanced_apworlds,
            |ui| {
                ui.label(
                    RichText::new(APWORLD_WARNING)
                        .color(theme::AMBER)
                        .size(13.0),
                );
                ui.checkbox(&mut form.apworld_ack, "J'ai compris");
                ui.add_space(6.0);
                file_list(ui, &mut form.apworlds, "Aucun apworld en plus.");
                ui.add_space(6.0);
                // `add_enabled` grise le bouton tant que l'avertissement n'est pas coché.
                let add = ui.add_enabled(
                    form.apworld_ack,
                    egui::Button::new(RichText::new("Ajouter des apworlds…").color(theme::TEXT)),
                );
                if add.clicked() {
                    form.pending_pick = Some(Pick::Apworld);
                }
            },
        );
    });
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

fn server_panel(ui: &mut Ui, form: &mut CreateForm, error: &mut Option<String>) {
    widgets::panel(ui, "Serveur", |ui| {
        labelled(ui, "Mot de passe de la salle (facultatif)");
        ui.add(
            egui::TextEdit::singleline(&mut form.password)
                .password(true)
                .desired_width(f32::INFINITY),
        );

        // Réglages d'expert repliés, avec un résumé d'une ligne de ce qui sera utilisé.
        ui.add_space(4.0);
        ui.label(
            widgets::dim(format::server_summary(
                &form.port,
                &form.public_host,
                form.encrypted,
            ))
            .size(12.0),
        );
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

        labelled(ui, "Préréglage de génération");
        ui.horizontal(|ui| {
            for preset in Preset::ALL {
                ui.selectable_value(&mut form.preset, preset, preset.label());
            }
        });
        ui.label(widgets::dim(form.preset.description()).size(13.0));

        ui.add_space(6.0);
        ui.checkbox(&mut form.rust_engine, "Moteur Rust (expérimental)");
        ui.label(
            widgets::dim(
                "Génère sans Python quand tous les joueurs jouent à APQuest ; sinon, Archipelago \
                 construit les mondes et Raftup place les objets.",
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

        ui.add_space(12.0);
        // Grisé pendant l'installation du générateur et pendant la génération.
        let enabled =
            !form.setup.is_busy() && !form.generation.is_busy() && !form.hosting.is_busy();
        if widgets::primary_button(ui, "▶ Générer la partie", enabled) {
            on_generate(ui.ctx(), form, error);
        }
    });
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
    let wss = widgets::choice_card(
        ui,
        form.encrypted,
        "Chiffré (wss) — recommandé",
        widgets::dim("Demande un certificat pour votre nom de domaine.").size(13.0),
        |ui| {
            ui.add_space(4.0);
            ui.label(widgets::dim("Certificat (PEM)").size(13.0));
            if file_row(ui, &form.cert) {
                form.pending_pick = Some(Pick::Cert);
            }
            ui.label(widgets::dim("Clé privée (PEM)").size(13.0));
            if file_row(ui, &form.key) {
                form.pending_pick = Some(Pick::Key);
            }
        },
    );
    if wss {
        form.encrypted = true;
    }
    ui.add_space(4.0);
    let plain = widgets::choice_card(
        ui,
        !form.encrypted,
        "En clair (ws)",
        RichText::new("Le mot de passe de la salle passe en clair sur le réseau.")
            .color(theme::AMBER)
            .size(13.0),
        |_| {},
    );
    if plain {
        form.encrypted = false;
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
    if form.yamls.is_empty() {
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
        Refused::Apworlds => form.open_advanced_apworlds = true,
        Refused::Server => form.open_advanced = true,
    }
}

/// Clic sur « Générer la partie » (ou « Réessayer ») : vérifie le formulaire, puis
/// le générateur, puis lance la génération.
fn on_generate(ctx: &egui::Context, form: &mut CreateForm, error: &mut Option<String>) {
    // Relu à chaque clic : un jeu a pu être installé depuis l'ouverture de l'écran.
    form.refresh_installed();
    match build_request(form) {
        Ok((request, host)) => {
            *error = None;
            // Mémorise le webhook (ou l'efface s'il est vide) et le préréglage.
            prefs::remember(host.webhook.as_ref(), Some(form.preset));
            form.hosting = hosting::Hosting::Idle;
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
                form.pending_host = Some(host);
                form.generation = generation::start(ctx, request, engine, note, time_balance);
                return;
            }
            // Formulaire bon : on regarde ensuite si le générateur est là.
            // S'il manque, le panneau de consentement s'affiche en haut.
            form.setup = generator_setup::check();
            form.generation = match form.setup {
                generator_setup::Setup::Ready => {
                    // Gardés pour héberger la seed à la fin de la génération.
                    form.pending_host = Some(host);
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
                "Moteur Rust seul impossible ({reason}) : Archipelago construit les mondes, Raftup place les objets."
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
    form.encrypted = false;
    form.port = port.to_string();
    form.rust_engine = std::env::var("RAFTUP_DEV_RUST").is_ok_and(|v| v == "1");
    form.time_balance = std::env::var("RAFTUP_DEV_TEMPS").is_ok_and(|v| v == "1");
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
pub fn dev_retry(form: &mut CreateForm, error: &mut Option<String>, worker: &Worker, port: u16) {
    form.port = port.to_string();
    let shared = worker.lock();
    retry_hosting(form, error, worker, &shared);
}

/// « Réessayer » après un démarrage raté : relit les réglages du serveur dans le
/// formulaire (l'hôte a pu changer le port) et relance avec la MÊME seed.
fn retry_hosting(
    form: &mut CreateForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) {
    let Some(output) = form.hosting.failed_output().cloned() else {
        return;
    };
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
                .set_title("Choisir les fichiers YAML des joueurs")
                .add_filter("Réglages de joueur", &["yaml", "yml"])
                .pick_files();
            add_files(&mut form.yamls, picked.unwrap_or_default());
        }
        Pick::Apworld => {
            let picked = rfd::FileDialog::new()
                .set_title("Choisir des apworlds")
                .add_filter("Monde Archipelago", &["apworld"])
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

fn build_request(form: &CreateForm) -> Result<(GenerateRequest, PartyHostOptions), String> {
    if form.yamls.is_empty() {
        return Err("Ajoutez au moins un fichier YAML.".into());
    }
    if !form.apworlds.is_empty() && !form.apworld_ack {
        return Err(format!("Cochez « J'ai compris » : {APWORLD_WARNING}"));
    }
    let port = format::parse_port(&form.port)?;
    let public_host = form.public_host.trim().to_string();
    if public_host.is_empty() {
        return Err("Indiquez l'adresse que les autres joueurs utiliseront.".into());
    }
    let encryption =
        if form.encrypted {
            match (&form.cert, &form.key) {
                (Some(cert), Some(key)) => Encryption::Wss {
                    cert: cert.clone(),
                    key: key.clone(),
                },
                _ => return Err(
                    "Le chiffrement wss demande un certificat et une clé privée (fichiers PEM). \
                     Sans certificat, choisissez « En clair (ws) »."
                        .into(),
                ),
            }
        } else {
            Encryption::Plain
        };
    let password = Some(form.password.clone()).filter(|p| !p.is_empty());
    let webhook = format::parse_webhook(&form.webhook)?;

    // `..GenerateRequest::default()` : les autres champs (seed, spoiler, délai)
    // gardent leur valeur par défaut, comme un constructeur avec valeurs par défaut.
    let mut request = GenerateRequest {
        player_files: form.yamls.clone(),
        apworlds: all_apworlds(form),
        ..GenerateRequest::default()
    };
    // Le preset ne règle que des champs génériques de la demande (F06, ADR 0023).
    form.preset.apply(&mut request);
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
            encrypted: false,
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

    #[test]
    fn sans_yaml() {
        let mut form = valid();
        form.yamls.clear();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Ajoutez au moins un fichier YAML."
        );
    }

    #[test]
    fn adresse_vide() {
        let mut form = valid();
        form.public_host = "   ".into();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Indiquez l'adresse que les autres joueurs utiliseront."
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
        form.encrypted = true;
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
        form.encrypted = true;
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
        let busy = std::net::TcpListener::bind(("0.0.0.0", 38296)).unwrap();
        let mut form = CreateForm {
            yamls: files,
            encrypted: false,
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
            assert!(!background(&mut form, &mut error, &worker, &shared));
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
        retry_hosting(&mut form, &mut error, &worker, &worker.lock());
        assert_eq!(error, None);
        wait_for(Duration::from_secs(10), || {
            take_worker_error(&worker, &mut error);
            let shared = worker.lock();
            let started = background(&mut form, &mut error, &worker, &shared);
            assert_eq!(error, None);
            started.then_some(())
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
