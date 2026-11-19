//! Stockage des comptes, sessions et amitiés (ADR 0002, 0033).
//!
//! SQLite pour les tests et le développement. Le SQL reste portable vers PostgreSQL
//! (`RETURNING`, pas de fonction propre à SQLite) ; seules les marques de paramètres
//! (`?` ici, `$1` en PostgreSQL) et le type des identifiants changeront. Le reste du
//! hub ne voit que les méthodes de [`Store`] : le jour du PostgreSQL, seul ce fichier
//! change (ADR 0033).

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::Row;

use crate::error::HubError;

/// Version du schéma de la base. À augmenter à chaque changement de schéma.
pub const SCHEMA_VERSION: i64 = 1;

/// Tables : le compte (`users`) est séparé de ses moyens de connexion
/// (`user_identities`), comme le demande l'ADR 0002. Les amitiés et les demandes
/// pointent vers l'identifiant du compte, jamais vers le pseudo.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL,
    username_norm TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS user_identities (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    secret_hash TEXT,
    verified INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (provider, subject)
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS friend_requests (
    from_user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    to_user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (from_user, to_user)
);
CREATE TABLE IF NOT EXISTS friendships (
    user_a INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b)
);
CREATE INDEX IF NOT EXISTS friend_requests_to ON friend_requests (to_user);
CREATE INDEX IF NOT EXISTS friendships_b ON friendships (user_b);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id)
";

/// Fournisseur de connexion « e-mail + mot de passe » dans `user_identities`.
const EMAIL_PROVIDER: &str = "email";

/// Un compte, vu de l'extérieur : identifiant interne et pseudo affiché.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
}

/// Accès à la base. `Clone` est bon marché : le pool est partagé derrière un compteur.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

