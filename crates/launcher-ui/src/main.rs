//! Launcher de bureau Raftup (ADR 0010 et 0011) : application egui / eframe.
//!
//! `main` prépare trois choses, puis ouvre la fenêtre :
//! 1. le journal en mémoire (couche `tracing`) que le panneau « Journal du serveur » affiche ;
//! 2. un runtime `tokio` multi-thread, sur lequel tourne le serveur de la partie ;
//! 3. la fenêtre egui, qui ne fait jamais d'`await` : elle parle au serveur par un « worker ».

mod app;
mod format;
mod screens;
mod theme;
mod widgets;
mod worker;

use std::sync::Arc;

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

    // Le runtime doit rester en vie tant que la fenêtre est ouverte : quand `runtime`
    // est détruit (fin de `main`), le serveur et le worker s'arrêtent avec lui.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Raftup")
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

    eframe::run_native(
        "Raftup",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            // `Context` est un pointeur partagé : on le clone pour que le worker
            // puisse demander un nouveau dessin depuis un autre thread.
            let worker = worker::Worker::spawn(&handle, cc.egui_ctx.clone(), logs.clone());
            Ok(Box::new(app::App::new(worker)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("fenêtre : {e}"))?;
    drop(runtime);
    Ok(())
}
