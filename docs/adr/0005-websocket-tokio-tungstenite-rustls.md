# 0005 — WebSocket : tokio-tungstenite et rustls

- Statut : accepté
- Date : 2026-11-15

## Contexte

`client-core` (J0) puis `server` (J1) parlent le protocole Archipelago sur WebSocket, chiffré (`wss`) ou non (`ws`).
La spec fixe tokio et « wss par défaut » (NF06), mais ni la bibliothèque WebSocket, ni la bibliothèque TLS.
Il faut aussi décider ce que fait le client quand l'adresse saisie n'a pas de schéma (par exemple `archipelago.gg:38281`), ce qui est le cas le plus courant pour les joueurs.

## Décision

- **WebSocket** : `tokio-tungstenite`, la bibliothèque WebSocket la plus utilisée avec tokio. Elle servira au client et au serveur.
- **TLS** : `rustls`, écrit en Rust : pas d'OpenSSL à installer, le même comportement sur tous les OS, et un binaire autonome (NF10). Les certificats racine viennent de `webpki-roots`, intégrés au binaire.
- **Adresse sans schéma** : le client essaie `wss://`, puis `ws://` si la connexion chiffrée échoue, avec un avertissement dans les logs. C'est le comportement du client officiel (`CommonClient.py`).
- **Adresse avec schéma** (`wss://` ou `ws://`) : utilisée telle quelle, sans essai de secours.

## Conséquences

- Un seul jeu de bibliothèques réseau pour le client et le serveur.
- Les serveurs locaux en clair (serveur Python lancé à la main, tests) marchent sans taper `ws://`.
- Le secours en `ws://` envoie le mot de passe de la salle en clair si le serveur n'a pas de TLS : c'est le même risque qu'avec le client officiel. Le launcher pourra l'afficher.
- Les certificats racine intégrés doivent être mis à jour avec les dépendances.
- **Risque : pas de compression.** tungstenite (0.30) ne gère pas la compression WebSocket (`permessage-deflate`). Le serveur officiel 0.6.8 accepte ces clients, mais il prévient : « your client does not support compressed websocket connections! It may stop working in the future ». On garde tungstenite pour l'instant (décision du 2026-11-15). Il faut revoir la question avant le serveur (J1) : soit tungstenite gère la compression d'ici là, soit on envisage une autre bibliothèque (par exemple `yawc`).
