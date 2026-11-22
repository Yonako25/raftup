//! Illustrations des jeux dans l'interface (ADR 0051).
//!
//! Les écrans demandent une jaquette ou une image large par [`Artwork::texture`].
//! L'image vient du disque : celle du créateur, rangée à l'installation du jeu, sinon
//! celle que Steam garde déjà sur ce PC (`launcher_core::art::ArtSources`). Aucun
//! réseau ni aucune API.
//!
//! La lecture et le décodage se font dans un thread à part, pour que l'écran ne
//! saccade pas ; tant que l'image n'est pas prête, ou s'il n'y en a pas, l'écran
//! dessine sa propre jaquette (aplat coloré et titre). Au démarrage, la Bibliothèque
//! demande d'un coup les images de tous les jeux : elles sont prêtes avant qu'on
//! ouvre un onglet.

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};
use launcher_core::art::{ArtKind, ArtSources};

/// Plus grande image acceptée, en pixels de côté.
const MAX_SIDE: u32 = 4096;

/// Ce qui va chercher les octets d'une image (JPEG ou PNG). `Arc<dyn Fn …>` : une
/// fonction partagée avec le thread de chargement ; le disque, ou un faux dans les tests.
pub type Loader = Arc<dyn Fn(&str, ArtKind) -> Option<Vec<u8>> + Send + Sync>;

/// Clé d'une image : identifiant du jeu et sorte d'image.
type Key = (String, ArtKind);

/// Réponse du thread de chargement.
struct Done {
    key: Key,
    /// `Ok(None)` : pas d'image pour ce jeu. `Err` : raison (pour le journal).
    result: Result<Option<ColorImage>, String>,
}

/// Une demande envoyée au thread.
struct Job {
    key: Key,
    ctx: egui::Context,
}

/// Où en est une image.
enum Slot {
    Loading,
    Ready(TextureHandle),
    /// Pas d'image (ou échec) : pas redemandée, sauf après [`Artwork::forget`].
    Absent,
}

/// Les illustrations chargées, et le thread qui les charge.
pub struct Artwork {
    loader: Option<Loader>,
    slots: BTreeMap<Key, Slot>,
    jobs: Option<Sender<Job>>,
    results_tx: Sender<Done>,
    results: Receiver<Done>,
}

impl Artwork {
    /// Sans source : aucune image, les jaquettes dessinées restent.
    pub fn new() -> Artwork {
        let (results_tx, results) = mpsc::channel();
        Artwork {
            loader: None,
            slots: BTreeMap::new(),
            jobs: None,
            results_tx,
            results,
        }
    }

    /// Images lues sur ce PC. `appids` : l'appid Steam de chaque jeu du catalogue.
    pub fn local(sources: ArtSources, appids: BTreeMap<String, u32>) -> Artwork {
        // `move` : la fermeture garde les sources et la table ; elle part dans le thread.
        let loader: Loader = Arc::new(move |game_id: &str, kind: ArtKind| {
            sources.find(game_id, appids.get(game_id).copied(), kind)
        });
        Artwork::with_loader(loader)
    }

    /// Images données par `loader` (tests, ou [`Artwork::local`]).
    pub fn with_loader(loader: Loader) -> Artwork {
        let mut artwork = Artwork::new();
        artwork.loader = Some(loader);
        artwork
    }

    /// Oublie les images de ce jeu : elles seront relues (après son installation, les
    /// images du créateur viennent d'arriver).
    pub fn forget(&mut self, game_id: &str) {
        self.slots.retain(|(id, _), _| id != game_id);
    }

