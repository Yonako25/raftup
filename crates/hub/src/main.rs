//! Binaire `rafthub` : sert l'API du hub et sa console d'admin en HTTP (ADR 0036).
//!
//! ```text
//! rafthub [--db sqlite://chemin.db] [--listen 127.0.0.1:8787]
//!         [--public-url https://hub.exemple.org]
//! rafthub admin create <e-mail> [--db …]   donne le rôle d'admin à un compte existant
//! rafthub admin remove <e-mail> [--db …]   le lui retire (et ferme ses sessions d'admin)
//! ```
//!
//! **Premier lancement** : sans `--db` ni `RAFTUP_HUB_DB`, le hub lit son choix dans
//! `rafthub-data/rafthub.toml` (dossier à côté de l'exe). S'il n'existe pas, `/admin`
//! ouvre un assistant : base intégrée (SQLite, `rafthub-data/rafthub.db`, rien à
//! installer) ou base externe (PostgreSQL) avec ses informations de connexion. Le
//! choix est essayé, puis gardé dans `rafthub.toml` (ADR 0037).
//!
//! **Premier admin** : tant que le hub n'a aucun admin, il écrit au journal un code
//! d'installation à usage unique ; la page `/admin/installation` crée le premier compte
//! admin avec ce code, depuis le navigateur (ADR 0037).
//!
//! **Version officielle** : compilé avec `--features officiel`, le hub peut proposer la
//! connexion Discord ; sans (Rafthub auto-hébergé), seulement pseudo ou e-mail et mot
//! de passe (ADR 0038).
//!
//! La console d'admin est sous `<public-url>/admin`. Son cookie n'a l'attribut
//! `Secure` que si `--public-url` n'est pas en `http://127.0.0.1` (ou `localhost`) :
//! derrière un proxy, donner l'adresse publique en `https://`.
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
//! | `RAFTUP_HUB_CACHE_DIR` | dossier du launcher officiel en cache (défaut : `rafthub-data/cache`) |
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
use hub::server_config::ServerConfig;
use hub::store::Role;
use hub::web::install::{install_router, InstallState};
use hub::{AppState, EnvSettings, HubConfig, SettingKey, Store};

struct Args {
    /// URL de la base ; `None` = base intégrée dans `rafthub-data/`.
    db: Option<String>,
    listen: String,
    public_url: Option<String>,
    /// Commande d'admin (`admin create|remove <e-mail>`) au lieu de lancer le serveur.
    admin: Option<(Role, String)>,
}

/// Variable d'environnement non vide.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Dossier `rafthub-data/` à côté de l'exe, créé s'il manque. On part de l'exe
/// et non du dossier courant : le hub retrouve sa base d'où qu'on le lance.
fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("chemin de l'exe")?;
    let dir = exe
        .parent()
        .context("dossier de l'exe")?
        .join("rafthub-data");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("création du dossier {}", dir.display()))?;
    Ok(dir)
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        db: env("RAFTUP_HUB_DB"),
        listen: "127.0.0.1:8787".to_string(),
        public_url: None,
        admin: None,
    };
    // Mots qui ne sont pas des options (`admin`, `create`, l'adresse…).
    let mut words = Vec::new();
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--db" => args.db = Some(iter.next().context("--db attend une URL")?),
            "--listen" => args.listen = iter.next().context("--listen attend une adresse")?,
            "--public-url" => {
                args.public_url = Some(iter.next().context("--public-url attend une URL")?)
            }
            "--help" | "-h" => {
                println!("rafthub [--db sqlite://chemin.db] [--listen 127.0.0.1:8787]");
                println!("  sans --db : base choisie au premier lancement, dans /admin");
                println!("rafthub admin create <e-mail>   donne le rôle d'admin à un compte");
                println!("rafthub admin remove <e-mail>   le lui retire");
                println!("  --db accepte aussi postgres://utilisateur:motdepasse@hote/base");
                println!("  --public-url : adresse publique du hub, pour les liens des e-mails");
                println!(
                    "  environnement : RAFTUP_HUB_DB, RAFTUP_HUB_SMTP_URL, RAFTUP_HUB_MAIL_FROM,"
                );
                println!("    RAFTUP_HUB_DISCORD_CLIENT_ID, RAFTUP_HUB_DISCORD_CLIENT_SECRET");
                std::process::exit(0);
            }
            other if other.starts_with('-') => bail!("option inconnue : {other}"),
            other => words.push(other.to_string()),
        }
    }
    // `as_slice` + motifs de tranche : on compare la liste entière des mots d'un coup.
    match words
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => {}
        ["admin", "create", email] => args.admin = Some((Role::Admin, email.to_string())),
        ["admin", "remove", email] => args.admin = Some((Role::Player, email.to_string())),
        _ => bail!("commande inconnue : attendu `rafthub admin create|remove <e-mail>`"),
    }
    Ok(args)
}

