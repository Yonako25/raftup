# Paquets capturés

Vecteurs de test pour `ap-protocol` (ADR 0004) : de vrais paquets envoyés par le serveur officiel.

## `serveur-0.6.8/`

- Serveur : `MultiServer.py` d'Archipelago 0.6.8 (code MIT), lancé en local avec `--disable_save`.
- Partie : seed solo Slime Rancher générée à l'étape 0 (apworld `slime_rancher` 0.3.2, seed `12345`), slot `Rancher1`.
- Date : 15 novembre 2026.
- Méthode : un script Python se connecte en WebSocket, envoie une suite de commandes (`GetDataPackage`, `Connect`, `LocationScouts`, `LocationChecks`, `Say`, `Set`, `Get`, `Bounce`, `Sync`, `ConnectUpdate`, paquet invalide, `StatusUpdate` objectif) et enregistre les trames reçues.

Chaque fichier contient une trame : un tableau JSON avec le premier paquet reçu de ce type, écrit sans espaces comme le serveur.
`trame_multiple.json` est une trame brute de plusieurs paquets, recopiée telle quelle.

**Modification** : dans `DataPackage.json`, les listes `item_name_to_id` et `location_name_to_id` sont réduites à 3 entrées.
Les noms viennent de l'apworld, publié sans licence ; on n'en garde que le minimum. Le `checksum` ne correspond donc plus au contenu.

## `client-0.6.8/`

- Paquets client → serveur écrits par l'encodeur officiel (`NetUtils.encode` d'Archipelago 0.6.8), avec les mêmes valeurs que le client officiel `CommonClient.py`.
- Chaque trame a ensuite été envoyée au serveur officiel (même partie, slot `Rancher1`) : **aucune** n'a été refusée (ni `InvalidPacket`, ni `ConnectionRefused`).
- `Get_SetNotify.json` contient deux paquets dans une même trame, comme le client officiel les envoie.
- `Bounce_Ping.json` est un `Bounce` sans `data`, que le client officiel envoie vers son propre slot.

## Ce que le serveur n'envoie pas

- Une commande inconnue (`{"cmd": "CommandeInconnue"}`) ne reçoit **aucune** réponse : le serveur 0.6.8 n'envoie pas d'`InvalidPacket` de type `cmd` dans ce cas.
- `SetNotify` ne reçoit pas de réponse directe ; seul le `Set` suivant déclenche un `SetReply`.
