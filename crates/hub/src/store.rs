//! Stockage des comptes, sessions et amitiés (ADR 0002, 0033).
//!
//! Deux bases possibles, avec **le même code** : SQLite (tests, développement) et
//! PostgreSQL (production). On passe par le pilote générique `sqlx::Any`, qui choisit
//! la base d'après le début de l'URL (`sqlite:` ou `postgres:`).
//!
//! Pour que le même SQL marche partout :
//! - les paramètres s'écrivent `$1`, `$2`… (PostgreSQL les exige, SQLite les accepte) ;
//! - les entiers sont des `BIGINT` (`i64` côté Rust) ;
//! - seule la colonne d'identifiant auto-incrémenté diffère ([`Backend::id_column`]).
//!
//! Le reste du hub ne voit que les méthodes de [`Store`].

use serde::Serialize;
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Row};

use crate::error::HubError;

/// Version du schéma de la base. À augmenter à chaque changement de schéma,
/// en ajoutant la migration correspondante dans [`MIGRATIONS`].
pub const SCHEMA_VERSION: i64 = 4;

/// Tables : le compte (`users`) est séparé de ses moyens de connexion
/// (`user_identities`), comme le demande l'ADR 0002. Les amitiés et les demandes
/// pointent vers l'identifiant du compte, jamais vers le pseudo.
///
/// `{id}` est remplacé par la colonne d'identifiant propre à chaque base.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS users (
    id {id},
    username TEXT NOT NULL,
    username_norm TEXT NOT NULL UNIQUE,
    created_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS user_identities (
    id {id},
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    subject TEXT NOT NULL,
    secret_hash TEXT,
    verified BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    UNIQUE (provider, subject)
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS friend_requests (
    from_user BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    to_user BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (from_user, to_user)
);
CREATE TABLE IF NOT EXISTS friendships (
    user_a BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    user_b BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b)
);
CREATE TABLE IF NOT EXISTS blocks (
    blocker BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    blocked BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (blocker, blocked),
    CHECK (blocker <> blocked)
);
CREATE INDEX IF NOT EXISTS blocks_blocked ON blocks (blocked);
CREATE TABLE IF NOT EXISTS email_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS email_tokens_user ON email_tokens (user_id);
CREATE UNIQUE INDEX IF NOT EXISTS user_identities_one_per_provider
    ON user_identities (user_id, provider);
CREATE INDEX IF NOT EXISTS friend_requests_to ON friend_requests (to_user);
CREATE INDEX IF NOT EXISTS friendships_b ON friendships (user_b);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id)
";

/// Migrations d'une version à la suivante : `(version atteinte, instructions)`.
/// Les tables nouvelles sont déjà créées par [`SCHEMA`] (`IF NOT EXISTS`) ; une
/// migration ne contient que ce que `SCHEMA` ne sait pas faire (ajout de colonne…).
const MIGRATIONS: &[(i64, &str)] = &[
    // Version 2 : table `blocks`, créée par `SCHEMA`.
    (2, ""),
    // Version 3 : table `email_tokens`, créée par `SCHEMA`.
    (3, ""),
    // Version 4 : une seule identité par fournisseur et par compte (index de `SCHEMA`).
    (4, ""),
];

/// Fournisseur de connexion « e-mail + mot de passe » dans `user_identities`.
const EMAIL_PROVIDER: &str = "email";
/// Fournisseur « Discord » : `subject` est l'identifiant Discord du joueur.
pub const DISCORD_PROVIDER: &str = "discord";

/// Un compte, vu de l'extérieur : identifiant interne et pseudo affiché.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
}

/// Usage d'un jeton envoyé par e-mail. Un jeton ne sert qu'à son usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPurpose {
    VerifyEmail,
    ResetPassword,
}

impl TokenPurpose {
    fn as_str(self) -> &'static str {
        match self {
            TokenPurpose::VerifyEmail => "verify_email",
            TokenPurpose::ResetPassword => "reset_password",
        }
    }
}

