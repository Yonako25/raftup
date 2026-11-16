//! Format de sauvegarde natif `.raftup` (JSON lignes, versionné).
//!
//! Une sauvegarde contient l'état *dynamique* d'une partie (lieux cochés, objets
//! reçus, statuts, indices, alias, stockage de données), jamais la seed : seule
//! son empreinte SHA-256 est notée, pour refuser de reprendre une sauvegarde avec
//! une autre seed. Aucun mot de passe n'est écrit.
//!
//! ```text
//! {"format_version":1,"kind":"raftup-save","raftup_version":"0.1.0","seed_name":"AP_123","seed_sha256":"ab12…","created":1760000000.5}
//! {"type":"snapshot","location_checks":[],"received_items":[],"client_status":[],"hints":[],"hints_used":[],"aliases":[],"group_collected":[],"store":[],"activity":[]}
//! {"type":"checks","team":0,"slot":1,"locations":[100,101]}
//! {"type":"alias","team":0,"slot":1,"alias":"Zoé"}
//! ```
//!
//! - ligne 1 : en-tête ([`SaveHeader`]) ;
//! - ligne 2 : instantané ([`Snapshot`]), état au dernier compactage ;
//! - lignes suivantes : un [`Record`] (delta) par ligne, ajouté à la fin.
//!
//! # Fiabilité (NF07)
//!
//! Chaque ajout est suivi d'un `sync_data()` : une coupure de courant perd au
//! plus la ligne en cours d'écriture. À la réouverture, une dernière ligne
//! tronquée ou invalide est ignorée (et coupée du fichier), alors qu'une ligne
//! invalide au milieu est une corruption et refuse l'ouverture. Le compactage
//! écrit `<fichier>.tmp` puis le renomme par-dessus l'original : le remplacement
//! est atomique, et un `.tmp` resté après une interruption est simplement ignoré.
//!
//! Choix consigné dans l'ADR 0014.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use ap_protocol::{Hint, NetworkItem};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Version du format écrite par ce code.
pub const FORMAT_VERSION: u32 = 1;
/// Valeur du champ `kind` d'une sauvegarde Raftup.
pub const SAVE_KIND: &str = "raftup-save";

/// Erreurs de lecture et d'écriture d'une sauvegarde.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("erreur d'entrée/sortie sur {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("JSON invalide à la ligne {line} : {source}")]
    Json {
        line: usize,
        source: serde_json::Error,
    },
    #[error("ce fichier n'est pas une sauvegarde Raftup")]
    NotASave,
    #[error("version de format {found} non prise en charge (maximum {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("la sauvegarde appartient à une autre seed")]
    SeedMismatch,
    #[error("sauvegarde corrompue à la ligne {line}")]
    Corrupted { line: usize },
}

/// Empreinte SHA-256 en hexadécimal minuscule.
pub fn seed_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// En-tête (ligne 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveHeader {
    pub format_version: u32,
    pub kind: String,
    pub raftup_version: String,
    pub seed_name: String,
    pub seed_sha256: String,
    /// Date de création, en secondes Unix.
    pub created: f64,
}

