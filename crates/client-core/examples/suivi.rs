//! Suivi d'une partie en ligne de commande (jalon J0).
//!
//! Se connecte à une salle, affiche les joueurs, les objets reçus et la
//! progression, puis les messages du serveur au fil de l'eau. Ctrl+C pour quitter.
//!
//! ```text
//! cargo run -p client-core --example suivi -- <adresse> <slot> [mot_de_passe]
//! cargo run -p client-core --example suivi -- archipelago.gg:38281 Rancher1
//! ```
//!
//! Le client se connecte comme un simple observateur (tag `TextOnly`, sans jeu) :
//! il ne peut pas valider de lieu à la place du joueur.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use ap_protocol::client::GetDataPackage;
use ap_protocol::server::{DataPackage, JsonMessagePart, PrintJson, ReceivedItems};
use ap_protocol::{ClientPacket, ItemId, LocationId, NetworkPlayer, ServerPacket, SlotId};
use client_core::{Client, ConnectOptions};

/// Les noms à afficher à la place des numéros.
#[derive(Default)]
struct Names {
    /// Nom affiché de chaque joueur.
    players: BTreeMap<SlotId, String>,
    /// Jeu de chaque slot.
    games: BTreeMap<SlotId, String>,
    /// Pour chaque jeu : numéro d'objet → nom.
    items: BTreeMap<String, BTreeMap<ItemId, String>>,
    /// Pour chaque jeu : numéro de lieu → nom.
    locations: BTreeMap<String, BTreeMap<LocationId, String>>,
}

impl Names {
    fn set_players(&mut self, players: &[NetworkPlayer]) {
        for p in players {
            self.players.insert(p.slot, p.alias.clone());
        }
    }

    fn add_data_package(&mut self, package: &DataPackage) {
        for (game, data) in &package.data.games {
            // On inverse les tables : le DataPackage donne nom → numéro.
            let items = data
                .item_name_to_id
                .iter()
                .map(|(name, id)| (*id, name.clone()))
                .collect();
            let locations = data
                .location_name_to_id
                .iter()
                .map(|(name, id)| (*id, name.clone()))
                .collect();
            self.items.insert(game.clone(), items);
            self.locations.insert(game.clone(), locations);
        }
    }

    fn player(&self, slot: SlotId) -> String {
        match self.players.get(&slot) {
            Some(name) => name.clone(),
            None if slot == 0 => "le serveur".to_string(),
            None => format!("joueur {slot}"),
        }
    }

    /// Cherche un nom dans la table du jeu du slot, puis dans celle
    /// d'« Archipelago » (lieux spéciaux comme l'inventaire de départ).
    fn lookup(
        &self,
        tables: &BTreeMap<String, BTreeMap<i64, String>>,
        slot: SlotId,
        id: i64,
    ) -> Option<String> {
        let game = self.games.get(&slot).map(String::as_str).unwrap_or("");
        [game, "Archipelago"]
            .iter()
            .find_map(|g| tables.get(*g).and_then(|t| t.get(&id)).cloned())
    }

    /// Nom d'un objet ; `owner` est le joueur dont le jeu définit l'objet.
    fn item(&self, owner: SlotId, id: ItemId) -> String {
        self.lookup(&self.items, owner, id)
            .unwrap_or_else(|| format!("objet {id}"))
    }

    /// Nom d'un lieu ; `owner` est le joueur dont le monde contient le lieu.
    fn location(&self, owner: SlotId, id: LocationId) -> String {
        self.lookup(&self.locations, owner, id)
            .unwrap_or_else(|| format!("lieu {id}"))
    }

    /// Texte d'un message `PrintJSON`, avec les numéros remplacés par des noms.
    fn render(&self, print: &PrintJson) -> String {
        print
            .data
            .iter()
            .map(|part| self.render_part(part))
            .collect()
    }

    fn render_part(&self, part: &JsonMessagePart) -> String {
        let text = part.text.clone().unwrap_or_default();
        let owner = part.player.unwrap_or(0);
        // Le numéro est dans `text`, sous forme de texte.
        let number = text.parse::<i64>();
        match (part.kind.as_deref(), number) {
            (Some("player_id"), Ok(slot)) => self.player(slot as SlotId),
            (Some("item_id"), Ok(id)) => self.item(owner, id),
            (Some("location_id"), Ok(id)) => self.location(owner, id),
            _ => text,
        }
    }
}

/// État de la partie suivi par le programme.
struct Game {
    names: Names,
    slot: SlotId,
    checked: BTreeSet<LocationId>,
    total_locations: usize,
    /// Nombre d'objets déjà reçus (et affichés).
    received: usize,
}

