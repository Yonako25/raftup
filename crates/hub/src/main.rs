//! Binaire `rafthub` : sert l'API du hub et sa console d'admin, en HTTP ou en HTTPS
//! (ADR 0036, 0039).
//!
//! ```text
//! rafthub [--db sqlite://chemin.db] [--listen 127.0.0.1:8787]
//!         [--public-url https://hub.exemple.org]
//!         [--tls-cert fullchain.pem --tls-key privkey.pem]
//!         [--tls-acme hub.exemple.org [--tls-acme-email moi@exemple.org] [--tls-acme-staging]]
//! rafthub admin create <e-mail> [--db …]   donne le rôle d'admin à un compte existant
//! rafthub admin remove <e-mail> [--db …]   le lui retire (et ferme ses sessions d'admin)
//! rafthub healthcheck [--listen …]         vérifie qu'un hub répond (sonde Docker)
//! ```
//!
//! **Premier lancement** : sans `--db` ni `RAFTUP_HUB_DB`, le hub lit son choix dans
//! `rafthub-data/rafthub.toml` (dossier à côté de l'exe, ou celui de
//! `RAFTUP_HUB_DATA_DIR`). S'il n'existe pas, `/admin`
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
//! **HTTP ou HTTPS** (ADR 0039), au choix :
//! - sans option TLS, le hub parle HTTP en clair : on le place derrière un proxy
//!   (Traefik, Caddy, nginx) qui fait le HTTPS ;
//! - `--tls-cert` + `--tls-key` : le hub sert lui-même en HTTPS avec ces fichiers
//!   PEM (par exemple ceux de certbot ; redémarrer après un renouvellement) ;
//! - `--tls-acme <domaine>` : certificat Let's Encrypt obtenu et renouvelé tout seul
//!   (défi TLS-ALPN-01 : le hub doit être joignable sur le port 443, par exemple
//!   `--listen 0.0.0.0:443`), rangé dans `rafthub-data/acme/`. `--tls-acme-staging`
//!   prend le serveur d'essai de Let's Encrypt, pour essayer sans risquer ses limites.
//!
//! L'assistant d'installation est servi de la même façon (HTTPS compris).
//!
//! La console d'admin est sous `<public-url>/admin`. Son cookie a l'attribut `Secure`
//! en HTTPS intégré, et sinon dès que `--public-url` n'est pas en `http://127.0.0.1`
//! (ou `localhost`) : derrière un proxy, donner l'adresse publique en `https://`.
//!
//! `--db` accepte aussi une base PostgreSQL : `postgres://utilisateur:motdepasse@hote/base`.
//! Les secrets passent plutôt par l'environnement (invisibles dans la liste des process) :
//!
//! | Variable | Rôle |
//! | --- | --- |
//! | `RAFTUP_HUB_DB` | URL de la base (remplace `--db`) |
//! | `RAFTUP_HUB_DATA_DIR` | dossier de données (défaut : `rafthub-data/` à côté de l'exe) |
//! | `RAFTUP_HUB_SMTP_URL` | serveur d'envoi : `smtps://utilisateur:motdepasse@hote:465` |
//! | `RAFTUP_HUB_MAIL_FROM` | expéditeur : `Raftup <no-reply@exemple.org>` |
//! | `RAFTUP_HUB_DISCORD_CLIENT_ID` | application Discord (portail développeur) |
//! | `RAFTUP_HUB_DISCORD_CLIENT_SECRET` | secret de cette application |
//! | `RAFTUP_HUB_PUBLIC_URL` | adresse publique (comme `--public-url`) |
//! | `RAFTUP_HUB_NAME` | nom du hub affiché (défaut : Rafthub) |
//! | `RAFTUP_HUB_REGISTRATION` | inscriptions : `open`, `invite` ou `closed` |
//! | `RAFTUP_HUB_PUBLIC_DOWNLOAD` | page publique de téléchargement : `true` ou `false` |
//! | `RAFTUP_HUB_CACHE_DIR` | dossier du launcher officiel en cache (défaut : `rafthub-data/cache`) |
//! | `RAFTUP_HUB_TLS_CERT` | certificat PEM (comme `--tls-cert`) |
//! | `RAFTUP_HUB_TLS_KEY` | clé privée PEM (comme `--tls-key`) |
//! | `RAFTUP_HUB_TLS_ACME` | domaine pour Let's Encrypt (comme `--tls-acme`) |
//! | `RAFTUP_HUB_TLS_ACME_EMAIL` | e-mail de contact pour Let's Encrypt |
//!
//! Ces réglages se changent aussi depuis la console d'admin (ADR 0036) : la valeur
//! écrite en base passe avant la variable d'environnement, qui passe avant le défaut.
//! SMTP et Discord sont relus à chaque usage : pas besoin de redémarrer.
//! Sans SMTP, aucun e-mail ne part (avertissement au journal).
//! Sans application Discord, la connexion Discord répond `discord_disabled`.
//! L'adresse de retour à déclarer chez Discord est `<public-url>/v1/oauth/discord/callback`.
//!
//! Déploiement Docker derrière Traefik (HTTP en clair derrière le proxy) : `deploy/rafthub/`.
//! Le hub s'arrête proprement sur Ctrl+C et, sous Linux, sur SIGTERM (`docker stop`).

