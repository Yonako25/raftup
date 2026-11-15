//! Types des paquets du protocole réseau compatible avec Archipelago.
//!
//! Cette crate est la seule source de vérité du protocole (ADR 0004) : le serveur
//! et le client s'appuient dessus, dans les deux sens.
//!
//! - [`server`] : paquets envoyés par le serveur au client (étape 2) ;
//! - [`client`] : paquets envoyés par le client au serveur (étape 3).
//!
//! Référence : `docs/network protocol.md` du dépôt ArchipelagoMW/Archipelago.
//! Les tests s'appuient sur des paquets capturés auprès du serveur officiel
//! (`tests/captures/`).

pub mod client;
mod codec;
pub mod common;
pub mod error;
pub mod server;

// Raccourcis : `ap_protocol::ServerPacket` au lieu de `ap_protocol::server::ServerPacket`.
pub use client::{decode_client_frame, encode_client_frame, ClientPacket};
pub use common::{
    ClientStatus, HintStatus, ItemFlags, ItemId, ItemsHandling, LocationId, NetworkItem,
    NetworkPlayer, NetworkSlot, NetworkVersion, Permission, SlotId, SlotType, TeamId,
};
pub use error::ProtocolError;
pub use server::{decode_server_frame, encode_server_frame, ServerPacket};
