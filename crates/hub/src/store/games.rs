//! Partie du stockage propre aux parties hébergées par le hub (ADR 0052).
//!
//! La base ne garde que la description de chaque partie : à qui elle est, son port,
//! son nom, et ses mots de passe **chiffrés** (voir `games::secret`). La seed et la
//! sauvegarde restent dans des fichiers, sous `<données du hub>/parties/<id>/`.
//!
//! Ce module est un enfant de `store` : il voit le pool et la méthode `q`.

use sqlx::any::AnyRow;
use sqlx::Row;

use super::Store;
use crate::error::HubError;

/// Colonnes lues par toutes les requêtes de ce module, dans cet ordre.
const COLUMNS: &str = "id, owner_id, name, port, seed_name, player_count, seed_size, \
                       password_enc, admin_password_enc, created_at";

/// Une partie hébergée, telle que la base la garde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRow {
    /// Identifiant opaque (32 chiffres hexadécimaux), donné aux joueurs.
    pub id: String,
    /// Compte de l'hôte.
    pub owner_id: i64,
    /// Nom choisi par l'hôte.
    pub name: String,
    /// Port de la plage réservée aux parties, gardé tant que la partie existe.
    pub port: i64,
    pub seed_name: String,
    /// Joueurs de la seed (sans les groupes).
    pub player_count: i64,
    /// Taille du fichier `.archipelago`, en octets.
    pub seed_size: i64,
    /// Mot de passe de la partie, chiffré (`None` : pas de mot de passe).
    pub password_enc: Option<String>,
    /// Mot de passe d'admin (`!admin`), chiffré.
    pub admin_password_enc: String,
    pub created_at: i64,
}

fn game_from_row(row: &AnyRow) -> Result<GameRow, HubError> {
    Ok(GameRow {
        id: row.try_get("id")?,
        owner_id: row.try_get("owner_id")?,
        name: row.try_get("name")?,
        port: row.try_get("port")?,
        seed_name: row.try_get("seed_name")?,
        player_count: row.try_get("player_count")?,
        seed_size: row.try_get("seed_size")?,
        password_enc: row.try_get("password_enc")?,
        admin_password_enc: row.try_get("admin_password_enc")?,
        created_at: row.try_get("created_at")?,
    })
}

impl Store {
    /// Enregistre une nouvelle partie.
    pub async fn insert_game(&self, game: &GameRow) -> Result<(), HubError> {
        self.q(concat!(
            "INSERT INTO hosted_games (id, owner_id, name, port, seed_name, player_count,",
            " seed_size, password_enc, admin_password_enc, created_at)",
            " VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        ))
        .bind(&game.id)
        .bind(game.owner_id)
        .bind(&game.name)
        .bind(game.port)
        .bind(&game.seed_name)
        .bind(game.player_count)
        .bind(game.seed_size)
        .bind(game.password_enc.as_deref())
        .bind(&game.admin_password_enc)
        .bind(game.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Une partie, d'après son identifiant.
    pub async fn game(&self, id: &str) -> Result<Option<GameRow>, HubError> {
        let sql = format!("SELECT {COLUMNS} FROM hosted_games WHERE id = $1");
        let row = self.q(&sql).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(game_from_row).transpose()
    }

    /// Toutes les parties du hub, des plus anciennes aux plus récentes (reprise au
    /// démarrage).
    pub async fn all_games(&self) -> Result<Vec<GameRow>, HubError> {
        let sql = format!("SELECT {COLUMNS} FROM hosted_games ORDER BY created_at, id");
        let rows = self.q(&sql).fetch_all(&self.pool).await?;
        rows.iter().map(game_from_row).collect()
    }

    /// Les parties d'un compte, des plus anciennes aux plus récentes.
    pub async fn games_of(&self, owner_id: i64) -> Result<Vec<GameRow>, HubError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM hosted_games WHERE owner_id = $1 ORDER BY created_at, id"
        );
        let rows = self.q(&sql).bind(owner_id).fetch_all(&self.pool).await?;
        rows.iter().map(game_from_row).collect()
    }

    /// Nombre de parties : de tout le hub (`None`) ou d'un compte.
    pub async fn count_games(&self, owner_id: Option<i64>) -> Result<i64, HubError> {
        let row = match owner_id {
            Some(owner) => {
                self.q("SELECT COUNT(*) AS n FROM hosted_games WHERE owner_id = $1")
                    .bind(owner)
                    .fetch_one(&self.pool)
                    .await?
            }
            None => {
                self.q("SELECT COUNT(*) AS n FROM hosted_games")
                    .fetch_one(&self.pool)
                    .await?
            }
        };
        Ok(row.try_get("n")?)
    }

    /// Ports déjà réservés par une partie (même arrêtée ou en erreur).
    pub async fn game_ports(&self) -> Result<Vec<i64>, HubError> {
        let rows = self
            .q("SELECT port FROM hosted_games ORDER BY port")
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| row.try_get("port").map_err(HubError::from))
            .collect()
    }

    /// Supprime une partie. `false` si elle n'existait pas.
    pub async fn delete_game(&self, id: &str) -> Result<bool, HubError> {
        let result = self
            .q("DELETE FROM hosted_games WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