use anyhow::{bail, Context};
use hub::server_config::ServerConfig;
use hub::store::Role;
use hub::tls::{AcmeOptions, TlsSetup};
use hub::web::install::{install_router, InstallState};
use hub::{AppState, EnvSettings, HubConfig, SettingKey, Store};

struct Args {
    /// URL de la base ; `None` = base intégrée dans `rafthub-data/`.
    db: Option<String>,
    listen: String,
    public_url: Option<String>,
    /// Commande d'admin (`admin create|remove <e-mail>`) au lieu de lancer le serveur.
    admin: Option<(Role, String)>,
    /// `rafthub healthcheck` : sonde un hub déjà lancé au lieu d'en lancer un.
    healthcheck: bool,
    /// HTTPS par fichiers PEM : certificat (avec sa chaîne) et clé privée (ADR 0039).
    tls_cert: Option<String>,
    tls_key: Option<String>,
    /// HTTPS par Let's Encrypt : nom de domaine, e-mail de contact, serveur d'essai.
    tls_acme: Option<String>,
    tls_acme_email: Option<String>,
    tls_acme_staging: bool,
}

/// Variable d'environnement non vide.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Dossier de données, créé s'il manque : `RAFTUP_HUB_DATA_DIR` s'il est donné
/// (conteneur Docker : un volume), sinon `rafthub-data/` à côté de l'exe. On part de
/// l'exe et non du dossier courant : le hub retrouve sa base d'où qu'on le lance.
fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("chemin de l'exe")?;
    let from_env = env(hub::server_config::DATA_DIR_ENV);
    let dir = hub::server_config::resolve_data_dir(from_env.as_deref(), &exe)
        .context("dossier de l'exe")?;
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
        healthcheck: false,
        tls_cert: env("RAFTUP_HUB_TLS_CERT"),
        tls_key: env("RAFTUP_HUB_TLS_KEY"),
        tls_acme: env("RAFTUP_HUB_TLS_ACME"),
        tls_acme_email: env("RAFTUP_HUB_TLS_ACME_EMAIL"),
        tls_acme_staging: false,
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
            "--tls-cert" => {
                args.tls_cert = Some(iter.next().context("--tls-cert attend un fichier .pem")?)
            }
            "--tls-key" => {
                args.tls_key = Some(iter.next().context("--tls-key attend un fichier .pem")?)
            }
            "--tls-acme" => {
                args.tls_acme = Some(iter.next().context("--tls-acme attend un nom de domaine")?)
            }
            "--tls-acme-email" => {
                args.tls_acme_email =
                    Some(iter.next().context("--tls-acme-email attend une adresse")?)
            }
            "--tls-acme-staging" => args.tls_acme_staging = true,
            "--help" | "-h" => {
                println!("rafthub [--db sqlite://chemin.db] [--listen 127.0.0.1:8787]");
                println!("  sans --db : base choisie au premier lancement, dans /admin");
                println!("rafthub admin create <e-mail>   donne le rôle d'admin à un compte");
                println!("rafthub admin remove <e-mail>   le lui retire");
                println!("rafthub healthcheck [--listen …] vérifie qu'un hub répond");
                println!("  --db accepte aussi postgres://utilisateur:motdepasse@hote/base");
                println!("  --public-url : adresse publique du hub, pour les liens des e-mails");
                println!(
                    "  environnement : RAFTUP_HUB_DB, RAFTUP_HUB_SMTP_URL, RAFTUP_HUB_MAIL_FROM,"
                );
                println!("    RAFTUP_HUB_DISCORD_CLIENT_ID, RAFTUP_HUB_DISCORD_CLIENT_SECRET");
                println!("HTTPS intégré (sinon HTTP, derrière un proxy) :");
                println!("  --tls-cert fullchain.pem --tls-key privkey.pem   certificat fourni");
                println!("  --tls-acme hub.exemple.org [--tls-acme-email moi@exemple.org]");
                println!("    certificat Let's Encrypt automatique (port 443 joignable)");
                println!("  --tls-acme-staging   serveur d'essai de Let's Encrypt");
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
        ["healthcheck"] => args.healthcheck = true,
        _ => bail!(
            "commande inconnue : attendu `rafthub admin create|remove <e-mail>` ou `rafthub healthcheck`"
        ),
    }
    Ok(args)
}