/// Assistant de premier lancement : sert seulement la page du choix de la base,
/// jusqu'à ce qu'une base soit essayée et enregistrée, puis rend son URL.
async fn run_install_wizard(
    args: &Args,
    dir: std::path::PathBuf,
    code: &str,
) -> anyhow::Result<(String, String)> {
    let public_url = args
        .public_url
        .clone()
        .or_else(|| env("RAFTUP_HUB_PUBLIC_URL"));
    // Cookie `Secure` sauf quand le hub est joint en local, comme la console.
    let secure = public_url.as_deref().is_some_and(|url| {
        !["http://127.0.0.1", "http://localhost"]
            .iter()
            .any(|local| url.starts_with(local))
    });
    // `oneshot` : un canal qui porte une seule valeur, ici l'URL de la base choisie.
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<String>();
    let state = InstallState::new(code.to_string(), dir, secure, done_tx);
    // Jeton du cookie de l'étape 1, rendu avec l'URL pour le vrai hub (étape 2).
    let pass = state.pass_token();

    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .with_context(|| format!("écoute sur {}", args.listen))?;
    tracing::warn!(
        adresse = %args.listen,
        "premier lancement : ouvrez /admin dans un navigateur, code d'installation {code}"
    );
    // Le petit serveur s'arrête dès que l'URL arrive ; on la garde au passage.
    // `Arc<Mutex<…>>` : la tâche d'arrêt écrit, `main` lit ensuite.
    let chosen = std::sync::Arc::new(tokio::sync::Mutex::new(None::<String>));
    let slot = chosen.clone();
    axum::serve(listener, install_router(state))
        .with_graceful_shutdown(async move {
            tokio::select! {
                url = done_rx => {
                    if let Ok(url) = url {
                        *slot.lock().await = Some(url);
                    }
                }
                _ = tokio::signal::ctrl_c() => {}
            }
        })
        .await
        .context("assistant d'installation")?;
    let url = chosen.lock().await.take();
    Ok((url.context("installation interrompue")?, pass))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;

    // Code d'installation, tiré seulement s'il sert (assistant ou premier admin).
    let mut setup_code: Option<String> = None;
    // Jeton du cookie posé par l'étape 1 : l'étape 2 ne redemande pas le code.
    let mut setup_pass: Option<String> = None;

    // Quelle base ? `--db` ou `RAFTUP_HUB_DB`, sinon le choix gardé dans
    // `rafthub-data/rafthub.toml`, sinon l'assistant de premier lancement (ADR 0037).
    let db = match &args.db {
        Some(url) => url.clone(),
        None => {
            let dir = data_dir()?;
            match ServerConfig::load(&dir)? {
                Some(saved) => saved.database_url_in(&dir),
                None if args.admin.is_some() => bail!(
                    "hub pas encore installé : lancez `rafthub` et ouvrez /admin dans un navigateur"
                ),
                None => {
                    let code = hub::session::new_invite_code();
                    let (url, pass) = run_install_wizard(&args, dir, &code).await?;
                    setup_code = Some(code);
                    setup_pass = Some(pass);
                    url
                }
            }
        }
    };
    // Pas d'URL dans le message d'erreur : elle peut contenir un mot de passe.
    let store = Store::open(&db)
        .await
        .context("ouverture de la base de données")?;

    if let Some((role, email)) = args.admin {
        return set_admin_role(&store, role, &email).await;
    }

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
    // Dossier du cache de l'exe officiel du launcher (page « Distribution »).
    config.distribution.cache_dir = match env("RAFTUP_HUB_CACHE_DIR") {
        Some(dir) => dir.into(),
        None => data_dir()?.join("cache"),
    };
    // Aucun admin encore : code d'installation à usage unique, montré au journal
    // seulement (celui qui le lit a accès au serveur), pour `/admin/installation`.
    // Après l'assistant, on garde le même code : la personne l'a déjà sous les yeux.
    if !store.has_admin().await? {
        let code = match setup_code {
            Some(code) => code,
            None => {
                let code = hub::session::new_invite_code();
                tracing::warn!(
                    "aucun admin : ouvrez /admin/installation avec le code d'installation {code}"
                );
                code
            }
        };
        config.setup_code = Some(code);
        config.setup_pass = setup_pass;
    }
    let state = AppState::from_settings(store.clone(), config).await?;
    // Avertissements de départ : les réglages restent modifiables ensuite.
    let settings = state.settings().await?;
    if settings.smtp_mailer().is_none() {
        tracing::warn!("SMTP non réglé : aucun e-mail ne sera envoyé");
    }
    if state.discord_allowed() && settings.discord_config().is_none() {
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
            if let Err(e) = store.purge_expired_admin_sessions(now).await {
                tracing::warn!(error = %e, "ménage des sessions d'admin expirées raté");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .with_context(|| format!("écoute sur {}", args.listen))?;
    tracing::info!(adresse = %args.listen, "rafthub démarré (console : /admin)");

    axum::serve(listener, hub::router(state))
        .with_graceful_shutdown(async {
            // Ctrl+C arrête le serveur proprement ; une erreur ici l'arrête aussi.
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// `rafthub admin create|remove <e-mail>` : change le rôle d'un compte existant.
async fn set_admin_role(store: &Store, role: Role, email: &str) -> anyhow::Result<()> {
    let email_norm = hub::identity::normalize_email(email).context("adresse e-mail invalide")?;
    if !store.set_role_by_email(&email_norm, role).await? {
        bail!("aucun compte n'a cette adresse : il faut d'abord s'inscrire depuis le launcher");
    }
    match role {
        Role::Admin => println!("{email_norm} est maintenant admin de ce hub."),
        Role::Player => println!("{email_norm} n'est plus admin (sessions d'admin fermées)."),
    }
    Ok(())
}
