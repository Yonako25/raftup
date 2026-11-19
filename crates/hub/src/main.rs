//! Binaire `raftup-hub` : sert l'API du hub en HTTP.
//!
//! ```text
//! raftup-hub [--db sqlite://raftup-hub.db] [--listen 127.0.0.1:8787]
//!            [--public-url https://hub.exemple.org]
//! ```
//!
//! `--db` accepte aussi une base PostgreSQL : `postgres://utilisateur:motdepasse@hote/base`.
//! Les secrets passent plutôt par l'environnement (invisibles dans la liste des process) :
//!
//! | Variable | Rôle |
//! | --- | --- |
//! | `RAFTUP_HUB_DB` | URL de la base (remplace `--db`) |
//! | `RAFTUP_HUB_SMTP_URL` | serveur d'envoi : `smtps://utilisateur:motdepasse@hote:465` |
//! | `RAFTUP_HUB_MAIL_FROM` | expéditeur : `Raftup <no-reply@exemple.org>` |
//! | `RAFTUP_HUB_DISCORD_CLIENT_ID` | application Discord (portail développeur) |
//! | `RAFTUP_HUB_DISCORD_CLIENT_SECRET` | secret de cette application |
//! | `RAFTUP_HUB_PUBLIC_URL` | adresse publique (comme `--public-url`) |
//! | `RAFTUP_HUB_NAME` | nom du hub affiché (défaut : Rafthub) |
//! | `RAFTUP_HUB_REGISTRATION` | inscriptions : `open`, `invite` ou `closed` |
//! | `RAFTUP_HUB_PUBLIC_DOWNLOAD` | page publique de téléchargement : `true` ou `false` |
//!
//! Ces réglages se changent aussi depuis la console d'admin (ADR 0036) : la valeur
//! écrite en base passe avant la variable d'environnement, qui passe avant le défaut.
//! SMTP et Discord sont relus à chaque usage : pas besoin de redémarrer.
//! Sans SMTP, aucun e-mail ne part (avertissement au journal).
//! Sans application Discord, la connexion Discord répond `discord_disabled`.
//! L'adresse de retour à déclarer chez Discord est `<public-url>/v1/oauth/discord/callback`.
//!
//! Le hub parle HTTP en clair : en production, il se place derrière un proxy
//! qui fait le TLS (ADR 0033). Pas de déploiement pour l'instant.

use anyhow::{bail, Context};
use hub::{AppState, EnvSettings, HubConfig, SettingKey, Store};

struct Args {
    db: String,
    listen: String,
    public_url: Option<String>,
}

/// Variable d'environnement non vide.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        db: env("RAFTUP_HUB_DB").unwrap_or_else(|| "sqlite://raftup-hub.db".to_string()),
        listen: "127.0.0.1:8787".to_string(),
        public_url: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => args.db = iter.next().context("--db attend une URL")?,
            "--listen" => args.listen = iter.next().context("--listen attend une adresse")?,
            "--public-url" => {
                args.public_url = Some(iter.next().context("--public-url attend une URL")?)
            }
            "--help" | "-h" => {
                println!("raftup-hub [--db sqlite://raftup-hub.db] [--listen 127.0.0.1:8787]");
                println!("  --db accepte aussi postgres://utilisateur:motdepasse@hote/base");
                println!("  --public-url : adresse publique du hub, pour les liens des e-mails");
                println!(
                    "  environnement : RAFTUP_HUB_DB, RAFTUP_HUB_SMTP_URL, RAFTUP_HUB_MAIL_FROM,"
                );
                println!("    RAFTUP_HUB_DISCORD_CLIENT_ID, RAFTUP_HUB_DISCORD_CLIENT_SECRET");
                std::process::exit(0);
            }
            other => bail!("argument inconnu : {other}"),
        }
    }
    Ok(args)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;

    // Pas d'URL dans le message d'erreur : elle peut contenir un mot de passe.
    let store = Store::open(&args.db)
        .await
        .context("ouverture de la base de données")?;

    // Réglages de l'environnement ; `--public-url` passe avant la variable.
    let mut config = HubConfig {
        settings_env: EnvSettings::from_env(),
        ..HubConfig::default()
    };
    if let Some(url) = args.public_url {
        config
            .settings_env
            .set(SettingKey::PublicUrl, &url)
            .context("--public-url")?;
    }
    let state = AppState::from_settings(store.clone(), config).await?;
    // Avertissements de départ : les réglages restent modifiables ensuite.
    let settings = state.settings().await?;
    if settings.smtp_mailer().is_none() {
        tracing::warn!("SMTP non réglé : aucun e-mail ne sera envoyé");
    }
    if settings.discord_config().is_none() {
        tracing::warn!("application Discord non réglée : connexion Discord désactivée");
    }

    // Ménage des sessions et jetons expirés, une fois par heure, dans une tâche à part.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let now = hub::now_unix();
            if let Err(e) = store.purge_expired_sessions(now).await {
                tracing::warn!(error = %e, "ménage des sessions expirées raté");
            }
            if let Err(e) = store.purge_expired_email_tokens(now).await {
                tracing::warn!(error = %e, "ménage des jetons d'e-mail expirés raté");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .with_context(|| format!("écoute sur {}", args.listen))?;
    tracing::info!(adresse = %args.listen, "raftup-hub démarré");

    axum::serve(listener, hub::router(state))
        .with_graceful_shutdown(async {
            // Ctrl+C arrête le serveur proprement ; une erreur ici l'arrête aussi.
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