/// Assistant de premier lancement : sert seulement la page du choix de la base,
/// jusqu'à ce qu'une base soit essayée et enregistrée, puis rend son URL.
async fn run_install_wizard(
    args: &Args,
    tls: Option<TlsSetup>,
    dir: std::path::PathBuf,
    code: &str,
) -> anyhow::Result<(String, String)> {
    let public_url = args
        .public_url
        .clone()
        .or_else(|| env("RAFTUP_HUB_PUBLIC_URL"));
    // Cookie `Secure` en HTTPS intégré, et sauf quand le hub est joint en local
    // en HTTP, comme la console.
    let secure = tls.is_some()
        || public_url.as_deref().is_some_and(|url| {
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
    // En HTTPS si `tls` est donné, comme le vrai hub ensuite.
    hub::tls::serve(listener, tls, install_router(state), async move {
        tokio::select! {
            url = done_rx => {
                if let Ok(url) = url {
                    *slot.lock().await = Some(url);
                }
            }
            _ = shutdown_signal() => {}
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

    // Sonde de santé : pas de base, pas de serveur, pas de TLS ; code de sortie 0 ou 1.
    if args.healthcheck {
        hub::health::check(&args.listen, std::time::Duration::from_secs(5))?;
        return Ok(());
    }

    // HTTPS intégré, lu avant tout : un certificat illisible arrête le hub tout de suite.
    let tls = if args.admin.is_some() {
        None
    } else {
        tls_setup(&args)?
    };

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
                    let (url, pass) = run_install_wizard(&args, tls.clone(), dir, &code).await?;
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
        https: tls.is_some(),
        ..HubConfig::default()
    };
    // Avec Let's Encrypt, l'adresse publique par défaut est celle du domaine.
    if let Some(domain) = &args.tls_acme {
        config.public_url = format!("https://{domain}");
    }
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
    if tls.is_some() && !settings.public_url.starts_with("https://") {
        tracing::warn!(
            "HTTPS activé mais adresse publique en http:// : donnez --public-url https://…"
        );
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
    let scheme = if tls.is_some() { "https" } else { "http" };
    tracing::info!(adresse = %args.listen, "rafthub démarré en {scheme} (console : /admin)");

    hub::tls::serve(listener, tls, hub::router(state), shutdown_signal()).await?;
    Ok(())
}

/// Attend une demande d'arrêt : Ctrl+C partout, et SIGTERM sous Linux (c'est ce
/// qu'envoie `docker stop`). Sans SIGTERM, le hub, process n° 1 du conteneur,
/// ignorerait le signal et Docker finirait par le tuer au bout de 10 s.
async fn shutdown_signal() {
    // `#[cfg(unix)]` : ce bloc n'est compilé que sous Linux / macOS.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            // Impossible d'écouter SIGTERM : Ctrl+C seulement.
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        // Une erreur ici arrête aussi le serveur.
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("arrêt demandé");
}

/// HTTPS intégré (ADR 0039) d'après les options : `None` = HTTP en clair (proxy).
fn tls_setup(args: &Args) -> anyhow::Result<Option<TlsSetup>> {
    match (&args.tls_cert, &args.tls_key, &args.tls_acme) {
        (None, None, None) => Ok(None),
        (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
            bail!("choisir --tls-cert/--tls-key ou --tls-acme, pas les deux")
        }
        (Some(cert), Some(key), None) => {
            let setup = TlsSetup::from_pem_files(cert.as_ref(), key.as_ref())
                .context("HTTPS (--tls-cert, --tls-key)")?;
            Ok(Some(setup))
        }
        (Some(_), None, None) => bail!("--tls-cert demande aussi --tls-key (la clé privée)"),
        (None, Some(_), None) => bail!("--tls-key demande aussi --tls-cert (le certificat)"),
        (None, None, Some(domain)) => {
            let options = AcmeOptions {
                domain: domain.clone(),
                contact_email: args.tls_acme_email.clone(),
                cache_dir: data_dir()?.join("acme"),
                staging: args.tls_acme_staging,
            };
            tracing::info!(domaine = %domain, "HTTPS : certificat Let's Encrypt demandé");
            Ok(Some(
                TlsSetup::acme(&options).context("HTTPS (--tls-acme)")?,
            ))
        }
    }
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