/// Paire rangée pour la table `friendships` (`user_a < user_b`).
fn ordered(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

impl Store {
    /// Ouvre (ou crée) une base SQLite, par exemple `sqlite://raftup-hub.db`.
    pub async fn open(url: &str) -> Result<Store, HubError> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        Store::init(pool).await
    }

    /// Base en mémoire, pour les tests. Une seule connexion, jamais fermée :
    /// une base SQLite en mémoire disparaît avec la connexion qui l'a créée.
    pub async fn in_memory() -> Result<Store, HubError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await?;
        Store::init(pool).await
    }

    /// Crée les tables si besoin et vérifie la version du schéma.
    async fn init(pool: SqlitePool) -> Result<Store, HubError> {
        for statement in SCHEMA.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(statement).execute(&pool).await?;
        }
        let version: Option<i64> = sqlx::query("SELECT version FROM schema_version")
            .fetch_optional(&pool)
            .await?
            .map(|row| row.try_get("version"))
            .transpose()?;
        match version {
            None => {
                sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                    .bind(SCHEMA_VERSION)
                    .execute(&pool)
                    .await?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) => {
                return Err(HubError::Internal(format!(
                    "schéma de base en version {v}, ce hub connaît la version {SCHEMA_VERSION}"
                )))
            }
        }
        Ok(Store { pool })
    }

    /// Crée un compte et son identité « e-mail », dans une seule transaction.
    pub async fn create_account(
        &self,
        username: &str,
        username_norm: &str,
        email_norm: &str,
        password_hash: &str,
        now: i64,
    ) -> Result<i64, HubError> {
        let mut tx = self.pool.begin().await?;
        // `&mut *tx` : la transaction se comporte comme une connexion ; on la prête
        // à chaque requête sans la céder, pour pouvoir la valider à la fin.
        let user_id: i64 = sqlx::query(
            "INSERT INTO users (username, username_norm, created_at) VALUES (?, ?, ?) RETURNING id",
        )
        .bind(username)
        .bind(username_norm)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_unique)?
        .try_get("id")?;
        sqlx::query(
            "INSERT INTO user_identities (user_id, provider, subject, secret_hash, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(EMAIL_PROVIDER)
        .bind(email_norm)
        .bind(password_hash)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_unique)?;
        tx.commit().await?;
        Ok(user_id)
    }

    /// Compte et hash du mot de passe liés à une adresse e-mail normalisée.
    pub async fn email_identity(
        &self,
        email_norm: &str,
    ) -> Result<Option<(i64, String)>, HubError> {
        let row = sqlx::query(
            "SELECT user_id, secret_hash FROM user_identities WHERE provider = ? AND subject = ?",
        )
        .bind(EMAIL_PROVIDER)
        .bind(email_norm)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(Some((row.try_get("user_id")?, row.try_get("secret_hash")?))),
            None => Ok(None),
        }
    }

    /// Adresse e-mail d'un compte (s'il en a une).
    pub async fn email_of(&self, user_id: i64) -> Result<Option<String>, HubError> {
        let row = sqlx::query(
            "SELECT subject FROM user_identities WHERE user_id = ? AND provider = ? ORDER BY id",
        )
        .bind(user_id)
        .bind(EMAIL_PROVIDER)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| r.try_get("subject"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn user_by_id(&self, user_id: i64) -> Result<Option<UserRow>, HubError> {
        let row = sqlx::query("SELECT id, username FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| user_row(&r)).transpose()
    }

    pub async fn user_by_username(&self, username_norm: &str) -> Result<Option<UserRow>, HubError> {
        let row = sqlx::query("SELECT id, username FROM users WHERE username_norm = ?")
            .bind(username_norm)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| user_row(&r)).transpose()
    }

    pub async fn insert_session(
        &self,
        token_hash: &str,
        user_id: i64,
        now: i64,
        expires_at: i64,
    ) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Compte d'une session encore valide.
    pub async fn session_user(&self, token_hash: &str, now: i64) -> Result<Option<i64>, HubError> {
        let row =
            sqlx::query("SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > ?")
                .bind(token_hash)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|r| r.try_get("user_id"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), HubError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Retire les sessions expirées ; renvoie le nombre de lignes supprimées.
    pub async fn purge_expired_sessions(&self, now: i64) -> Result<u64, HubError> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn are_friends(&self, a: i64, b: i64) -> Result<bool, HubError> {
        let (a, b) = ordered(a, b);
        let row = sqlx::query("SELECT 1 FROM friendships WHERE user_a = ? AND user_b = ?")
            .bind(a)
            .bind(b)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn request_exists(&self, from: i64, to: i64) -> Result<bool, HubError> {
        let row = sqlx::query("SELECT 1 FROM friend_requests WHERE from_user = ? AND to_user = ?")
            .bind(from)
            .bind(to)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn insert_request(&self, from: i64, to: i64, now: i64) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO friend_requests (from_user, to_user, created_at) VALUES (?, ?, ?)",
        )
        .bind(from)
        .bind(to)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            // Deux demandes simultanées : la clé primaire refuse la seconde.
            if is_unique_violation(&e) {
                HubError::RequestExists
            } else {
                HubError::Database(e)
            }
        })?;
        Ok(())
    }

    /// Supprime une demande ; renvoie `false` si elle n'existait pas.
    pub async fn delete_request(&self, from: i64, to: i64) -> Result<bool, HubError> {
        let result = sqlx::query("DELETE FROM friend_requests WHERE from_user = ? AND to_user = ?")
            .bind(from)
            .bind(to)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Crée l'amitié et retire les demandes dans les deux sens, en une transaction.
    pub async fn make_friends(&self, a: i64, b: i64, now: i64) -> Result<(), HubError> {
        let (low, high) = ordered(a, b);
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM friend_requests WHERE (from_user = ? AND to_user = ?) \
             OR (from_user = ? AND to_user = ?)",
        )
        .bind(a)
        .bind(b)
        .bind(b)
        .bind(a)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO friendships (user_a, user_b, created_at) VALUES (?, ?, ?) \
             ON CONFLICT (user_a, user_b) DO NOTHING",
        )
        .bind(low)
        .bind(high)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Retire une amitié ; renvoie `false` si elle n'existait pas.
    pub async fn remove_friendship(&self, a: i64, b: i64) -> Result<bool, HubError> {
        let (a, b) = ordered(a, b);
        let result = sqlx::query("DELETE FROM friendships WHERE user_a = ? AND user_b = ?")
            .bind(a)
            .bind(b)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Amis d'un compte, rangés par pseudo normalisé (ordre stable).
    pub async fn friends_of(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM friendships f \
             JOIN users u ON u.id = CASE WHEN f.user_a = ? THEN f.user_b ELSE f.user_a END \
             WHERE f.user_a = ? OR f.user_b = ? \
             ORDER BY u.username_norm, u.id",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }

    /// Demandes reçues, de la plus ancienne à la plus récente.
    pub async fn incoming_requests(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM friend_requests r JOIN users u ON u.id = r.from_user \
             WHERE r.to_user = ? ORDER BY r.created_at, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }

    /// Demandes envoyées, de la plus ancienne à la plus récente.
    pub async fn outgoing_requests(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM friend_requests r JOIN users u ON u.id = r.to_user \
             WHERE r.from_user = ? ORDER BY r.created_at, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }
}

fn user_row(row: &sqlx::sqlite::SqliteRow) -> Result<UserRow, HubError> {
    Ok(UserRow {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
    })
}

/// Traduit une violation d'unicité en erreur lisible. Le message de SQLite nomme la
/// colonne en cause (`users.username_norm`, `user_identities.provider, ...`).
fn map_unique(error: sqlx::Error) -> HubError {
    if let sqlx::Error::Database(db) = &error {
        if db.is_unique_violation() {
            let message = db.message();
            if message.contains("username_norm") {
                return HubError::UsernameTaken;
            }
            if message.contains("user_identities") {
                return HubError::EmailTaken;
            }
        }
    }
    HubError::Database(error)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}
