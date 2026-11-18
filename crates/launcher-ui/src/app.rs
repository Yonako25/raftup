//! État de l'application, navigation et barre latérale.

use egui::{Align2, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};

use crate::screens::{create, library, matches, running, setup, soon};
use crate::theme;
use crate::widgets;
use crate::worker::{Command, Worker};

/// L'écran affiché.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Page {
    /// « En cours » : la liste des parties lancées.
    InProgress,
    /// « Bibliothèque » : les jeux du catalogue, leurs mods, le bouton « Lancer ».
    Library,
    /// La partie hébergée par ce PC, ou le formulaire d'hébergement s'il n'y en a pas.
    Hosted,
    /// « Créer une partie » : générer une partie à partir des YAML des joueurs.
    Create,
    /// Écran « Bientôt » (nom de la fonction à venir). Deux liens vers le même nom
    /// donnent la même page : la barre latérale s'en sert pour savoir lequel est actif.
    Soon(&'static str),
}

pub struct App {
    page: Page,
    worker: Worker,
    setup: setup::SetupForm,
    create: create::CreateForm,
    running: running::RunningState,
    library: library::LibraryState,
    /// Erreur à montrer en rouge en haut de l'écran.
    error: Option<String>,
    /// Macro de développement (`RAFTUP_DEV_CAPTURE`), seulement en debug.
    #[cfg(debug_assertions)]
    dev: Option<crate::devscript::DevScript>,
}

impl App {
    pub fn new(worker: Worker) -> App {
        dev_host(&worker);
        App {
            page: start_page(),
            worker,
            setup: setup::SetupForm::default(),
            create: create::CreateForm::default(),
            running: running::RunningState::default(),
            library: library::LibraryState::new(),
            error: None,
            #[cfg(debug_assertions)]
            dev: crate::devscript::DevScript::from_env(),
        }
    }
}

/// Page d'ouverture. En debug, `RAFTUP_DEV_PAGE` choisit l'écran affiché au démarrage
/// (captures d'écran pour les comparaisons avec la maquette) : `bibliotheque`,
/// `partie`, `creer`, `telechargements`, `salons`, `catalogue`, `mises-a-jour`, `profil`.
fn start_page() -> Page {
    // `cfg!(debug_assertions)` vaut faux en `--release` : le binaire livré ignore la variable.
    if !cfg!(debug_assertions) {
        return Page::InProgress;
    }
    match std::env::var("RAFTUP_DEV_PAGE").ok().as_deref() {
        Some("bibliotheque") => Page::Library,
        Some("partie") => Page::Hosted,
        Some("creer") => Page::Create,
        Some("telechargements") => Page::Soon("Téléchargements"),
        Some("salons") => Page::Soon("Salons"),
        Some("catalogue") => Page::Soon("Catalogue"),
        Some("mises-a-jour") => Page::Soon("Mises à jour"),
        Some("profil") => Page::Soon("Mon profil"),
        _ => Page::InProgress,
    }
}

/// En debug, `RAFTUP_DEV_SEED=<fichier .archipelago>` héberge cette seed dès l'ouverture
/// (en `ws`, port 38299, sans sauvegarde) : sert aux captures des écrans de partie.
fn dev_host(worker: &Worker) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(seed) = std::env::var_os("RAFTUP_DEV_SEED") else {
        return;
    };
    worker.send(Command::Start(launcher_core::HostOptions {
        seed: seed.into(),
        port: 38299,
        password: None,
        encryption: launcher_core::Encryption::Plain,
        public_host: "localhost".to_string(),
        disable_save: true,
    }));
}

