//! Launcher de bureau Raftup (ADR 0010 et 0011) : application egui / eframe.
//!
//! `main` prépare trois choses, puis ouvre la fenêtre :
//! 1. le journal en mémoire (couche `tracing`) que le panneau « Journal du serveur » affiche ;
//! 2. un runtime `tokio` multi-thread, sur lequel tourne le serveur de la partie ;
//! 3. la fenêtre egui, qui ne fait jamais d'`await` : elle parle au serveur par un « worker ».
//!
//! Avant tout cela, `startup` lit le lien `raftup://` éventuel et vérifie qu'aucune autre
//! fenêtre Raftup n'est ouverte ; s'il y en a une, il lui passe le lien et s'arrête.

mod app;
// Macro de développement (captures sans être devant le PC) : jamais dans `--release`.
#[cfg(debug_assertions)]
mod devscript;
mod format;
mod resize;
mod screens;
mod startup;
mod theme;
mod titlebar;
mod update_ui;
mod widgets;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use launcher_core::LogBuffer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Icône de la fenêtre, intégrée au binaire.
const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

fn main() -> anyhow::Result<()> {
    // Le journal : une couche garde les messages en mémoire pour l'interface,
    // une autre les écrit aussi dans la console (utile en développement).
    let logs = LogBuffer::new();
    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(logs.layer())
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Lien reçu de Windows, instance unique et schéma `raftup://` (ADR 0020).
    let link = startup::link_from_args(std::env::args());
    let (guard, instance_dir, link) = match startup::prepare(link) {
        startup::Start::Forwarded => return Ok(()),
        startup::Start::Run {
            guard,
            instance_dir,
            link,
        } => (guard, instance_dir, link),
    };
    // `_guard` (et non `_`) : la variable vit jusqu'à la fin de `main`, donc le verrou
    // de l'instance unique aussi. `let _ = …` le relâcherait tout de suite.
    let _guard = guard;
    // Launcher téléchargé depuis un Rafthub : il vient avec son adresse (ADR 0036).
    startup::adopt_rafthub_file();

    // Le runtime doit rester en vie tant que la fenêtre est ouverte : quand `runtime`
    // est détruit (fin de `main`), le serveur et le worker s'arrêtent avec lui.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Raftup")
        // Sans barre de titre Windows : `titlebar.rs` dessine la nôtre.
        .with_decorations(false)
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([900.0, 600.0]);
    // Une icône illisible ne doit pas empêcher de démarrer : on s'en passe.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON_PNG) {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // Le worker prévient par `done` quand il a arrêté la partie.
    let (done, worker_done) = tokio::sync::oneshot::channel();
    let result = eframe::run_native(
        "Raftup",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            // `Context` est un pointeur partagé : on le clone pour que le worker
            // puisse demander un nouveau dessin depuis un autre thread.
            let worker = worker::Worker::spawn(&handle, cc.egui_ctx.clone(), logs.clone(), done);
            Ok(Box::new(app::App::new(worker, instance_dir, link)))
        }),
    );
    // Fenêtre fermée : on laisse au worker le temps d'arrêter la partie
    // (sauvegarde compactée, joueurs prévenus), au plus 5 secondes, avant de
    // détruire le runtime. Même si la fenêtre a échoué.
    runtime.block_on(async {
        let _ = tokio::time::timeout(Duration::from_secs(5), worker_done).await;
    });
    drop(runtime);
    result.map_err(|e| anyhow::anyhow!("fenêtre : {e}"))?;
    // Mise à jour installée et « Redémarrer » cliqué : le verrou d'instance unique est
    // relâché avant de lancer le nouvel exécutable, sinon il se prendrait pour un doublon.
    drop(_guard);
    update_ui::relaunch_if_requested();
    Ok(())
}
