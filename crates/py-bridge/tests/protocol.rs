use py_bridge::{
    decode_response, encode_request, BridgeError, RpcError, WORKER_PROTOCOL_VERSION, WORKER_SCRIPT,
};
use serde_json::{json, Value};

#[test]
fn requete_sur_une_seule_ligne() {
    let params = json!({"texte": "a\nb\r\nc", "n": 1});
    let line = encode_request(7, "echo", &params).unwrap();
    assert!(!line.contains('\n') && !line.contains('\r'));
    let back: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(back["id"], 7);
    assert_eq!(back["method"], "echo");
    assert_eq!(back["params"], params);
}

#[test]
fn reponse_result() {
    let (id, res) = decode_response(r#"{"id":3,"result":{"a":1}}"#).unwrap();
    assert_eq!(id, Some(3));
    assert_eq!(res.unwrap(), json!({"a": 1}));
}

#[test]
fn reponse_result_null_est_ok() {
    let (id, res) = decode_response(r#"{"id":1,"result":null}"#).unwrap();
    assert_eq!(id, Some(1));
    assert_eq!(res.unwrap(), Value::Null);
}

#[test]
fn reponse_error_sans_data() {
    let (_, res) =
        decode_response(r#"{"id":2,"error":{"code":-32601,"message":"inconnue"}}"#).unwrap();
    let err = res.unwrap_err();
    assert_eq!(err.code, -32601);
    assert_eq!(err.message, "inconnue");
    assert_eq!(err.data, None);
}

#[test]
fn reponse_error_avec_data() {
    let (_, res) =
        decode_response(r#"{"id":2,"error":{"code":1,"message":"m","data":{"x":true}}}"#).unwrap();
    assert_eq!(res.unwrap_err().data, Some(json!({"x": true})));
}

#[test]
fn reponse_error_avec_id_null() {
    let (id, res) =
        decode_response(r#"{"id":null,"error":{"code":-32700,"message":"parse"}}"#).unwrap();
    assert_eq!(id, None);
    assert!(res.is_err());
}

#[test]
fn ni_result_ni_error() {
    assert!(matches!(
        decode_response(r#"{"id":1}"#),
        Err(BridgeError::InvalidResponse(_))
    ));
}

#[test]
fn result_et_error_ensemble() {
    assert!(matches!(
        decode_response(r#"{"id":1,"result":1,"error":{"code":1,"message":"m"}}"#),
        Err(BridgeError::InvalidResponse(_))
    ));
}

#[test]
fn json_tronque() {
    assert!(matches!(
        decode_response(r#"{"id":1,"res"#),
        Err(BridgeError::InvalidJson { .. })
    ));
}

#[test]
fn json_invalide_tronque_la_ligne() {
    let long = "x".repeat(500);
    match decode_response(&long) {
        Err(BridgeError::InvalidJson { line, .. }) => assert_eq!(line.chars().count(), 200),
        autre => panic!("attendu InvalidJson, obtenu {autre:?}"),
    }
}

#[test]
fn id_negatif_ou_texte() {
    assert!(matches!(
        decode_response(r#"{"id":-1,"result":1}"#),
        Err(BridgeError::InvalidResponse(_))
    ));
    assert!(matches!(
        decode_response(r#"{"id":"abc","result":1}"#),
        Err(BridgeError::InvalidResponse(_))
    ));
}

#[test]
fn pas_un_objet() {
    assert!(matches!(
        decode_response("[1,2]"),
        Err(BridgeError::InvalidResponse(_))
    ));
}

#[test]
fn script_worker_petit() {
    assert!(WORKER_SCRIPT.len() < 16 * 1024);
    assert_eq!(WORKER_PROTOCOL_VERSION, 1);
}

#[test]
fn messages_en_francais() {
    let rpc = RpcError {
        code: -32601,
        message: "inconnue".into(),
        data: None,
    };
    assert_eq!(rpc.to_string(), "erreur -32601 du worker : inconnue");
    assert_eq!(
        BridgeError::from(rpc).to_string(),
        "erreur -32601 du worker : inconnue"
    );
    assert_eq!(BridgeError::Closed.to_string(), "le pont est fermé");
    assert_eq!(
        BridgeError::LineTooLong { max: 10 }.to_string(),
        "ligne trop longue : plus de 10 octets"
    );
    assert!(BridgeError::UnexpectedId {
        expected: 1,
        got: Some(2)
    }
    .to_string()
    .contains("id inattendu"));
    assert!(BridgeError::IncompatibleWorker {
        expected: 1,
        got: 2
    }
    .to_string()
    .contains("incompatible"));
}
