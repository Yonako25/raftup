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
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use sqlx::any::{AnyArguments, AnyPoolOptions, AnyRow};
use sqlx::query::Query;
use sqlx::{Any, AnyPool, Row};

use crate::error::HubError;

// Méthodes de la console d'admin, dans deux fichiers à part (des modules enfants
// voient les champs privés de `Store`) :
// - `store/admin_auth.rs` : rôles et sessions d'admin (connexion à la console) ;
// - `store/admin.rs` : suspension, réglages, invitations, journal, liste des joueurs.
mod admin;
mod admin_auth;
pub use admin::EventFilter;
pub use admin::{
    AdminEvent, DashboardCounts, InviteRow, Page, PlayerFilter, PlayerSummary, MAX_PAGE_SIZE,
};
pub use admin_auth::Role;

/// Version du schéma de la base. À augmenter à chaque changement de schéma,
/// en ajoutant la migration correspondante dans [`MIGRATIONS`].
pub const SCHEMA_VERSION: i64 = 6;

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
    username_norm {key} NOT NULL UNIQUE,
    created_at BIGINT NOT NULL,
    role {key} NOT NULL DEFAULT 'player',
    suspended_at BIGINT
);
CREATE TABLE IF NOT EXISTS user_identities (
    id {id},
    user_id BIGINT NOT NULL,
    provider {key} NOT NULL,
    subject {key} NOT NULL,
    secret_hash TEXT,
    verified BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    CONSTRAINT user_identities_provider_subject UNIQUE (provider, subject),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS sessions (
    token_hash {key} PRIMARY KEY,
    user_id BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS friend_requests (
    from_user BIGINT NOT NULL,
    to_user BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (from_user, to_user),
    FOREIGN KEY (from_user) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (to_user) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS friendships (
    user_a BIGINT NOT NULL,
    user_b BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_a, user_b),
    CHECK (user_a < user_b),
    FOREIGN KEY (user_a) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (user_b) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS blocks (
    blocker BIGINT NOT NULL,
    blocked BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (blocker, blocked),
    CHECK (blocker <> blocked),
    FOREIGN KEY (blocker) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (blocked) REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS blocks_blocked ON blocks (blocked);
CREATE TABLE IF NOT EXISTS email_tokens (
    token_hash {key} PRIMARY KEY,
    user_id BIGINT NOT NULL,
    purpose TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS email_tokens_user ON email_tokens (user_id);
CREATE UNIQUE INDEX IF NOT EXISTS user_identities_one_per_provider
    ON user_identities (user_id, provider);
CREATE INDEX IF NOT EXISTS friend_requests_to ON friend_requests (to_user);
CREATE INDEX IF NOT EXISTS friendships_b ON friendships (user_b);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id);
CREATE TABLE IF NOT EXISTS admin_sessions (
    token_hash {key} PRIMARY KEY,
    user_id BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS admin_sessions_user ON admin_sessions (user_id);
CREATE TABLE IF NOT EXISTS settings (
    name {key} PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE TABLE IF NOT EXISTS invites (
    id {id},
    code_hash {key} NOT NULL UNIQUE,
    created_by BIGINT,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_by BIGINT,
    used_at BIGINT,
    FOREIGN KEY (created_by) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (used_by) REFERENCES users(id) ON DELETE SET NULL
);
CREATE TABLE IF NOT EXISTS admin_events (
    id {id},
    created_at BIGINT NOT NULL,
    actor_id BIGINT,
    kind TEXT NOT NULL,
    target_id BIGINT,
    detail TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS admin_events_created ON admin_events (created_at)
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
    // Version 5 (ADR 0036) : rôle du compte (`player` ou `admin`) ; la table
    // `admin_sessions` est créée par `SCHEMA`.
    (
        5,
        "ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'player'",
    ),
    // Version 6 (console Rafthub, ADR 0036) : suspension des comptes. Les tables
    // `settings`, `invites` et `admin_events` sont créées par `SCHEMA`.
    (6, "ALTER TABLE users ADD COLUMN suspended_at BIGINT"),
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
    /// MySQL et MariaDB : même pilote, même dialecte SQL (ADR 0037).
    Mysql,
}

impl Backend {
    /// Reconnaît la base d'après le début de l'URL.
    pub fn from_url(url: &str) -> Result<Backend, HubError> {
        if url.starts_with("sqlite:") {
            Ok(Backend::Sqlite)
        } else if url.starts_with("postgres:") || url.starts_with("postgresql:") {
            Ok(Backend::Postgres)
        } else if url.starts_with("mysql:") || url.starts_with("mariadb:") {
            Ok(Backend::Mysql)
        } else {
            Err(HubError::Internal(
                "URL de base inconnue : attendu sqlite:…, postgres://… ou mysql://…".to_string(),
            ))
        }
    }

    /// Colonne d'identifiant auto-incrémenté. En SQLite, `INTEGER PRIMARY KEY` (et
    /// pas `BIGINT`) est le seul type qui donne un identifiant automatique.
    fn id_column(self) -> &'static str {
        match self {
            Backend::Sqlite => "INTEGER PRIMARY KEY",
            Backend::Postgres => "BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY",
            Backend::Mysql => "BIGINT AUTO_INCREMENT PRIMARY KEY",
        }
    }

    /// Type d'une colonne de texte indexée (clé, unique). MySQL ne sait pas indexer
    /// un `TEXT` sans longueur : il lui faut un `VARCHAR`.
    fn key_column(self) -> &'static str {
        match self {
            Backend::Sqlite | Backend::Postgres => "TEXT",
            Backend::Mysql => "VARCHAR(255)",
        }
    }

    /// Schéma complet pour cette base.
    fn schema(self) -> String {
        let schema = SCHEMA
            .replace("{id}", self.id_column())
            .replace("{key}", self.key_column());
        if self != Backend::Mysql {
            return schema;
        }
        // MySQL envoie ses colonnes `TEXT` comme des blocs binaires, que le pilote
        // générique ne sait pas lire en `String` : on y met des `VARCHAR`, assez longs
        // pour nos valeurs (une adresse SMTP, un détail de journal de 200 caractères).
        let schema = schema.replace(" TEXT", " VARCHAR(1024)");
        // Tables InnoDB en UTF-8 complet. Comparaison `utf8mb4_unicode_ci` (sans tenir
        // compte des majuscules) : avec `utf8mb4_bin`, MySQL présente aussi le texte
        // comme binaire. Le hub range déjà pseudos, e-mails et empreintes en
        // minuscules : aucune différence.
        // `CREATE INDEX` n'y connaît pas `IF NOT EXISTS`
        // (un index déjà là est reconnu à son erreur, voir `Store::init`).
        schema
            .split(';')
            .map(|statement| {
                let statement = statement.trim();
                if statement.starts_with("CREATE TABLE") {
                    // MySQL refuse un `CHECK` sur une colonne de clé étrangère à
                    // `ON DELETE CASCADE` : le code garantit déjà ces règles
                    // (`ordered`, refus de se bloquer soi-même).
                    let statement: String = statement
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("CHECK ("))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("{statement} ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci")
                } else {
                    statement.replace("INDEX IF NOT EXISTS", "INDEX")
                }
            })
            .collect::<Vec<_>>()
            .join(";\n")
    }
}

/// Traduit une requête écrite pour SQLite et PostgreSQL vers MySQL / MariaDB :
/// - les paramètres `$1`, `$2`… deviennent `?` (chaque requête les numérote dans
///   l'ordre, une seule fois chacun : vérifié par le test `parametres_dans_l_ordre`) ;
/// - dans `ESCAPE '\'`, MySQL lit `\` comme un caractère d'échappement : il faut l'écrire deux fois.
fn mysql_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    // `peekable` : regarder le caractère suivant sans l'avancer.
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek().is_some_and(char::is_ascii_digit) {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
            out.push('?');
        } else {
            out.push(c);
        }
    }
    out.replace("ESCAPE '\\'", "ESCAPE '\\\\'")
}

/// Requête traduite pour MySQL, gardée pour toute la vie du process.
///
/// `sqlx::query` emprunte le texte de la requête : il doit vivre au moins aussi
/// longtemps qu'elle. La traduction est une `String` neuve ; on la garde donc dans
/// une table globale (`Box::leak` la rend `'static`). Le hub n'a qu'un nombre fini
/// de requêtes différentes : la mémoire gardée reste petite.
fn mysql_interned(sql: &str) -> &'static str {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    // Un verrou empoisonné (panique ailleurs) garde des données valides : on continue.
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = cache.get(sql) {
        return found;
    }
    let translated: &'static str = Box::leak(mysql_sql(sql).into_boxed_str());
    cache.insert(sql.to_string(), translated);
    translated
}