impl SaveHeader {
    /// En-tête courant (version du format et `kind` remplis).
    pub fn new(
        raftup_version: &str,
        seed_name: &str,
        seed_sha256: &str,
        created: f64,
    ) -> SaveHeader {
        SaveHeader {
            format_version: FORMAT_VERSION,
            kind: SAVE_KIND.to_string(),
            raftup_version: raftup_version.to_string(),
            seed_name: seed_name.to_string(),
            seed_sha256: seed_sha256.to_string(),
            created,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotChecks {
    pub team: u32,
    pub slot: u32,
    pub locations: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotItems {
    pub team: u32,
    pub slot: u32,
    pub remote: bool,
    pub items: Vec<NetworkItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotStatusEntry {
    pub team: u32,
    pub slot: u32,
    pub status: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotHints {
    pub team: u32,
    pub slot: u32,
    pub hints: Vec<Hint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotHintsUsed {
    pub team: u32,
    pub slot: u32,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotAlias {
    pub team: u32,
    pub slot: u32,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupCollectedEntry {
    pub group: u32,
    pub member: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreEntry {
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotActivity {
    pub team: u32,
    pub slot: u32,
    pub time: f64,
}

/// État complet de la partie (ligne 2). L'ordre des listes est conservé.
// `#[serde(tag = "type", rename = "snapshot")]` sur une structure : serde écrit
// un champ `"type":"snapshot"` en tête, et refuse une ligne qui porte un autre type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "snapshot")]
pub struct Snapshot {
    #[serde(default)]
    pub location_checks: Vec<SlotChecks>,
    #[serde(default)]
    pub received_items: Vec<SlotItems>,
    #[serde(default)]
    pub client_status: Vec<SlotStatusEntry>,
    #[serde(default)]
    pub hints: Vec<SlotHints>,
    #[serde(default)]
    pub hints_used: Vec<SlotHintsUsed>,
    #[serde(default)]
    pub aliases: Vec<SlotAlias>,
    #[serde(default)]
    pub group_collected: Vec<GroupCollectedEntry>,
    #[serde(default)]
    pub store: Vec<StoreEntry>,
    #[serde(default)]
    pub activity: Vec<SlotActivity>,
}

/// Un delta : un changement d'état à ajouter à la fin du fichier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Checks {
        team: u32,
        slot: u32,
        locations: Vec<i64>,
    },
    /// Objets AJOUTÉS (à concaténer à ceux déjà reçus).
    Items {
        team: u32,
        slot: u32,
        remote: bool,
        items: Vec<NetworkItem>,
    },
    Status {
        team: u32,
        slot: u32,
        status: u8,
    },
    /// Liste complète des indices du slot (remplace l'ancienne).
    Hints {
        team: u32,
        slot: u32,
        hints: Vec<Hint>,
    },
    HintsUsed {
        team: u32,
        slot: u32,
        total: u32,
    },
    /// `None` retire l'alias.
    Alias {
        team: u32,
        slot: u32,
        alias: Option<String>,
    },
    GroupCollected {
        group: u32,
        member: u32,
    },
    Store {
        key: String,
        value: serde_json::Value,
    },
}

impl Snapshot {
    /// Applique un delta, sans aucune logique de jeu : seulement de la mise à jour de données.
    pub fn apply(&mut self, record: &Record) {
        match record {
            Record::Checks {
                team,
                slot,
                locations,
            } => {
                let pos = match self
                    .location_checks
                    .iter()
                    .position(|e| e.team == *team && e.slot == *slot)
                {
                    Some(i) => i,
                    None => {
                        self.location_checks.push(SlotChecks {
                            team: *team,
                            slot: *slot,
                            locations: Vec::new(),
                        });
                        self.location_checks.len() - 1
                    }
                };
                let entry = &mut self.location_checks[pos];
                entry.locations.extend_from_slice(locations);
                entry.locations.sort_unstable();
                entry.locations.dedup();
            }
            Record::Items {
                team,
                slot,
                remote,
                items,
            } => {
                // Une liste par (équipe, slot, remote) : les deux sens sont séparés.
                let pos = match self
                    .received_items
                    .iter()
                    .position(|e| e.team == *team && e.slot == *slot && e.remote == *remote)
                {
                    Some(i) => i,
                    None => {
                        self.received_items.push(SlotItems {
                            team: *team,
                            slot: *slot,
                            remote: *remote,
                            items: Vec::new(),
                        });
                        self.received_items.len() - 1
                    }
                };
                self.received_items[pos].items.extend_from_slice(items);
            }
            Record::Status { team, slot, status } => {
                match self
                    .client_status
                    .iter_mut()
                    .find(|e| e.team == *team && e.slot == *slot)
                {
                    Some(e) => e.status = *status,
                    None => self.client_status.push(SlotStatusEntry {
                        team: *team,
                        slot: *slot,
                        status: *status,
                    }),
                }
            }
            Record::Hints { team, slot, hints } => {
                match self
                    .hints
                    .iter_mut()
                    .find(|e| e.team == *team && e.slot == *slot)
                {
                    Some(e) => e.hints = hints.clone(),
                    None => self.hints.push(SlotHints {
                        team: *team,
                        slot: *slot,
                        hints: hints.clone(),
                    }),
                }
            }
            Record::HintsUsed { team, slot, total } => {
                match self
                    .hints_used
                    .iter_mut()
                    .find(|e| e.team == *team && e.slot == *slot)
                {
                    Some(e) => e.count = *total,
                    None => self.hints_used.push(SlotHintsUsed {
                        team: *team,
                        slot: *slot,
                        count: *total,
                    }),
                }
            }
            Record::Alias { team, slot, alias } => {
                let pos = self
                    .aliases
                    .iter()
                    .position(|e| e.team == *team && e.slot == *slot);
                match (pos, alias) {
                    (Some(i), Some(a)) => self.aliases[i].alias = a.clone(),
                    (Some(i), None) => {
                        self.aliases.remove(i);
                    }
                    (None, Some(a)) => self.aliases.push(SlotAlias {
                        team: *team,
                        slot: *slot,
                        alias: a.clone(),
                    }),
                    (None, None) => {}
                }
            }
            Record::GroupCollected { group, member } => {
                let entry = GroupCollectedEntry {
                    group: *group,
                    member: *member,
                };
                if !self.group_collected.contains(&entry) {
                    self.group_collected.push(entry);
                }
            }
            Record::Store { key, value } => match self.store.iter_mut().find(|e| e.key == *key) {
                Some(e) => e.value = value.clone(),
                None => self.store.push(StoreEntry {
                    key: key.clone(),
                    value: value.clone(),
                }),
            },
        }
    }
}

/// Fichier de sauvegarde ouvert en ajout.
#[derive(Debug)]
pub struct SaveFile {
    path: PathBuf,
    header: SaveHeader,
    file: File,
    lines_since_compact: usize,
    ignored_tail: bool,
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> SaveError + '_ {
    move |source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Chemin du fichier temporaire de compactage : `<path>.tmp`.
fn tmp_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

/// Écrit en-tête + instantané dans `path` (écrase), puis synchronise sur le disque.
fn write_base(path: &Path, header: &SaveHeader, snapshot: &Snapshot) -> Result<(), SaveError> {
    let mut text =
        serde_json::to_string(header).map_err(|source| SaveError::Json { line: 1, source })?;
    text.push('\n');
    text.push_str(
        &serde_json::to_string(snapshot).map_err(|source| SaveError::Json { line: 2, source })?,
    );
    text.push('\n');
    let mut f = File::create(path).map_err(io_err(path))?;
    f.write_all(text.as_bytes()).map_err(io_err(path))?;
    f.sync_all().map_err(io_err(path))
}

fn open_append(path: &Path) -> Result<File, SaveError> {
    OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(io_err(path))
}

impl SaveFile {
    /// Crée (ou écrase) une sauvegarde avec un instantané vide.
    pub fn create(path: &Path, header: SaveHeader) -> Result<SaveFile, SaveError> {
        write_base(path, &header, &Snapshot::default())?;
        Ok(SaveFile {
            path: path.to_path_buf(),
            header,
            file: open_append(path)?,
            lines_since_compact: 0,
            ignored_tail: false,
        })
    }

    /// Ouvre une sauvegarde, vérifie l'empreinte de la seed et reconstruit l'état.
    pub fn open(
        path: &Path,
        expected_seed_sha256: &str,
    ) -> Result<(SaveFile, Snapshot), SaveError> {
        let text = std::fs::read_to_string(path).map_err(io_err(path))?;
        // `split_inclusive` garde le `\n` : on peut ainsi calculer l'offset exact
        // de la fin de la dernière bonne ligne.
        let lines: Vec<&str> = text.split_inclusive('\n').collect();

        let first = lines.first().ok_or(SaveError::NotASave)?;
        let value: serde_json::Value =
            serde_json::from_str(first.trim()).map_err(|_| SaveError::NotASave)?;
        if value.get("kind").and_then(|k| k.as_str()) != Some(SAVE_KIND) {
            return Err(SaveError::NotASave);
        }
        // Vérifié avant la désérialisation complète : une version future peut
        // avoir un en-tête de forme différente.
        let version = value
            .get("format_version")
            .and_then(|v| v.as_u64())
            .ok_or(SaveError::NotASave)?;
        if version > u64::from(FORMAT_VERSION) {
            return Err(SaveError::UnsupportedVersion {
                found: u32::try_from(version).unwrap_or(u32::MAX),
                supported: FORMAT_VERSION,
            });
        }
        let header: SaveHeader =
            serde_json::from_value(value).map_err(|source| SaveError::Json { line: 1, source })?;
        if header.seed_sha256 != expected_seed_sha256 {
            return Err(SaveError::SeedMismatch);
        }

        let second = lines.get(1).ok_or(SaveError::Corrupted { line: 2 })?;
        let mut snapshot: Snapshot =
            serde_json::from_str(second.trim()).map_err(|_| SaveError::Corrupted { line: 2 })?;

        let mut good_len: usize = lines[0].len() + lines[1].len();
        let mut count = 0usize;
        let mut ignored_tail = false;
        for (i, raw) in lines.iter().enumerate().skip(2) {
            let line = raw.trim();
            if line.is_empty() {
                good_len += raw.len();
                continue;
            }
            match serde_json::from_str::<Record>(line) {
                Ok(record) => {
                    snapshot.apply(&record);
                    count += 1;
                    good_len += raw.len();
                }
                // Dernière ligne : écriture interrompue, on l'ignore.
                Err(_) if i + 1 == lines.len() => ignored_tail = true,
                Err(_) => return Err(SaveError::Corrupted { line: i + 1 }),
            }
        }

        // On remet le fichier dans un état propre avant d'ajouter : sans cela, la
        // prochaine ligne serait collée à la ligne tronquée.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(io_err(path))?;
        if ignored_tail {
            file.set_len(good_len as u64).map_err(io_err(path))?;
        }
        drop(file);
        let mut file = open_append(path)?;
        if !ignored_tail && !text.ends_with('\n') {
            file.write_all(b"\n").map_err(io_err(path))?;
        }

        Ok((
            SaveFile {
                path: path.to_path_buf(),
                header,
                file,
                lines_since_compact: count,
                ignored_tail,
            },
            snapshot,
        ))
    }

    /// Ajoute des deltas puis force l'écriture sur le disque.
    pub fn append(&mut self, records: &[Record]) -> Result<(), SaveError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for r in records {
            text.push_str(
                &serde_json::to_string(r).map_err(|source| SaveError::Json { line: 0, source })?,
            );
            text.push('\n');
        }
        self.file
            .write_all(text.as_bytes())
            .map_err(io_err(&self.path))?;
        self.file.sync_data().map_err(io_err(&self.path))?;
        self.lines_since_compact += records.len();
        Ok(())
    }

    /// Remplace le fichier par en-tête + `snapshot` (atomique : fichier temporaire puis renommage).
    pub fn compact(&mut self, snapshot: &Snapshot) -> Result<(), SaveError> {
        let tmp = tmp_path(&self.path);
        write_base(&tmp, &self.header, snapshot)?;
        std::fs::rename(&tmp, &self.path).map_err(io_err(&self.path))?;
        self.file = open_append(&self.path)?;
        self.lines_since_compact = 0;
        Ok(())
    }

    /// Nombre de deltas écrits depuis le dernier instantané (pour décider de compacter).
    pub fn lines_since_compact(&self) -> usize {
        self.lines_since_compact
    }

    /// Vrai si `open` a ignoré une dernière ligne tronquée ou invalide (à journaliser).
    pub fn ignored_tail(&self) -> bool {
        self.ignored_tail
    }

    pub fn header(&self) -> &SaveHeader {
        &self.header
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
