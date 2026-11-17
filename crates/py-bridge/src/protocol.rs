//! Encodage des requêtes et décodage des réponses (protocole v1, ADR 0016).

use serde_json::{json, Map, Value};

use crate::error::{BridgeError, RpcError};

/// Résultat du décodage : l'id (s'il est lisible) et la valeur ou l'erreur du worker.
pub type DecodedResponse = (Option<u64>, Result<Value, RpcError>);

/// Encode une requête en une seule ligne JSON, sans `\n` final.
pub fn encode_request(id: u64, method: &str, params: &Value) -> Result<String, BridgeError> {
    let request = json!({ "id": id, "method": method, "params": params });
    // serde_json échappe les retours à la ligne dans les chaînes : le résultat tient sur une ligne.
    serde_json::to_string(&request).map_err(|e| BridgeError::Encode(e.to_string()))
}

/// Décode une réponse. L'id est `None` si le worker n'a pas pu le lire (`"id": null`).
pub fn decode_response(line: &str) -> Result<DecodedResponse, BridgeError> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| BridgeError::invalid_json(line, e.to_string()))?;
    // On travaille sur une `Map` pour tester la PRÉSENCE des clés : `"result": null` est valide.
    let Value::Object(mut map) = value else {
        return Err(BridgeError::InvalidResponse(
            "la réponse n'est pas un objet JSON".into(),
        ));
    };

    let id = read_id(&map)?;

    match (map.remove("result"), map.remove("error")) {
        (Some(result), None) => Ok((id, Ok(result))),
        (None, Some(error)) => {
            let rpc: RpcError = serde_json::from_value(error)
                .map_err(|e| BridgeError::InvalidResponse(format!("champ error invalide : {e}")))?;
            Ok((id, Err(rpc)))
        }
        (None, None) => Err(BridgeError::InvalidResponse(
            "ni result ni error dans la réponse".into(),
        )),
        (Some(_), Some(_)) => Err(BridgeError::InvalidResponse(
            "result et error présents ensemble".into(),
        )),
    }
}

/// Lit le champ `id` : absent ou `null` donne `None` ; un entier positif donne `Some`.
fn read_id(map: &Map<String, Value>) -> Result<Option<u64>, BridgeError> {
    match map.get("id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| BridgeError::InvalidResponse("id négatif ou non entier".into())),
        Some(_) => Err(BridgeError::InvalidResponse(
            "id qui n'est pas un nombre".into(),
        )),
    }
}