    /// Texture de cette image si elle est prête. La première demande lance le
    /// chargement ; `None` en attendant, ou s'il n'y a pas d'image.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        game_id: &str,
        kind: ArtKind,
    ) -> Option<TextureHandle> {
        let key = (game_id.to_string(), kind);
        match self.slots.get(&key) {
            Some(Slot::Ready(texture)) => return Some(texture.clone()),
            Some(Slot::Loading | Slot::Absent) => return None,
            None => {}
        }
        let jobs = self.worker()?;
        let job = Job {
            key: key.clone(),
            ctx: ctx.clone(),
        };
        if jobs.send(job).is_ok() {
            self.slots.insert(key, Slot::Loading);
        }
        None
    }

    /// Range les images arrivées depuis la dernière image de l'écran.
    pub fn poll(&mut self, ctx: &egui::Context) {
        // `try_iter` : prend ce qui est déjà arrivé, sans jamais attendre.
        for done in self.results.try_iter() {
            // Une réponse pour une image oubliée entre-temps est jetée : elle sera relue.
            if !matches!(self.slots.get(&done.key), Some(Slot::Loading)) {
                continue;
            }
            let slot = match done.result {
                Ok(Some(image)) => {
                    let (id, kind) = &done.key;
                    let name = format!("art-{id}-{}", kind.as_str());
                    Slot::Ready(ctx.load_texture(name, image, TextureOptions::LINEAR))
                }
                Ok(None) => Slot::Absent,
                Err(reason) => {
                    tracing::info!(%reason, "illustration non chargée : jaquette dessinée gardée");
                    Slot::Absent
                }
            };
            self.slots.insert(done.key, slot);
        }
    }

    /// Canal vers le thread de chargement, lancé à la première demande.
    fn worker(&mut self) -> Option<&Sender<Job>> {
        if self.jobs.is_none() {
            let loader = Arc::clone(self.loader.as_ref()?);
            let (jobs, queue) = mpsc::channel::<Job>();
            let results = self.results_tx.clone();
            // Un seul thread, qui lit les images l'une après l'autre.
            let spawned = std::thread::Builder::new()
                .name("illustrations".to_string())
                .spawn(move || {
                    for job in queue {
                        let (id, kind) = &job.key;
                        let result = loader(id, *kind).map(|bytes| decode(&bytes)).transpose();
                        if results
                            .send(Done {
                                key: job.key,
                                result,
                            })
                            .is_err()
                        {
                            break;
                        }
                        // Réveille l'interface : la texture est prête à être rangée.
                        job.ctx.request_repaint();
                    }
                });
            match spawned {
                Ok(_) => self.jobs = Some(jobs),
                Err(error) => {
                    tracing::warn!(%error, "thread des illustrations non lancé");
                    return None;
                }
            }
        }
        self.jobs.as_ref()
    }
}

/// Décode une image JPEG ou PNG en image egui (RGBA).
fn decode(bytes: &[u8]) -> Result<ColorImage, String> {
    let decoded = image::load_from_memory(bytes).map_err(|e| format!("image illisible : {e}"))?;
    if decoded.width() > MAX_SIDE || decoded.height() > MAX_SIDE {
        return Err("image trop grande".to_string());
    }
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Partie de l'image (coordonnées de 0 à 1) qui remplit un cadre de taille `frame`
/// sans déformer l'image : on coupe ce qui dépasse, au centre (comme « cover » en CSS).
pub fn cover_uv(image: egui::Vec2, frame: egui::Vec2) -> egui::Rect {
    if image.x <= 0.0 || image.y <= 0.0 || frame.x <= 0.0 || frame.y <= 0.0 {
        return egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let image_ratio = image.x / image.y;
    let frame_ratio = frame.x / frame.y;
    if image_ratio > frame_ratio {
        // Image plus large que le cadre : on garde toute la hauteur.
        let width = frame_ratio / image_ratio;
        let left = (1.0 - width) / 2.0;
        egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(left + width, 1.0))
    } else {
        let height = image_ratio / frame_ratio;
        let top = (1.0 - height) / 2.0;
        egui::Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(1.0, top + height))
    }
}

/// Peint `texture` dans `rect` en remplissant le cadre (angles de la charte).
pub fn paint_cover(ui: &egui::Ui, texture: &TextureHandle, rect: egui::Rect) {
    let uv = cover_uv(texture.size_vec2(), rect.size());
    egui::Image::from_texture(texture)
        .uv(uv)
        .corner_radius(egui::CornerRadius::same(crate::theme::RADIUS))
        .paint_at(ui, rect);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoupe_au_centre() {
        // Image carrée dans un cadre en portrait 2:3 : on garde les 2/3 du milieu en largeur.
        let uv = cover_uv(egui::vec2(300.0, 300.0), egui::vec2(100.0, 150.0));
        assert!((uv.min.x - 1.0 / 6.0).abs() < 1e-4);
        assert!((uv.max.x - 5.0 / 6.0).abs() < 1e-4);
        assert_eq!((uv.min.y, uv.max.y), (0.0, 1.0));
        // Image en 16:9 dans un cadre plus haut que large : idem, coupée sur les côtés.
        let uv = cover_uv(egui::vec2(1280.0, 720.0), egui::vec2(400.0, 160.0));
        assert_eq!((uv.min.x, uv.max.x), (0.0, 1.0));
        assert!(uv.min.y > 0.0 && uv.max.y < 1.0);
        // Tailles nulles : toute l'image.
        assert_eq!(
            cover_uv(egui::Vec2::ZERO, egui::vec2(1.0, 1.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
        );
    }
}