impl Game {
    fn print_progress(&self) {
        println!(
            "Progression : {} / {} lieux visités",
            self.checked.len(),
            self.total_locations
        );
    }

    fn on_received_items(&mut self, received: &ReceivedItems) {
        let start = received.index as usize;
        if start == 0 && self.received > 0 {
            // Liste complète renvoyée par le serveur (après un `Sync`) : on ne
            // réaffiche que les nouveaux objets.
            tracing::debug!("liste complète des objets reçue");
        }
        for (offset, item) in received.items.iter().enumerate() {
            if start + offset < self.received {
                continue;
            }
            // Un objet reçu appartient à notre jeu ; le lieu est dans le monde
            // de celui qui l'a trouvé (`item.player`).
            let name = self.names.item(self.slot, item.item);
            if item.player == 0 {
                // Slot 0 = le serveur : inventaire de départ ou objet donné par commande.
                println!("Reçu : {name}  (au départ)");
            } else {
                println!(
                    "Reçu : {name}  (de {}, {})",
                    self.names.player(item.player),
                    self.names.location(item.player, item.location)
                );
            }
        }
        self.received = self.received.max(start + received.items.len());
    }

    fn on_packet(&mut self, packet: ServerPacket) {
        match packet {
            ServerPacket::ReceivedItems(received) => self.on_received_items(&received),
            ServerPacket::PrintJson(print) => println!("» {}", self.names.render(&print)),
            ServerPacket::RoomUpdate(update) => {
                if let Some(players) = &update.players {
                    self.names.set_players(players);
                }
                if let Some(checked) = update.checked_locations {
                    let before = self.checked.len();
                    self.checked.extend(checked);
                    if self.checked.len() != before {
                        self.print_progress();
                    }
                }
            }
            ServerPacket::DataPackage(package) => self.names.add_data_package(&package),
            other => tracing::debug!(cmd = other.cmd(), "paquet ignoré"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Les avertissements du client (connexion non chiffrée…) s'affichent.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (address, slot) = match args.as_slice() {
        [address, slot, ..] => (address.clone(), slot.clone()),
        _ => bail!("usage : suivi <adresse> <slot> [mot_de_passe]"),
    };

    // Observateur : pas de jeu, tag `TextOnly` (comme le client texte officiel).
    let mut options = ConnectOptions::new(&address, &slot, "");
    options.tags = vec!["AP".to_string(), "TextOnly".to_string()];
    options.password = args.get(2).cloned();

    let (mut client, session) = Client::connect(&options)
        .await
        .with_context(|| format!("connexion à {address}"))?;
    let connected = &session.connected;
    println!(
        "Connecté à {} (seed {})",
        session.url, session.room.seed_name
    );

    // Les noms des objets et des lieux : on les demande pour tous les jeux.
    client
        .send(&[ClientPacket::GetDataPackage(GetDataPackage {
            games: Some(session.room.games.clone()),
        })])
        .await?;

    let mut game = Game {
        names: Names::default(),
        slot: connected.slot,
        checked: connected.checked_locations.iter().copied().collect(),
        total_locations: connected.checked_locations.len() + connected.missing_locations.len(),
        received: 0,
    };
    game.names.set_players(&connected.players);
    for (slot, info) in &connected.slot_info {
        game.names.games.insert(*slot, info.game.clone());
    }

    // On attend le DataPackage avant d'afficher quoi que ce soit, pour avoir
    // les noms. Les autres paquets sont gardés pour après.
    let mut waiting = Vec::new();
    loop {
        match client.recv().await? {
            Some(ServerPacket::DataPackage(package)) => {
                game.names.add_data_package(&package);
                break;
            }
            Some(other) => waiting.push(other),
            None => bail!("le serveur a fermé la connexion"),
        }
    }

    println!();
    println!("Joueurs :");
    for (slot, info) in &connected.slot_info {
        let marker = if *slot == connected.slot {
            "  ← toi"
        } else {
            ""
        };
        println!(
            "  {slot:>2}. {} ({}){marker}",
            game.names.player(*slot),
            info.game
        );
    }
    println!();
    game.print_progress();
    println!();

    for packet in waiting {
        game.on_packet(packet);
    }

    // Boucle principale : paquets du serveur, ou Ctrl+C.
    // `tokio::select!` attend la première des deux choses qui arrive.
    loop {
        tokio::select! {
            packet = client.recv() => match packet? {
                Some(packet) => game.on_packet(packet),
                None => {
                    println!("Le serveur a fermé la connexion.");
                    return Ok(());
                }
            },
            _ = tokio::signal::ctrl_c() => {
                println!("Au revoir.");
                client.close().await?;
                return Ok(());
            }
        }
    }
}