/// Moyen de connexion, tel que l'export RGPD le montre (jamais le hash du secret).
#[derive(Clone, Debug, Serialize)]
pub struct IdentityExport {
    pub provider: String,
    pub subject: String,
    pub verified: bool,
    pub created_at: i64,
}

/// Session ouverte, sans son empreinte.
#[derive(Clone, Debug, Serialize)]
pub struct SessionExport {
    pub created_at: i64,
    pub expires_at: i64,
}

/// Un autre joueur et la date du lien (amitié, demande, blocage).
#[derive(Clone, Debug, Serialize)]
pub struct LinkExport {
    pub username: String,
    pub since: i64,
}

/// Tout ce que la base garde sur un compte (RGPD, droit d'accès).
#[derive(Clone, Debug, Serialize)]
pub struct AccountExport {
    pub user_id: i64,
    pub username: String,
    pub created_at: i64,
    pub identities: Vec<IdentityExport>,
    pub sessions: Vec<SessionExport>,
    pub friends: Vec<LinkExport>,
    pub requests_received: Vec<LinkExport>,
    pub requests_sent: Vec<LinkExport>,
    pub blocked: Vec<LinkExport>,
}

/// La base derrière le pilote générique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl Backend {
    /// Reconnaît la base d'après le début de l'URL.
    pub fn from_url(url: &str) -> Result<Backend, HubError> {
        if url.starts_with("sqlite:") {
            Ok(Backend::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Backend::Postgres)
        } else {
            Err(HubError::Internal(
                "URL de base inconnue : attendu sqlite:… ou postgres://…".to_string(),
            ))
        }
    }

    /// Colonne d'identifiant auto-incrémenté. En SQLite, `INTEGER PRIMARY KEY` (et
    /// pas `BIGINT`) est le seul type qui donne un identifiant automatique.
    fn id_column(self) -> &'static str {
        match self {
            Backend::Sqlite => "INTEGER PRIMARY KEY",
            Backend::Postgres => "BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY",
        }
    }
}

/// Accès à la base. `Clone` est bon marché : le pool est partagé derrière un compteur.
#[derive(Clone)]
pub struct Store {
    pool: AnyPool,
    backend: Backend,
}