impl eframe::App for App {
    /// Fermeture de la fenêtre : le worker arrête la partie proprement.
    fn on_exit(&mut self) {
        self.worker.send(Command::Shutdown);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::BG.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // La durée « il y a … » doit avancer même si rien d'autre ne bouge.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        theme::paint_background(&ctx);

        // Une erreur d'une commande (port pris, certificat refusé…) arrive du worker.
        if let Some(message) = self.worker.take_error() {
            self.error = Some(message);
        }

        // Le verrou est tenu le temps du dessin : le worker attend au plus une image.
        let shared = self.worker.lock();
        let hosting = shared.status.is_some();

        let mut goto: Option<Page> = None;
        // Génération et hébergement avancent même si l'écran « Créer » n'est pas affiché.
        let started = create::background(&mut self.create, &mut self.error, &self.worker, &shared);
        // On ne bascule vers la partie que si l'hôte regarde l'écran « Créer » : ailleurs,
        // on ne lui retire pas l'écran sous les yeux (la partie apparaît dans « En cours »).
        if started && self.page == Page::Create {
            goto = Some(Page::Hosted);
        }
        // Plus de partie : la fenêtre « Arrêter le serveur ? », l'onglet et le message en
        // cours de frappe ne doivent pas réapparaître sur la partie suivante.
        if !hosting {
            self.running = running::RunningState::default();
        }
        // La barre de titre est déclarée avant la barre latérale : elle prend toute la largeur.
        egui::Panel::top("titlebar")
            .exact_size(crate::titlebar::HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(theme::sidebar_fill()))
            .show(ui, crate::titlebar::show);
        egui::Panel::left("sidebar")
            .exact_size(220.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::sidebar_fill())
                    .inner_margin(Margin::symmetric(0, 18)),
            )
            .show(ui, |ui| {
                // `if let` : sans clic dans la barre, on garde la bascule déjà décidée
                // (partie tout juste démarrée depuis « Créer une partie »).
                // Défilement : si la fenêtre est très basse, le bas de la barre reste
                // atteignable au lieu d'être coupé.
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if let Some(page) =
                            sidebar(ui, &self.page, &shared, self.library.game_count())
                        {
                            goto = Some(page);
                        }
                    });
            });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let next = match &self.page {
                Page::InProgress => matches::show(ui, &shared, self.error.as_deref()),
                Page::Library => {
                    library::show(ui, &mut self.library, &shared, self.error.as_deref())
                }
                Page::Hosted if hosting => running::show(
                    ui,
                    &mut self.running,
                    &mut self.error,
                    &self.worker,
                    &shared,
                ),
                Page::Hosted => {
                    setup::show(ui, &mut self.setup, &mut self.error, &self.worker, &shared)
                }
                Page::Create => {
                    create::show(ui, &mut self.create, &mut self.error, &self.worker, &shared)
                }
                Page::Soon(name) => soon::show(ui, name),
            };
            if next.is_some() {
                goto = next;
            }
        });

        drop(shared);
        // Verrou relâché : on peut ouvrir une boîte de dialogue (elle bloque).
        setup::open_pending_dialog(&mut self.setup, &self.worker);
        create::open_pending_dialog(&mut self.create);
        library::open_pending_dialog(&mut self.library, &ctx);
        if let Some(page) = goto {
            // Changement d'écran : l'erreur de l'écran quitté ne le suit pas (« Ajoutez
            // au moins un fichier YAML. » n'a pas de sens sur « Héberger une partie »).
            if self.page != page {
                self.error = None;
            }
            self.page = page;
        }
        // Poignées invisibles sur les bords de la fenêtre sans bordure.
        crate::resize::handle(&ctx);
        #[cfg(debug_assertions)]
        if let Some(dev) = &mut self.dev {
            dev.step(
                &ctx,
                &self.page,
                &mut crate::devscript::Forms {
                    create: &mut self.create,
                    setup: &mut self.setup,
                },
                &mut self.error,
                &self.worker,
            );
        }
    }
}

/// Largeur de la barre latérale.
const SIDEBAR_WIDTH: f32 = 220.0;
/// Marge gauche des cartes du bas de la barre.
const CARD_GAP: f32 = 14.0;