/// Prépare une requête pour la base `backend` (voir [`mysql_sql`]).
fn query_for<'q>(backend: Backend, sql: &'q str) -> Query<'q, Any, AnyArguments<'q>> {
    match backend {
        Backend::Mysql => sqlx::query(mysql_interned(sql)),
        Backend::Sqlite | Backend::Postgres => sqlx::query(sql),
    }
}

/// Accès à la base. `Clone` est bon marché : le pool est partagé derrière un compteur.
#[derive(Clone)]
pub struct Store {
    pool: AnyPool,
    backend: Backend,
}

impl Store {
    /// Prépare une requête pour la base de ce hub : à utiliser à la place de
    /// `sqlx::query`, pour que la même requête marche aussi sur MySQL / MariaDB.
    pub(crate) fn q<'q>(&self, sql: &'q str) -> Query<'q, Any, AnyArguments<'q>> {
        query_for(self.backend, sql)
    }
}

/// Pour une instruction `ALTER TABLE <table> ADD COLUMN <colonne> …` : la colonne
/// existe-t-elle déjà ? (SQLite n'a pas `ADD COLUMN IF NOT EXISTS`.) Toute autre
/// instruction répond `false`.
async fn added_column_exists(pool: &AnyPool, statement: &str) -> bool {
    let words: Vec<&str> = statement.split_whitespace().collect();
    let [alter, table_kw, table, add, column_kw, column, ..] = words.as_slice() else {
        return false;
    };
    let is_add_column = alter.eq_ignore_ascii_case("ALTER")
        && table_kw.eq_ignore_ascii_case("TABLE")
        && add.eq_ignore_ascii_case("ADD")
        && column_kw.eq_ignore_ascii_case("COLUMN");
    // Noms tirés de nos propres migrations (constantes), jamais d'une saisie.
    is_add_column
        && sqlx::query(&format!("SELECT {column} FROM {table} LIMIT 0"))
            .fetch_optional(pool)
            .await
            .is_ok()
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
            Backend::Postgres | Backend::Mysql => {
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
        let schema = backend.schema();
        for statement in schema.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            match sqlx::query(statement).execute(&pool).await {
                Ok(_) => {}
                // MySQL : index déjà créé (erreur 1061, « Duplicate key name »).
                Err(sqlx::Error::Database(db))
                    if backend == Backend::Mysql && db.message().contains("Duplicate key name") => {
                }
                Err(error) => return Err(error.into()),
            }
        }
        let version: Option<i64> = query_for(backend, "SELECT version FROM schema_version")
            .fetch_optional(&pool)
            .await?
            .map(|row| row.try_get("version"))
            .transpose()?;
        match version {
            None => {
                query_for(backend, "INSERT INTO schema_version (version) VALUES ($1)")
                    .bind(SCHEMA_VERSION)
                    .execute(&pool)
                    .await?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) if v < SCHEMA_VERSION => {
                for (target, statements) in MIGRATIONS.iter().filter(|(t, _)| *t > v) {
                    // Colonnes déjà là (base ramenée à une version plus ancienne,
                    // migration rejouée) : vérifié avant la transaction, car une
                    // requête en échec annule toute la transaction sous PostgreSQL.
                    let mut todo = Vec::new();
                    for statement in statements.split(';').map(str::trim) {
                        if statement.is_empty() || added_column_exists(&pool, statement).await {
                            continue;
                        }
                        todo.push(statement);
                    }
                    let mut tx = pool.begin().await?;
                    for statement in todo {
                        query_for(backend, statement).execute(&mut *tx).await?;
                    }
                    query_for(backend, "UPDATE schema_version SET version = $1")
                        .bind(*target)
                        .execute(&mut *tx)
                        .await?;
                    tx.commit().await?;
                    tracing::info!(version = target, "schéma de la base migré");
                }
                query_for(backend, "UPDATE schema_version SET version = $1")
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

    /// Ajoute la ligne `users` d'un nouveau compte et rend son identifiant.
    ///
    /// SQLite et PostgreSQL rendent l'identifiant avec `RETURNING id` ; MySQL ne
    /// connaît pas `RETURNING` : il donne l'identifiant créé dans le résultat de
    /// l'`INSERT` (`last_insert_id`).
    async fn insert_user(
        &self,
        tx: &mut sqlx::Transaction<'static, Any>,
        username: &str,
        username_norm: &str,
        now: i64,
    ) -> Result<i64, HubError> {
        const INSERT: &str =
            "INSERT INTO users (username, username_norm, created_at) VALUES ($1, $2, $3)";
        if self.backend == Backend::Mysql {
            let result = self
                .q(INSERT)
                .bind(username)
                .bind(username_norm)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(map_unique)?;
            return result
                .last_insert_id()
                .ok_or_else(|| HubError::Internal("identifiant du compte non rendu".into()));
        }
        let row = self
            .q(concat!(
                "INSERT INTO users (username, username_norm, created_at) VALUES ($1, $2, $3)",
                " RETURNING id"
            ))
            .bind(username)
            .bind(username_norm)
            .bind(now)
            .fetch_one(&mut **tx)
            .await
            .map_err(map_unique)?;
        Ok(row.try_get("id")?)
    }

    /// Crée un compte et son identité « e-mail », dans une seule transaction.
    ///
    /// `invite_hash` : empreinte d'un code d'invitation (mode d'inscription `invite`).
    /// Le code est consommé dans la même transaction : s'il est faux, expiré ou déjà
    /// pris, rien n'est créé ([`HubError::InvalidInvite`]).
    pub async fn create_account(
        &self,
        username: &str,
        username_norm: &str,
        email_norm: &str,
        password_hash: &str,
        invite_hash: Option<&str>,
        now: i64,
    ) -> Result<i64, HubError> {
        let mut tx = self.pool.begin().await?;
        // `&mut *tx` : la transaction se comporte comme une connexion ; on la prête
        // à chaque requête sans la céder, pour pouvoir la valider à la fin.
        let user_id = self
            .insert_user(&mut tx, username, username_norm, now)
            .await?;
        self.q(
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
        if let Some(hash) = invite_hash {
            admin::consume_invite(self.backend, &mut tx, hash, user_id, now).await?;
        }
        tx.commit().await?;
        Ok(user_id)
    }

    /// Compte et hash du mot de passe liés à une adresse e-mail normalisée.
    pub async fn email_identity(
        &self,
        email_norm: &str,
    ) -> Result<Option<(i64, String)>, HubError> {
        let row = self.q(
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
        let row = self.q(
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
        let row = self
            .q("SELECT id, username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| user_row(&r)).transpose()
    }

    pub async fn user_by_username(&self, username_norm: &str) -> Result<Option<UserRow>, HubError> {
        let row = self
            .q("SELECT id, username FROM users WHERE username_norm = $1")
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
        self.q(
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

    /// Compte d'une session encore valide. Un compte suspendu n'a plus de session
    /// (elles sont supprimées à la suspension) ; la jointure le vérifie quand même.
    pub async fn session_user(&self, token_hash: &str, now: i64) -> Result<Option<i64>, HubError> {
        let row = self
            .q(
                "SELECT s.user_id FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.expires_at > $2 AND u.suspended_at IS NULL",
            )
            .bind(token_hash)
            .bind(now)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| r.try_get("user_id"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn delete_session(&self, token_hash: &str) -> Result<(), HubError> {
        self.q("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Retire les sessions expirées ; renvoie le nombre de lignes supprimées.
    pub async fn purge_expired_sessions(&self, now: i64) -> Result<u64, HubError> {
        let result = self
            .q("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn are_friends(&self, a: i64, b: i64) -> Result<bool, HubError> {
        let (a, b) = ordered(a, b);
        let row = self
            .q("SELECT 1 AS found FROM friendships WHERE user_a = $1 AND user_b = $2")
            .bind(a)
            .bind(b)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn request_exists(&self, from: i64, to: i64) -> Result<bool, HubError> {
        let row = self
            .q("SELECT 1 AS found FROM friend_requests WHERE from_user = $1 AND to_user = $2")
            .bind(from)
            .bind(to)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    pub async fn insert_request(&self, from: i64, to: i64, now: i64) -> Result<(), HubError> {
        self.q("INSERT INTO friend_requests (from_user, to_user, created_at) VALUES ($1, $2, $3)")
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
        let result = self
            .q("DELETE FROM friend_requests WHERE from_user = $1 AND to_user = $2")
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
        self.q(
            "DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) \
             OR (from_user = $3 AND to_user = $4)",
        )
        .bind(a)
        .bind(b)
        .bind(b)
        .bind(a)
        .execute(&mut *tx)
        .await?;
        // Déjà là : rien à faire. MySQL dit `INSERT IGNORE` au lieu de `ON CONFLICT`.
        let insert = if self.backend == Backend::Mysql {
            "INSERT IGNORE INTO friendships (user_a, user_b, created_at) VALUES ($1, $2, $3)"
        } else {
            "INSERT INTO friendships (user_a, user_b, created_at) VALUES ($1, $2, $3) \
             ON CONFLICT (user_a, user_b) DO NOTHING"
        };
        self.q(insert)
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
        let result = self
            .q("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
            .bind(a)
            .bind(b)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Amis d'un compte, rangés par pseudo normalisé (ordre stable). Les comptes
    /// suspendus sont cachés (ADR 0036), comme dans les demandes et les blocages.
    pub async fn friends_of(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        // Le compte sert trois fois : un paramètre par usage (MySQL n'a que `?`).
        let rows = self
            .q("SELECT u.id, u.username FROM friendships f \
             JOIN users u ON u.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END \
             WHERE (f.user_a = $2 OR f.user_b = $3) AND u.suspended_at IS NULL \
             ORDER BY u.username_norm, u.id")
            .bind(user_id)
            .bind(user_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(user_row).collect()
    }

    /// Demandes reçues, de la plus ancienne à la plus récente.
    pub async fn incoming_requests(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = self.q(
            "SELECT u.id, u.username FROM friend_requests r JOIN users u ON u.id = r.from_user \
             WHERE r.to_user = $1 AND u.suspended_at IS NULL ORDER BY r.created_at, u.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(user_row).collect()
    }

    /// Demandes envoyées, de la plus ancienne à la plus récente.
    pub async fn outgoing_requests(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = self
            .q(
                "SELECT u.id, u.username FROM friend_requests r JOIN users u ON u.id = r.to_user \
             WHERE r.from_user = $1 AND u.suspended_at IS NULL ORDER BY r.created_at, u.id",
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
        self.q("DELETE FROM friendships WHERE user_a = $1 AND user_b = $2")
            .bind(low)
            .bind(high)
            .execute(&mut *tx)
            .await?;
        self.q(
            "DELETE FROM friend_requests WHERE (from_user = $1 AND to_user = $2) \
             OR (from_user = $3 AND to_user = $4)",
        )
        .bind(blocker)
        .bind(blocked)
        .bind(blocked)
        .bind(blocker)
        .execute(&mut *tx)
        .await?;
        // Déjà là : rien à faire. MySQL dit `INSERT IGNORE` au lieu de `ON CONFLICT`.
        let insert = if self.backend == Backend::Mysql {
            "INSERT IGNORE INTO blocks (blocker, blocked, created_at) VALUES ($1, $2, $3)"
        } else {
            "INSERT INTO blocks (blocker, blocked, created_at) VALUES ($1, $2, $3) \
             ON CONFLICT (blocker, blocked) DO NOTHING"
        };
        self.q(insert)
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
        let result = self
            .q("DELETE FROM blocks WHERE blocker = $1 AND blocked = $2")
            .bind(blocker)
            .bind(blocked)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `blocker` a-t-il bloqué `blocked` ? (un seul sens)
    pub async fn has_blocked(&self, blocker: i64, blocked: i64) -> Result<bool, HubError> {
        let row = self
            .q("SELECT 1 AS found FROM blocks WHERE blocker = $1 AND blocked = $2")
            .bind(blocker)
            .bind(blocked)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Joueurs bloqués par un compte, rangés par pseudo normalisé.
    pub async fn blocked_by(&self, user_id: i64) -> Result<Vec<UserRow>, HubError> {
        let rows = self
            .q(
                "SELECT u.id, u.username FROM blocks b JOIN users u ON u.id = b.blocked \
             WHERE b.blocker = $1 AND u.suspended_at IS NULL ORDER BY u.username_norm, u.id",
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
        let row = self
            .q(
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
        let Some(user) = self
            .q("SELECT id, username, created_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };

        let mut identities = Vec::new();
        for row in self
            .q(
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
        for row in self
            .q(
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
                 WHERE f.user_a = $2 OR f.user_b = $3 ORDER BY u.username_norm",
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
        // Un paramètre par usage du compte (`$1`, `$2`…) : on le lie autant de fois.
        let mut query = self.q(sql);
        for _ in 0..sql.matches('$').count() {
            query = query.bind(user_id);
        }
        let rows = query.fetch_all(&self.pool).await?;
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
    /// Le journal de la console (`admin_events`) n'a pas de clé étrangère : on y
    /// efface à la main l'identifiant du compte, pour qu'aucune ligne ne le désigne
    /// encore (et qu'un futur compte au même numéro n'hérite pas de son passé).
    /// Renvoie `false` si le compte n'existait pas.
    pub async fn delete_account(&self, user_id: i64) -> Result<bool, HubError> {
        let mut tx = self.pool.begin().await?;
        self.q("UPDATE admin_events SET actor_id = NULL WHERE actor_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        self.q("UPDATE admin_events SET target_id = NULL WHERE target_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        let result = self
            .q("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// Nombre de lignes rattachées à un compte, toutes tables confondues
    /// (pour vérifier dans les tests que la suppression n'oublie rien).
    pub async fn rows_of_user(&self, user_id: i64) -> Result<i64, HubError> {
        let query = self.q("SELECT \
             (SELECT COUNT(*) FROM users WHERE id = $1) + \
             (SELECT COUNT(*) FROM user_identities WHERE user_id = $2) + \
             (SELECT COUNT(*) FROM sessions WHERE user_id = $3) + \
             (SELECT COUNT(*) FROM friend_requests WHERE from_user = $4 OR to_user = $5) + \
             (SELECT COUNT(*) FROM friendships WHERE user_a = $6 OR user_b = $7) + \
             (SELECT COUNT(*) FROM blocks WHERE blocker = $8 OR blocked = $9) + \
             (SELECT COUNT(*) FROM email_tokens WHERE user_id = $10) + \
             (SELECT COUNT(*) FROM admin_events WHERE actor_id = $11 OR target_id = $12) + \
             (SELECT COUNT(*) FROM invites WHERE created_by = $13 OR used_by = $14) AS total");
        // Le compte sert 14 fois : un paramètre par usage.
        let mut query = query;
        for _ in 0..14 {
            query = query.bind(user_id);
        }
        let row = query.fetch_one(&self.pool).await?;
        Ok(row.try_get("total")?)
    }
}

impl Store {
    /// Adresse e-mail d'un compte et si elle est vérifiée.
    pub async fn email_status(&self, user_id: i64) -> Result<Option<(String, bool)>, HubError> {
        let row = self.q(
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
        self.q("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2")
            .bind(user_id)
            .bind(purpose.as_str())
            .execute(&mut *tx)
            .await?;
        self.q(
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
        if self.backend == Backend::Mysql {
            return self
                .consume_email_token_mysql(token_hash, purpose, now)
                .await;
        }
        let row = self.q(
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

    /// Même chose sans `DELETE … RETURNING` (MySQL) : on lit le compte, puis on
    /// supprime. Seule la requête qui supprime vraiment la ligne (une ligne touchée)
    /// rend le compte : deux requêtes simultanées ne réussissent pas toutes les deux.
    async fn consume_email_token_mysql(
        &self,
        token_hash: &str,
        purpose: TokenPurpose,
        now: i64,
    ) -> Result<Option<i64>, HubError> {
        let row = self
            .q("SELECT user_id FROM email_tokens \
                WHERE token_hash = $1 AND purpose = $2 AND expires_at > $3")
            .bind(token_hash)
            .bind(purpose.as_str())
            .bind(now)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let user_id: i64 = row.try_get("user_id")?;
        let deleted = self
            .q("DELETE FROM email_tokens \
                WHERE token_hash = $1 AND purpose = $2 AND expires_at > $3")
            .bind(token_hash)
            .bind(purpose.as_str())
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok((deleted.rows_affected() == 1).then_some(user_id))
    }

    /// Retire les jetons expirés ; renvoie le nombre de lignes supprimées.
    pub async fn purge_expired_email_tokens(&self, now: i64) -> Result<u64, HubError> {
        let result = self
            .q("DELETE FROM email_tokens WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Marque l'adresse e-mail d'un compte comme vérifiée.
    pub async fn mark_email_verified(&self, user_id: i64) -> Result<(), HubError> {
        self.q("UPDATE user_identities SET verified = 1 WHERE user_id = $1 AND provider = $2")
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
        self.q("UPDATE user_identities SET secret_hash = $1, verified = 1 \
             WHERE user_id = $2 AND provider = $3")
            .bind(password_hash)
            .bind(user_id)
            .bind(EMAIL_PROVIDER)
            .execute(&mut *tx)
            .await?;
        self.q("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        self.q("DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2")
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
        let row = self
            .q("SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2")
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
        let rows = self
            .q("SELECT provider FROM user_identities WHERE user_id = $1 ORDER BY id")
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
        self.q(
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
    ///
    /// `invite_hash` : comme pour [`Store::create_account`].
    pub async fn create_account_with_identity(
        &self,
        username: &str,
        username_norm: &str,
        provider: &str,
        subject: &str,
        invite_hash: Option<&str>,
        now: i64,
    ) -> Result<i64, HubError> {
        let mut tx = self.pool.begin().await?;
        let user_id = self
            .insert_user(&mut tx, username, username_norm, now)
            .await?;
        self.q(
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
        if let Some(hash) = invite_hash {
            admin::consume_invite(self.backend, &mut tx, hash, user_id, now).await?;
        }
        tx.commit().await?;
        Ok(user_id)
    }

    /// Retire une identité d'un compte ; `false` s'il n'en avait pas.
    pub async fn unlink_identity(&self, user_id: i64, provider: &str) -> Result<bool, HubError> {
        let result = self
            .q("DELETE FROM user_identities WHERE user_id = $1 AND provider = $2")
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
    fn requetes_traduites_pour_mysql() {
        assert_eq!(
            mysql_sql("SELECT a FROM t WHERE b = $1 AND c > $2 LIMIT $10"),
            "SELECT a FROM t WHERE b = ? AND c > ? LIMIT ?"
        );
        assert_eq!(
            mysql_sql(r"WHERE n LIKE $1 ESCAPE '\'"),
            r"WHERE n LIKE ? ESCAPE '\\'"
        );
        // Un `$` qui n'est pas suivi d'un chiffre ne bouge pas.
        assert_eq!(mysql_sql("SELECT '$x'"), "SELECT '$x'");
        // Même texte, même traduction gardée.
        assert!(std::ptr::eq(
            mysql_interned("SELECT $1"),
            mysql_interned("SELECT $1")
        ));
    }

    /// MySQL n'a que des `?`, lus dans l'ordre : chaque requête doit numéroter ses
    /// paramètres `$1`, `$2`… dans l'ordre, sans en réutiliser un.
    #[test]
    fn parametres_dans_l_ordre() {
        let sources = [
            include_str!("store.rs"),
            include_str!("store/admin.rs"),
            include_str!("store/admin_auth.rs"),
        ];
        for source in sources {
            // Le code seulement, pas les tests (qui écrivent des exemples exprès).
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            // Chaque littéral de chaîne entre guillemets (les requêtes du hub).
            for literal in code.split('"').skip(1).step_by(2) {
                let numbers: Vec<u32> = literal
                    .split('$')
                    .skip(1)
                    .filter_map(|rest| {
                        let digits: String =
                            rest.chars().take_while(char::is_ascii_digit).collect();
                        digits.parse().ok()
                    })
                    .collect();
                let expected: Vec<u32> = (1..=numbers.len() as u32).collect();
                assert_eq!(numbers, expected, "requête : {literal}");
            }
        }
    }

    #[test]
    fn schema_mysql() {
        let schema = Backend::Mysql.schema();
        assert!(schema.contains("BIGINT AUTO_INCREMENT PRIMARY KEY"));
        assert!(schema.contains("username_norm VARCHAR(255) NOT NULL UNIQUE"));
        assert!(schema.contains("ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"));
        assert!(!schema.contains(" TEXT"));
        assert!(!schema.contains("IF NOT EXISTS blocks_blocked"));
        assert!(!schema.contains("CHECK ("));
        assert!(!schema.contains("{key}") && !schema.contains("{id}"));
        // SQLite et PostgreSQL gardent `TEXT` et leurs `CHECK`.
        let sqlite = Backend::Sqlite.schema();
        assert!(sqlite.contains("username_norm TEXT NOT NULL UNIQUE"));
        assert!(sqlite.contains("CHECK (user_a < user_b)"));
    }

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
        assert_eq!(
            Backend::from_url("mysql://u@h/b").ok(),
            Some(Backend::Mysql)
        );
        assert_eq!(
            Backend::from_url("mariadb://u@h/b").ok(),
            Some(Backend::Mysql)
        );
        assert!(Backend::from_url("mssql://h").is_err());
    }
}