/// Paire rangée pour la table `friendships` (`user_a < user_b`).
fn ordered(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Récrit une URL SQLite pour le pilote générique.
///
/// Le pilote `Any` lit l'URL avec la crate `url` avant de la passer à SQLite. Une
/// forme `sqlite://C:\dossier\hub.db` y serait lue comme un nom d'hôte « C » suivi
/// d'un port invalide ; la forme équivalente `sqlite:C:\dossier\hub.db` passe telle
/// quelle. On ajoute aussi `mode=rwc` (création du fichier s'il manque).
fn sqlite_url(url: &str) -> String {
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    if rest == ":memory:" || rest.contains("mode=") {
        return format!("sqlite:{rest}");
    }
    let separator = if rest.contains('?') { '&' } else { '?' };
    format!("sqlite:{rest}{separator}mode=rwc")
}

impl Store {
    /// Ouvre (ou crée) une base : `sqlite://raftup-hub.db` ou
    /// `postgres://utilisateur:motdepasse@hote/base`.
    pub async fn open(url: &str) -> Result<Store, HubError> {
        // Le pilote `Any` doit connaître les pilotes compilés ; l'appel est sans
        // effet après le premier.
        sqlx::any::install_default_drivers();
        let backend = Backend::from_url(url)?;
        let pool = match backend {
            Backend::Sqlite => {
                let pool = AnyPoolOptions::new()
                    .max_connections(5)
                    .connect(&sqlite_url(url))
                    .await?;
                // WAL : lecteurs et écrivain ne se bloquent pas ; le réglage reste
                // inscrit dans le fichier. Les clés étrangères sont activées par sqlx
                // sur chaque connexion SQLite.
                sqlx::query("PRAGMA journal_mode = WAL")
                    .fetch_optional(&pool)
                    .await?;
                pool
            }
            Backend::Postgres => {
                AnyPoolOptions::new()
                    .max_connections(10)
                    .connect(url)
                    .await?
            }
        };
        Store::init(pool, backend).await
    }

    /// Base SQLite en mémoire, pour les tests. Une seule connexion, jamais fermée :
    /// une base SQLite en mémoire disparaît avec la connexion qui l'a créée.
    pub async fn in_memory() -> Result<Store, HubError> {
        sqlx::any::install_default_drivers();
        let pool = AnyPoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect("sqlite::memory:")
            .await?;
        Store::init(pool, Backend::Sqlite).await
    }

    /// La base utilisée.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Crée les tables si besoin, applique les migrations et vérifie la version.
    async fn init(pool: AnyPool, backend: Backend) -> Result<Store, HubError> {
        let schema = SCHEMA.replace("{id}", backend.id_column());
        for statement in schema.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(statement).execute(&pool).await?;
        }
        let version: Option<i64> = sqlx::query("SELECT version FROM schema_version")
            .fetch_optional(&pool)
            .await?
            .map(|row| row.try_get("version"))
            .transpose()?;
        match version {
            None => {
                sqlx::query("INSERT INTO schema_version (version) VALUES ($1)")
                    .bind(SCHEMA_VERSION)
                    .execute(&pool)
                    .await?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) if v < SCHEMA_VERSION => {
                for (target, statements) in MIGRATIONS.iter().filter(|(t, _)| *t > v) {
                    let mut tx = pool.begin().await?;
                    for statement in statements.split(';').map(str::trim) {
                        if !statement.is_empty() {
                            sqlx::query(statement).execute(&mut *tx).await?;
                        }
                    }
                    sqlx::query("UPDATE schema_version SET version = $1")
                        .bind(*target)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    tracing::info!(version = target, "schéma de la base migré");
                }
                sqlx::query("UPDATE schema_version SET version = $1")
                    .bind(SCHEMA_VERSION)
                    .execute(&pool)
                    .await?;
            }
            Some(v) => {
                return Err(HubError::Internal(format!(
                    "schéma de base en version {v}, ce hub connaît la version {SCHEMA_VERSION}"
                )))
            }
        }
        Ok(Store { pool, backend })
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
            "INSERT INTO users (username, username_norm, created_at) VALUES ($1, $2, $3) \
             RETURNING id",
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
             VALUES ($1, $2, $3, $4, $5)",
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
            "SELECT user_id, secret_hash FROM user_identities WHERE provider = $1 AND subject = $2",
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
            "SELECT subject FROM user_identities WHERE user_id = $1 AND provider = $2 ORDER BY id",
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
        let row = sqlx::query("SELECT id, username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| user_row(&r)).transpose()
    }

    pub async fn user_by_username(&self, username_norm: &str) -> Result<Option<UserRow>, HubError> {
        let row = sqlx::query("SELECT id, username FROM users WHERE username_norm = $1")
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
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) \
             VALUES ($1, $2, $3, $4)",
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
            sqlx::query("SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > $2")
                .bind(token_hash)
                .bind(now)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|r| r.try_get("user_id"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), HubError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Retire les sessions expirées ; renvoie le nombre de lignes supprimées.
    pub async fn purge_expired_sessions(&self, now: i64) -> Result<u64, HubError> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn are_friends(&self, a: i64, b: i64) -> Result<bool, HubError> {
        let (a, b) = ordered(a, b);
        let row =
            sqlx::query("SELECT 1 AS found FROM friendships WHERE user_a = $1 AND user_b = $2")
                .bind(a)
                .bind(b)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn request_exists(&self, from: i64, to: i64) -> Result<bool, HubError> {
        let row = sqlx::query(
            "SELECT 1 AS found FROM friend_requests WHERE from_user = $1 AND to_user = $2",
        )
        .bind(from)
        .bind(to)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn insert_request(&self, from: i64, to: i64, now: i64) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO friend_requests (from_user, to_user, created_at) VALUES ($1, $2, $3)",
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
        let result =
            sqlx::query("DELETE FROM friend_requests WHERE from_user = $1 AND to_user = $2")
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
            "DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) \
             OR (from_user = $2 AND to_user = $1)",
        )
        .bind(a)
        .bind(b)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO friendships (user_a, user_b, created_at) VALUES ($1, $2, $3) \
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
        let result = sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
            .bind(a)
            .bind(b)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Amis d'un compte, rangés par pseudo normalisé (ordre stable).
    pub async fn friends_of(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        // `$1` sert trois fois : on ne le lie qu'une fois.
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM friendships f \
             JOIN users u ON u.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END \
             WHERE f.user_a = $1 OR f.user_b = $1 \
             ORDER BY u.username_norm, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }

    /// Demandes reçues, de la plus ancienne à la plus récente.
    pub async fn incoming_requests(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM friend_requests r JOIN users u ON u.id = r.from_user \
             WHERE r.to_user = $1 ORDER BY r.created_at, u.id",
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
             WHERE r.from_user = $1 ORDER BY r.created_at, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }
}

impl Store {
    /// Bloque un joueur : retire l'amitié et les demandes dans les deux sens, puis
    /// inscrit le blocage, en une transaction. Bloquer deux fois ne change rien.
    pub async fn block(&self, blocker: i64, blocked: i64, now: i64) -> Result<(), HubError> {
        let (low, high) = ordered(blocker, blocked);
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
            .bind(low)
            .bind(high)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) \
             OR (from_user = $2 AND to_user = $1)",
        )
        .bind(blocker)
        .bind(blocked)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO blocks (blocker, blocked, created_at) VALUES ($1, $2, $3) \
             ON CONFLICT (blocker, blocked) DO NOTHING",
        )
        .bind(blocker)
        .bind(blocked)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Lève un blocage ; renvoie `false` s'il n'existait pas. L'amitié perdue ne
    /// revient pas : il faut une nouvelle demande.
    pub async fn unblock(&self, blocker: i64, blocked: i64) -> Result<bool, HubError> {
        let result = sqlx::query("DELETE FROM blocks WHERE blocker = $1 AND blocked = $2")
            .bind(blocker)
            .bind(blocked)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `blocker` a-t-il bloqué `blocked` ? (un seul sens)
    pub async fn has_blocked(&self, blocker: i64, blocked: i64) -> Result<bool, HubError> {
        let row = sqlx::query("SELECT 1 AS found FROM blocks WHERE blocker = $1 AND blocked = $2")
            .bind(blocker)
            .bind(blocked)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Joueurs bloqués par un compte, rangés par pseudo normalisé.
    pub async fn blocked_by(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username FROM blocks b JOIN users u ON u.id = b.blocked \
             WHERE b.blocker = $1 ORDER BY u.username_norm, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }
}

impl Store {
    /// Hash du mot de passe d'un compte, s'il en a un (identité « e-mail »).
    pub async fn password_hash_of(&self, user_id: i64) -> Result<Option<String>, HubError> {
        let row = sqlx::query(
            "SELECT secret_hash FROM user_identities WHERE user_id = $1 AND provider = $2 \
             ORDER BY id",
        )
        .bind(user_id)
        .bind(EMAIL_PROVIDER)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(row.try_get("secret_hash")?),
            None => Ok(None),
        }
    }

    /// Rassemble toutes les données d'un compte (RGPD). `None` si le compte n'existe pas.
    pub async fn export_account(&self, user_id: i64) -> Result<Option<AccountExport>, HubError> {
        let Some(user) = sqlx::query("SELECT id, username, created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };

        let mut identities = Vec::new();
        for row in sqlx::query(
            "SELECT provider, subject, verified, created_at FROM user_identities \
             WHERE user_id = $1 ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?
        {
            let verified: i64 = row.try_get("verified")?;
            identities.push(IdentityExport {
                provider: row.try_get("provider")?,
                subject: row.try_get("subject")?,
                verified: verified != 0,
                created_at: row.try_get("created_at")?,
            });
        }

        let mut sessions = Vec::new();
        for row in sqlx::query(
            "SELECT created_at, expires_at FROM sessions WHERE user_id = $1 \
             ORDER BY created_at, expires_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?
        {
            sessions.push(SessionExport {
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
            });
        }

        let friends = self
            .links(
                "SELECT u.username, f.created_at FROM friendships f \
                 JOIN users u ON u.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END \
                 WHERE f.user_a = $1 OR f.user_b = $1 ORDER BY u.username_norm",
                user_id,
            )
            .await?;
        let requests_received = self
            .links(
                "SELECT u.username, r.created_at FROM friend_requests r \
                 JOIN users u ON u.id = r.from_user WHERE r.to_user = $1 \
                 ORDER BY r.created_at, u.username_norm",
                user_id,
            )
            .await?;
        let requests_sent = self
            .links(
                "SELECT u.username, r.created_at FROM friend_requests r \
                 JOIN users u ON u.id = r.to_user WHERE r.from_user = $1 \
                 ORDER BY r.created_at, u.username_norm",
                user_id,
            )
            .await?;
        let blocked = self
            .links(
                "SELECT u.username, b.created_at FROM blocks b \
                 JOIN users u ON u.id = b.blocked WHERE b.blocker = $1 \
                 ORDER BY u.username_norm",
                user_id,
            )
            .await?;

        Ok(Some(AccountExport {
            user_id: user.try_get("id")?,
            username: user.try_get("username")?,
            created_at: user.try_get("created_at")?,
            identities,
            sessions,
            friends,
            requests_received,
            requests_sent,
            blocked,
        }))
    }

    /// Exécute une requête qui renvoie `(username, created_at)` pour un compte.
    async fn links(&self, sql: &str, user_id: i64) -> Result<Vec<LinkExport>, HubError> {
        let rows = sqlx::query(sql).bind(user_id).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                Ok(LinkExport {
                    username: row.try_get("username")?,
                    since: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    /// Supprime un compte et tout ce qui s'y rattache (RGPD, droit à l'effacement).
    ///
    /// Une seule ligne effacée suffit : les autres tables pointent vers `users` avec
    /// `ON DELETE CASCADE` (identités, sessions, amitiés, demandes, blocages).
    /// Renvoie `false` si le compte n'existait pas.
    pub async fn delete_account(&self, user_id: i64) -> Result<bool, HubError> {
        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Nombre de lignes rattachées à un compte, toutes tables confondues
    /// (pour vérifier dans les tests que la suppression n'oublie rien).
    pub async fn rows_of_user(&self, user_id: i64) -> Result<i64, HubError> {
        let row = sqlx::query(
            "SELECT \
             (SELECT COUNT(*) FROM users WHERE id = $1) + \
             (SELECT COUNT(*) FROM user_identities WHERE user_id = $1) + \
             (SELECT COUNT(*) FROM sessions WHERE user_id = $1) + \
             (SELECT COUNT(*) FROM friend_requests WHERE from_user = $1 OR to_user = $1) + \
             (SELECT COUNT(*) FROM friendships WHERE user_a = $1 OR user_b = $1) + \
             (SELECT COUNT(*) FROM blocks WHERE blocker = $1 OR blocked = $1) + \
             (SELECT COUNT(*) FROM email_tokens WHERE user_id = $1) AS total",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get("total")?)
    }
}

impl Store {
    /// Adresse e-mail d'un compte et si elle est vérifiée.
    pub async fn email_status(&self, user_id: i64) -> Result<Option<(String, bool)>, HubError> {
        let row = sqlx::query(
            "SELECT subject, verified FROM user_identities WHERE user_id = $1 AND provider = $2 \
             ORDER BY id",
        )
        .bind(user_id)
        .bind(EMAIL_PROVIDER)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => {
                let verified: i64 = row.try_get("verified")?;
                Ok(Some((row.try_get("subject")?, verified != 0)))
            }
            None => Ok(None),
        }
    }

    /// Enregistre un jeton à usage unique (seule son empreinte est gardée). Les
    /// jetons précédents du même usage pour ce compte sont retirés : seul le
    /// dernier message envoyé reste valable.
    pub async fn insert_email_token(
        &self,
        token_hash: &str,
        user_id: i64,
        purpose: TokenPurpose,
        now: i64,
        expires_at: i64,
    ) -> Result<(), HubError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2")
            .bind(user_id)
            .bind(purpose.as_str())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO email_tokens (token_hash, user_id, purpose, created_at, expires_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token_hash)
        .bind(user_id)
        .bind(purpose.as_str())
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Consomme un jeton : le supprime et renvoie son compte, s'il existe, a le bon
    /// usage et n'a pas expiré. Une seule requête (`DELETE … RETURNING`) : deux
    /// requêtes simultanées avec le même jeton ne peuvent pas réussir toutes les deux.
    pub async fn consume_email_token(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: i64,
    ) -> Result<Option<i64>, HubError> {
        let row = sqlx::query(
            "DELETE FROM email_tokens WHERE token_hash = $1 AND purpose = $2 AND expires_at > $3 \
             RETURNING user_id",
        )
        .bind(token_hash)
        .bind(purpose.as_str())
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| r.try_get("user_id"))
            .transpose()
            .map_err(Into::into)
    }

    /// Retire les jetons expirés ; renvoie le nombre de lignes supprimées.
    pub async fn purge_expired_email_tokens(&self, now: i64) -> Result<u64, HubError> {
        let result = sqlx::query("DELETE FROM email_tokens WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Marque l'adresse e-mail d'un compte comme vérifiée.
    pub async fn mark_email_verified(&self, user_id: i64) -> Result<(), HubError> {
        sqlx::query("UPDATE user_identities SET verified = 1 WHERE user_id = $1 AND provider = $2")
            .bind(user_id)
            .bind(EMAIL_PROVIDER)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Nouveau mot de passe, en une transaction : nouveau hash, adresse vérifiée (le
    /// joueur a lu le message), toutes les sessions fermées (celles d'un éventuel
    /// voleur aussi) et jetons de réinitialisation restants retirés.
    pub async fn reset_password(&self, user_id: i64, password_hash: &str) -> Result<(), HubError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE user_identities SET secret_hash = $1, verified = 1 \
             WHERE user_id = $2 AND provider = $3",
        )
        .bind(password_hash)
        .bind(user_id)
        .bind(EMAIL_PROVIDER)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2")
            .bind(user_id)
            .bind(TokenPurpose::ResetPassword.as_str())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

impl Store {
    /// Compte lié à une identité (`discord`, identifiant Discord…).
    pub async fn identity_user(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<i64>, HubError> {
        let row =
            sqlx::query("SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2")
                .bind(provider)
                .bind(subject)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|r| r.try_get("user_id"))
            .transpose()
            .map_err(Into::into)
    }

    /// Fournisseurs liés à un compte (`email`, `discord`), dans l'ordre de liaison.
    pub async fn identity_providers(&self, user_id: i64) -> Result<Vec<String>, HubError> {
        let rows =
            sqlx::query("SELECT provider FROM user_identities WHERE user_id = $1 ORDER BY id")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| row.try_get("provider").map_err(Into::into))
            .collect()
    }

    /// Lie une identité externe (sans mot de passe, vérifiée par le fournisseur) à
    /// un compte existant.
    pub async fn link_identity(
        &self,
        user_id: i64,
        provider: &str,
        subject: &str,
        now: i64,
    ) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO user_identities (user_id, provider, subject, secret_hash, verified, \
             created_at) VALUES ($1, $2, $3, NULL, 1, $4)",
        )
        .bind(user_id)
        .bind(provider)
        .bind(subject)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(map_identity_unique)?;
        Ok(())
    }

    /// Crée un compte dont le seul moyen de connexion est une identité externe.
    pub async fn create_account_with_identity(
        &self,
        username: &str,
        username_norm: &str,
        provider: &str,
        subject: &str,
        now: i64,
    ) -> Result<i64, HubError> {
        let mut tx = self.pool.begin().await?;
        let user_id: i64 = sqlx::query(
            "INSERT INTO users (username, username_norm, created_at) VALUES ($1, $2, $3) \
             RETURNING id",
        )
        .bind(username)
        .bind(username_norm)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_unique)?
        .try_get("id")?;
        sqlx::query(
            "INSERT INTO user_identities (user_id, provider, subject, secret_hash, verified, \
             created_at) VALUES ($1, $2, $3, NULL, 1, $4)",
        )
        .bind(user_id)
        .bind(provider)
        .bind(subject)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_identity_unique)?;
        tx.commit().await?;
        Ok(user_id)
    }

    /// Retire une identité d'un compte ; `false` s'il n'en avait pas.
    pub async fn unlink_identity(&self, user_id: i64, provider: &str) -> Result<bool, HubError> {
        let result =
            sqlx::query("DELETE FROM user_identities WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(provider)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}

/// Violation d'unicité sur `user_identities` lors d'une liaison : soit l'identité
/// est déjà prise par un autre compte, soit ce compte a déjà ce fournisseur.
fn map_identity_unique(error: sqlx::Error) -> HubError {
    if let sqlx::Error::Database(db) = &error {
        if db.is_unique_violation() {
            let text = format!("{} {}", db.message(), db.constraint().unwrap_or(""));
            if text.contains("one_per_provider") || text.contains("user_identities.user_id") {
                return HubError::AlreadyLinked;
            }
            return HubError::DiscordTaken;
        }
    }
    HubError::Database(error)
}

fn user_row(row: &AnyRow) -> Result<UserRow, HubError> {
    Ok(UserRow {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
    })
}

/// Traduit une violation d'unicité en erreur lisible. Le message des deux bases
/// nomme la contrainte en cause : `users.username_norm` (SQLite),
/// `users_username_norm_key` (PostgreSQL), et de même pour `user_identities`.
fn map_unique(error: sqlx::Error) -> HubError {
    if let sqlx::Error::Database(db) = &error {
        if db.is_unique_violation() {
            let message = db.message();
            let constraint = db.constraint().unwrap_or("");
            if message.contains("username_norm") || constraint.contains("username_norm") {
                return HubError::UsernameTaken;
            }
            if message.contains("user_identities") || constraint.contains("user_identities") {
                return HubError::EmailTaken;
            }
        }
    }
    HubError::Database(error)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(db) if db.is_unique_violation())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_sqlite_recrites() {
        assert_eq!(sqlite_url("sqlite://hub.db"), "sqlite:hub.db?mode=rwc");
        assert_eq!(
            sqlite_url(r"sqlite://C:\d\hub.db"),
            r"sqlite:C:\d\hub.db?mode=rwc"
        );
        assert_eq!(sqlite_url("sqlite:hub.db?mode=ro"), "sqlite:hub.db?mode=ro");
        assert_eq!(sqlite_url("sqlite::memory:"), "sqlite::memory:");
    }

    #[test]
    fn base_reconnue_par_l_url() {
        assert_eq!(Backend::from_url("sqlite:x").ok(), Some(Backend::Sqlite));
        assert_eq!(
            Backend::from_url("postgres://u@h/b").ok(),
            Some(Backend::Postgres)
        );
        assert!(Backend::from_url("mysql://h").is_err());
    }
}