/// Un lien de la barre latérale : texte, compteur, page ouverte au clic.
type NavLink = (&'static str, usize, Page);

/// Barre latérale : logo, liens rangés par section, cartes du bas. Renvoie la page demandée.
fn sidebar(
    ui: &mut Ui,
    current: &Page,
    shared: &crate::worker::Shared,
    games: usize,
) -> Option<Page> {
    let mut goto = None;
    ui.set_width(SIDEBAR_WIDTH);
    ui.spacing_mut().item_spacing.y = 2.0;

    // Logo + nom.
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        logo(ui, 34.0);
        ui.add_space(2.0);
        ui.label(
            RichText::new("RAFTUP")
                .font(egui::FontId::new(22.0, theme::title_bold_family()))
                .extra_letter_spacing(2.0)
                .color(theme::TEXT),
        );
    });
    ui.add_space(10.0);

    // Les compteurs à 0 ne s'affichent pas (jeux du catalogue, partie hébergée).
    let hosted = usize::from(shared.status.is_some());
    let in_progress = matches!(current, Page::InProgress | Page::Hosted | Page::Create);
    let sections: [(&str, Vec<NavLink>); 3] = [
        (
            "JEUX",
            vec![
                ("Accueil", 0, Page::Soon("Accueil")),
                ("Bibliothèque", games, Page::Library),
                ("Téléchargements", 0, Page::Soon("Téléchargements")),
            ],
        ),
        (
            "PARTIES",
            vec![
                ("En cours", hosted, Page::InProgress),
                ("Salons", 0, Page::Soon("Salons")),
                ("Invitations", 0, Page::Soon("Invitations")),
            ],
        ),
        (
            "CATALOGUE",
            vec![
                ("Parcourir", 0, Page::Soon("Catalogue")),
                ("Mises à jour", 0, Page::Soon("Mises à jour")),
            ],
        ),
    ];
    for (heading, links) in sections {
        section_heading(ui, heading);
        for (label, count, page) in links {
            // « En cours » reste actif quand on ouvre la partie hébergée.
            let active = *current == page || (page == Page::InProgress && in_progress);
            if nav_link(ui, label, count, active) {
                goto = Some(page);
            }
        }
    }

    // Le bas de la barre : amis, profil, serveur, mention de compatibilité.
    // Pour le coller en bas, on saute l'espace libre. Sa hauteur n'est connue qu'après
    // l'avoir dessiné : on la garde dans la mémoire d'egui pour l'image suivante.
    let memory_id = ui.scope_id().with("sidebar_bottom");
    let bottom_height: f32 = ui
        .ctx()
        .data(|data| data.get_temp(memory_id))
        .unwrap_or(0.0);
    let free = ui.available_height() - bottom_height;
    if free > 0.0 {
        ui.add_space(free);
    }
    let top = ui.cursor().top();
    ui.spacing_mut().item_spacing.y = 10.0;
    if side_card(ui, friends_card) {
        goto = Some(Page::Soon("Amis"));
    }
    if side_card(ui, profile_card) {
        goto = Some(Page::Soon("Mon profil"));
    }
    if side_card(ui, |ui| server_card(ui, shared)) {
        goto = Some(Page::Hosted);
    }
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.label(widgets::dim("Compatible avec Archipelago 0.6.8").size(11.0));
    });
    let height = ui.cursor().top() - top;
    if (height - bottom_height).abs() > 0.5 {
        ui.ctx()
            .data_mut(|data| data.insert_temp(memory_id, height));
        // La hauteur a changé (première image, carte du serveur qui grandit) : redessiner.
        ui.ctx().request_repaint();
    }
    goto
}

/// Titre de section en petites capitales espacées (« JEUX », « PARTIES »…).
fn section_heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        ui.label(
            RichText::new(text)
                .size(11.0)
                .extra_letter_spacing(1.5)
                .color(theme::TEXT_DIM),
        );
    });
    ui.add_space(2.0);
}

/// Une carte du bas de la barre latérale, cliquable en entier. Renvoie vrai au clic.
fn side_card(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui)) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(CARD_GAP);
        let response = Frame::new()
            .fill(theme::FIELD)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                // Largeur intérieure : la barre, moins les deux marges et le cadre.
                ui.set_width(SIDEBAR_WIDTH - 2.0 * CARD_GAP - 26.0);
                // Le cadre est dans une ligne (`horizontal`) : sans `vertical`, son
                // contenu se mettrait en ligne lui aussi.
                ui.vertical(add_contents);
            })
            .response;
        // `interact` rend toute la surface de la carte cliquable, pas seulement son texte.
        let response = response.interact(Sense::click());
        if response.hovered() {
            ui.painter().rect_stroke(
                response.rect,
                CornerRadius::same(theme::RADIUS),
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        clicked = response.clicked();
    });
    clicked
}

