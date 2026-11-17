//! Faux worker pour les tests du pont, sans Python (ADR 0016).
//!
//! Il parle le protocole v1 : une ligne JSON par requête, une ligne par réponse.
//! Le comportement est choisi par la variable d'environnement `RAFTUP_FAKE_WORKER`
//! (défaut `ok`). Les arguments de la ligne de commande sont ignorés.
//!
//! Par défaut, le PREMIER appel (la poignée de main `version` faite par `PyBridge::start`)
//! reçoit une réponse normale, puis le mode s'applique aux appels suivants. Avec
//! `RAFTUP_FAKE_WORKER_AFTER_HANDSHAKE=0`, le mode s'applique dès le premier appel
//! (pour tester les échecs au démarrage).
//!
//! Modes : `ok`, `error` (ping refusé), `old-protocol`, `crash-on-start`, `crash`,
//! `noisy-stderr`, `noisy-crash`, `garbage`, `huge`, `silent`, `wrong-id`, `null`
//! (`ping` renvoie `result: null`), `null-id-error` (`id: null` avec `error`),
//! `bad-version` (`version` mal formée), `no-read` (ne lit plus stdin), `ignore-eof`.
//! Binaire de test : volontairement simple, il travaille avec `serde_json::Value`.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};

fn main() -> io::Result<()> {
    // Mode de fonctionnement ; `unwrap_or_else` donne la valeur par défaut si la variable est absente.
    let mode = std::env::var("RAFTUP_FAKE_WORKER").unwrap_or_else(|_| "ok".to_string());
    let after_handshake =
        std::env::var("RAFTUP_FAKE_WORKER_AFTER_HANDSHAKE").map_or(true, |v| v != "0");

    if mode == "crash-on-start" {
        std::process::exit(4);
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut count: u32 = 0;

    // `lines()` s'arrête à EOF de stdin : on sort alors proprement.
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        count += 1;
        let first = count == 1;
        // Le mode « défaillant » est actif selon le réglage de poignée de main.
        let active = !after_handshake || !first;

        if first && (mode == "noisy-stderr" || mode == "noisy-crash") {
            // 2 Mio de bruit sur stderr, par blocs de 1 Kio.
            let err = io::stderr();
            let mut err = err.lock();
            let chunk = format!("{}\n", "x".repeat(1023));
            for _ in 0..2048 {
                err.write_all(chunk.as_bytes())?;
            }
            err.flush()?;
        }
        if active && (mode == "crash" || mode == "noisy-crash") {
            eprintln!("boom");
            std::process::exit(3);
        }

        let reply = match mode.as_str() {
            "silent" if active => continue,
            "garbage" if active => "ceci n'est pas du JSON".to_string(),
            "huge" if active => "x".repeat(2 * 1024 * 1024),
            "null-id-error" if active => {
                json!({"id": null, "error": {"code": -32700, "message": "JSON illisible"}})
                    .to_string()
            }
            _ => build_reply(&mode, active, &req).to_string(),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;

        if mode == "no-read" {
            // Ne lit plus jamais stdin ; le pont doit nous tuer.
            std::thread::sleep(Duration::from_secs(60));
            return Ok(());
        }
    }

    if mode == "ignore-eof" {
        std::thread::sleep(Duration::from_secs(60));
    }
    Ok(())
}

/// Construit la réponse JSON pour une requête, selon le mode (`active` : mode défaillant actif).
fn build_reply(mode: &str, active: bool, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    // Mauvais identifiant : on décale l'id de 100.
    let id = if mode == "wrong-id" && active {
        json!(id.as_i64().unwrap_or(0) + 100)
    } else {
        id
    };

    let result = match method {
        "ping" if mode == "error" => {
            return json!({"id": id, "error": {"code": -32000, "message": "ping refusé"}});
        }
        "ping" if mode == "null" => Value::Null,
        "ping" => json!("pong"),
        "version" if mode == "bad-version" && active => json!({"x": 1}),
        "version" => {
            let protocol = if mode == "old-protocol" { 0 } else { 1 };
            json!({"python": "3.12.0", "implementation": "Fake", "protocol": protocol})
        }
        "echo" => req.get("params").cloned().unwrap_or(Value::Null),
        _ => {
            return json!({"id": id, "error": {"code": -32601, "message": "méthode inconnue"}});
        }
    };
    json!({"id": id, "result": result})
}
