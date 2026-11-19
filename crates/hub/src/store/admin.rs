//! Stockage propre à la console d'admin (ADR 0036) : rôle des comptes, sessions
//! d'admin et chiffres du tableau de bord.
//!
//! Les sessions d'admin vivent dans leur propre table, `admin_sessions` : un jeton
//! de l'API (`sessions`) ne peut donc jamais servir à entrer dans la console.

use sqlx::Row;

use super::{Store, EMAIL_PROVIDER};
use crate::error::HubError;

/// Rôle d'un compte, colonne `users.role`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Joueur ordinaire (valeur par défaut).
    Player,
    /// Peut entrer dans la console d'admin.
    Admin,
}

impl Role {
    /// Texte rangé en base.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Player => "player",
            Role::Admin => "admin",
        }
    }
}

/// Chiffres du tableau de bord tirés de la base (la présence, elle, est en mémoire).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DashboardCounts {
    /// Nombre total de comptes.
    pub accounts: i64,
    /// Comptes dont l'adresse e-mail n'est pas encore vérifiée.
    pub unverified_emails: i64,
}

impl Store {
    /// Donne un rôle au compte qui porte cette adresse e-mail (déjà normalisée).
    /// Renvoie `false` si aucun compte n'a cette adresse.
    pub async fn set_role_by_email(&self, email_norm: &str, role: Role) -> Result<bool, HubError> {
        let result = sqlx::query(
            "UPDATE users SET role = $1 WHERE id = \
             (SELECT user_id FROM user_identities WHERE provider = $2 AND subject = $3)",
        )
        .bind(role.as_str())
        .bind(EMAIL_PROVIDER)
        .bind(email_norm)
        .execute(&self.pool)
        .await?;
        let found = result.rows_affected() > 0;
        // Un compte qui perd le rôle d'admin perd aussi ses sessions d'admin.
        if found && role == Role::Player {
            sqlx::query(
                "DELETE FROM admin_sessions WHERE user_id = \
                 (SELECT user_id FROM user_identities WHERE provider = $1 AND subject = $2)",
            )
            .bind(EMAIL_PROVIDER)
            .bind(email_norm)
            .execute(&self.pool)
            .await?;
        }
        Ok(found)
    }

    /// Vrai si le compte a le rôle d'admin.
    pub async fn is_admin(&self, user_id: i64) -> Result<bool, HubError> {
        let row = sqlx::query("SELECT role FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => {
                let role: String = row.try_get("role")?;
                Ok(role == Role::Admin.as_str())
            }
            None => Ok(false),
        }
    }

    pub async fn insert_admin_session(
        &self,
        token_hash: &str,
        user_id: i64,
        now: i64,
        expires_at: i64,
    ) -> Result<(), HubError> {
        sqlx::query(
            "INSERT INTO admin_sessions (token_hash, user_id, created_at, expires_at) \
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

    /// Compte et pseudo d'une session d'admin encore valide, **si le compte est
    /// toujours admin** : retirer le rôle ferme aussitôt l'accès.
    pub async fn admin_session_user(
        &self,
        token_hash: &str,
        now: i64,
    ) -> Result<Option<(i64, String)>, HubError> {
        let row = sqlx::query(
            "SELECT users.id AS id, users.username AS username FROM admin_sessions \
             JOIN users ON users.id = admin_sessions.user_id \
             WHERE admin_sessions.token_hash = $1 AND admin_sessions.expires_at > $2 \
             AND users.role = $3",
        )
        .bind(token_hash)
        .bind(now)
        .bind(Role::Admin.as_str())
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(row) => Ok(Some((row.try_get("id")?, row.try_get("username")?))),
            None => Ok(None),
        }
    }

    pub async fn delete_admin_session(&self, token_hash: &str) -> Result<(), HubError> {
        sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Supprime les sessions d'admin expirées ; renvoie leur nombre.
    pub async fn purge_expired_admin_sessions(&self, now: i64) -> Result<u64, HubError> {
        let result = sqlx::query("DELETE FROM admin_sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Chiffres du tableau de bord.
    pub async fn dashboard_counts(&self) -> Result<DashboardCounts, HubError> {
        let accounts: i64 = sqlx::query("SELECT COUNT(*) AS n FROM users")
            .fetch_one(&self.pool)
            .await?
            .try_get("n")?;
        let unverified_emails: i64 = sqlx::query(
            "SELECT COUNT(*) AS n FROM user_identities WHERE provider = $1 AND verified = 0",
        )
        .bind(EMAIL_PROVIDER)
        .fetch_one(&self.pool)
        .await?
        .try_get("n")?;
        Ok(DashboardCounts {
            accounts,
            unverified_emails,
        })
    }

    /// Vérifie que la base répond (état des services du tableau de bord).
    pub async fn ping(&self) -> Result<(), HubError> {
        sqlx::query("SELECT 1 AS one").fetch_one(&self.pool).await?;
        Ok(())
    }
}