/// Carte « Amis » : les comptes et la présence arrivent avec le hub, après le MVP.
fn friends_card(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Amis").strong().color(theme::TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(widgets::dim("bientôt").size(12.0));
        });
    });
}

/// Carte du profil : pas encore de compte, on montre un joueur local.
fn profile_card(ui: &mut Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::ITEM);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "?",
            theme::title_font(16.0),
            theme::FIELD,
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new("Joueur").strong().color(theme::TEXT));
            ui.label(widgets::dim("Mon profil").size(12.0));
        });
    });
}

/// Carte « Serveur intégré » : ce que le serveur du launcher héberge.
fn server_card(ui: &mut Ui, shared: &crate::worker::Shared) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.horizontal(|ui| {
        logo(ui, 16.0);
        ui.label(
            RichText::new("SERVEUR INTÉGRÉ")
                .size(11.0)
                .extra_letter_spacing(1.5)
                .color(theme::TEXT_DIM),
        );
    });
    match &shared.status {
        Some(status) => {
            let name: String = status.seed.seed_name.chars().take(8).collect();
            let dots = if status.seed.seed_name.chars().count() > 8 {
                "…"
            } else {
                ""
            };
            ui.label(RichText::new("● En ligne").color(theme::GREEN));
            ui.label(widgets::dim(format!("seed {name}{dots}")).size(12.0));
            let connected = status
                .server
                .slots
                .iter()
                .filter(|s| !s.is_group && s.connections > 0)
                .count();
            let plural = if connected > 1 { "s" } else { "" };
            ui.label(
                widgets::dim(format!("{connected} joueur{plural} connecté{plural}")).size(12.0),
            );
        }
        None if shared.starting => {
            ui.label(RichText::new("Démarrage…").color(theme::AMBER));
        }
        None => {
            ui.label(widgets::dim("Arrêté · aucune partie hébergée"));
        }
    }
}

/// Un lien de navigation : le lien actif a un fond bleu sombre et un bord gauche bleu.
/// Le compteur n'apparaît que s'il vaut plus de 0.
fn nav_link(ui: &mut Ui, label: &str, count: usize, active: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 0.0, theme::ACCENT_BG);
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height()));
        painter.rect_filled(bar, 0.0, theme::ACCENT);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, Color32::from_white_alpha(10));
    }
    let color = if active { theme::TEXT } else { theme::TEXT_DIM };
    painter.text(
        rect.left_center() + egui::vec2(18.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(15.0),
        color,
    );
    if count > 0 {
        painter.text(
            rect.right_center() - egui::vec2(18.0, 0.0),
            Align2::RIGHT_CENTER,
            count.to_string(),
            egui::FontId::proportional(13.0),
            theme::TEXT_DIM,
        );
    }
    response.clicked()
}

/// Le logo, découpé tel quel dans la maquette (`docs/maquette`), fond transparent.
const LOGO_PNG: &[u8] = include_bytes!("../icons/logo.png");

/// Affiche le logo dans un carré de `size` points.
///
/// L'image n'est envoyée à la carte graphique qu'une fois : la « texture » est gardée
/// dans la mémoire d'egui (`ctx.data`) et réutilisée à chaque image suivante.
fn logo(ui: &mut Ui, size: f32) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique("raftup_logo");
    // `TextureHandle` est une poignée partagée : la cloner ne copie pas l'image.
    let mut texture: Option<egui::TextureHandle> = ctx.data(|data| data.get_temp(id));
    if texture.is_none() {
        // Même décodeur PNG que l'icône de la fenêtre (main.rs) : pas de dépendance en plus.
        if let Ok(icon) = eframe::icon_data::from_png_bytes(LOGO_PNG) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            let handle = ctx.load_texture("raftup_logo", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|data| data.insert_temp(id, handle.clone()));
            texture = Some(handle);
        }
    }
    // Le logo est presque carré (156 × 159) : on garde ses proportions, `size` est sa hauteur.
    let size = egui::vec2(size * 156.0 / 159.0, size);
    match texture {
        Some(texture) => {
            ui.add(egui::Image::new((texture.id(), size)));
        }
        // PNG illisible (ne devrait pas arriver, il est compilé dans le programme) :
        // on garde juste la place.
        None => {
            ui.allocate_exact_size(size, Sense::hover());
        }
    }
}
