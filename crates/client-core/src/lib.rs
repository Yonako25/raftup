//! Client du protocole réseau compatible avec Archipelago (ADR 0004), sur tokio.
//!
//! Utilisé par la CLI de J0, puis par le launcher et les tests du serveur.
//!
//! ```no_run
//! # async fn exemple() -> Result<(), client_core::ClientError> {
//! use client_core::{Client, ConnectOptions};
//!
//! let options = ConnectOptions::new("archipelago.gg:38281", "Rancher1", "Slime Rancher");
//! let (mut client, session) = Client::connect(&options).await?;
//! println!("connecté au slot {}", session.connected.slot);
//! while let Some(packet) = client.recv().await? {
//!     println!("reçu : {}", packet.cmd());
//! }
//! # Ok(())
//! # }
//! ```

mod address;
mod client;
mod error;
mod pin;

pub use address::{candidate_urls, DEFAULT_PORT};
pub use client::{Client, ConnectOptions, Session, CLIENT_VERSION, DEFAULT_MAX_MESSAGE_SIZE};
pub use error::ClientError;
pub use pin::Fingerprint;
