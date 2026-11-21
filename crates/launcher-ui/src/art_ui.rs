//! Illustrations des jeux dans l'interface (ADR 0047).
//!
//! La Bibliothèque demande une jaquette ou une image large par [`Artwork::texture`].
//! La première demande part dans un thread à part (réseau, disque, décodage du
//! JPEG) ; tant que la réponse n'est pas là, ou s'il n'y en a pas (pas de hub, hub
//! hors ligne sans cache, jeu inconnu), l'écran garde son aplat coloré. Rien n'est
//! jamais affiché en erreur : la raison va au journal.
//!
//! Les images viennent du hub, qui les prend chez IGDB ; elles ne sont jamais dans
//! le dépôt ni dans le binaire.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};
use launcher_core::art::{ArtClient, ArtError, ArtImage, ArtKind, ArtRef};
use launcher_core::hub_client::HubUrl;

/// Plus grande image acceptée, en pixels de côté (IGDB envoie au plus 1280 × 720 pour
/// l'image large et 528 × 748 pour la jaquette).
const MAX_SIDE: u32 = 4096;

/// Ce qui va chercher une image. `Arc<dyn Fn …>` : une fonction partagée avec le
/// thread de chargement ; le vrai client du hub, ou un faux dans les tests.
pub type Loader = Arc<dyn Fn(ArtRef, ArtKind) -> Result<Option<ArtImage>, ArtError> + Send + Sync>;

/// Une image décodée, prête à devenir une texture, avec son attribution.
struct Decoded {
    image: ColorImage,
    attribution: String,
    attribution_url: String,
}

/// Réponse du thread de chargement.
struct Done {
    key: (ArtRef, ArtKind),
    /// `Ok(None)` : pas d'image pour ce jeu. `Err` : raison (pour le journal).
    result: Result<Option<Decoded>, String>,
}

/// Une demande envoyée au thread.
struct Job {
    key: (ArtRef, ArtKind),
    ctx: egui::Context,
}

/// Où en est une image.
enum Slot {
    Loading,
    Ready(TextureHandle),
    /// Pas d'image (ou échec) : on n'en redemande pas pendant cette session.
    Absent,
}

/// Les illustrations chargées, et le thread qui les charge.
pub struct Artwork {
    /// Adresse du hub et dossier du cache qui ont servi à fabriquer `loader`.
    source: Option<(HubUrl, PathBuf)>,
    loader: Option<Loader>,
    slots: BTreeMap<(ArtRef, ArtKind), Slot>,
    jobs: Option<Sender<Job>>,
    results_tx: Sender<Done>,
    results: Receiver<Done>,
    /// Texte et lien d'attribution de la dernière image reçue.
    attribution: Option<(String, String)>,
}

impl Artwork {
    /// Sans hub : aucune image, les aplats restent.
    pub fn new() -> Artwork {
        let (results_tx, results) = mpsc::channel();
        Artwork {
            source: None,
            loader: None,
            slots: BTreeMap::new(),
            jobs: None,
            results_tx,
            results,
            attribution: None,
        }
    }

    /// Tests : images données par `loader`, sans hub ni disque.
    #[cfg(test)]
    pub fn with_loader(loader: Loader) -> Artwork {
        let mut artwork = Artwork::new();
        artwork.loader = Some(loader);
        artwork
    }

    /// Suit le hub choisi dans « Amis ». Un autre hub (ou plus de hub) oublie les
    /// images chargées : elles viendront de son cache à lui.
    pub fn set_hub(&mut self, hub: Option<&HubUrl>, cache_root: Option<&Path>) {
        let wanted = match (hub, cache_root) {
            (Some(hub), Some(root)) => Some((hub.clone(), root.to_path_buf())),
            _ => None,
        };
        if wanted == self.source {
            return;
        }
        self.source = wanted.clone();
        self.loader = wanted.map(|(hub, root)| {
            let client = ArtClient::new(hub, &root);
            // `move` : la fermeture garde le client ; elle part dans le thread.
            Arc::new(move |reference, kind| client.fetch(reference, kind)) as Loader
        });
        self.slots.clear();
        // L'ancien thread finit ce qu'il a reçu puis s'arrête (son canal est fermé).
        self.jobs = None;
        self.attribution = None;
    }

    /// Texture de cette image si elle est prête. La première demande lance le
    /// chargement ; `None` en attendant, ou s'il n'y a pas d'image.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        reference: Option<ArtRef>,
        kind: ArtKind,
    ) -> Option<TextureHandle> {
        let reference = reference?;
        let key = (reference, kind);
        match self.slots.get(&key) {
            Some(Slot::Ready(texture)) => return Some(texture.clone()),
            Some(Slot::Loading | Slot::Absent) => return None,
            None => {}
        }
        let jobs = self.worker()?;
        let job = Job {
            key,
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
            let slot = match done.result {
                Ok(Some(decoded)) => {
                    let (reference, kind) = done.key;
                    let name = format!("art-{reference:?}-{}", kind.as_str());
                    let texture = ctx.load_texture(name, decoded.image, TextureOptions::LINEAR);
                    self.attribution = Some((decoded.attribution, decoded.attribution_url));
                    Slot::Ready(texture)
                }
                Ok(None) => Slot::Absent,
                Err(reason) => {
                    tracing::info!(%reason, "illustration non chargée : aplat gardé");
                    Slot::Absent
                }
            };
            self.slots.insert(done.key, slot);
        }
    }

    /// Texte et lien d'attribution à afficher, dès qu'une image est à l'écran.
    pub fn attribution(&self) -> Option<(&str, &str)> {
        self.attribution
            .as_ref()
            .map(|(text, url)| (text.as_str(), url.as_str()))
    }

    /// Canal vers le thread de chargement, lancé à la première demande.
    fn worker(&mut self) -> Option<&Sender<Job>> {
        if self.jobs.is_none() {
            let loader = Arc::clone(self.loader.as_ref()?);
            let (jobs, queue) = mpsc::channel::<Job>();
            let results = self.results_tx.clone();
            // Un seul thread, qui traite les demandes l'une après l'autre : le hub ne
            // reçoit pas toute la Bibliothèque d'un coup.
            let spawned = std::thread::Builder::new()
                .name("illustrations".to_string())
                .spawn(move || {
                    for job in queue {
                        let (reference, kind) = job.key;
                        let result = loader(reference, kind)
                            .map_err(|e| e.to_string())
                            .and_then(|image| image.map(decode).transpose());
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

/// Décode le JPEG reçu en image egui (RGBA).
fn decode(image: ArtImage) -> Result<Decoded, String> {
    let decoded = image::load_from_memory_with_format(&image.bytes, image::ImageFormat::Jpeg)
        .map_err(|e| format!("JPEG illisible : {e}"))?;
    if decoded.width() > MAX_SIDE || decoded.height() > MAX_SIDE {
        return Err("image trop grande".to_string());
    }
    let rgba = decoded.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(Decoded {
        image: ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        attribution: image.attribution,
        attribution_url: image.attribution_url,
    })
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
