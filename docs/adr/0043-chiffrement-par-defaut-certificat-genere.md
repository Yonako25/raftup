# 0043 — Chiffrement par défaut sans fichier PEM : certificat généré par Raftup

- Statut : accepté (choix de l'utilisatrice le 21 novembre 2026 ; mise en œuvre à faire)
- Date : 2026-11-21
- Complète l'ADR 0009 (wss par certificat PEM) et l'ADR 0020 (liens `raftup://`)

## Contexte

Le plan de l'interface (`docs/plan-interface-launcher.md`, étape 1) veut un chiffrement par défaut qui ne demande aucun fichier.
Aujourd'hui, « Créer une partie » et « Héberger » proposent wss par défaut, mais wss exige un certificat et une clé PEM liés à un nom de domaine (ADR 0009). Un joueur chez lui n'en a pas : il est bloqué dès le premier clic, ou doit choisir « En clair (ws) » lui-même.

Contrainte non négociable : un mod Archipelago non modifié doit pouvoir se connecter. Les clients officiels essaient `wss://` puis `ws://`. Un certificat auto-signé fait échouer leur `wss`.

Options examinées :
1. certificat généré par Raftup, avec ws et wss sur le même port ;
2. clair par défaut, wss PEM en option avancée ;
3. Let's Encrypt automatique (demande un nom de domaine et le port 80 ouvert : hors de portée d'un joueur chez lui).

## Décision

Option 1.

- **Certificat généré** : au premier hébergement sans PEM, Raftup crée un certificat auto-signé et sa clé (`rcgen`, aujourd'hui dépendance de test, qui passe en dépendance normale), rangés dans son dossier de données et réutilisés ensuite. La clé n'est jamais écrite au journal ni dans un lien.
- **Port mixte** : avec ce certificat, le serveur accepte TLS **et** clair sur le même port. Il lit le premier octet de la connexion sans le consommer : `0x16` (début d'une poignée de main TLS) part en wss, le reste en ws. Cela remplace, pour ce mode seulement, la règle « un port sert soit wss, soit ws » de l'ADR 0009. Avec des PEM fournis par l'hôte, rien ne change : wss seul.
- **Empreinte dans le lien** : le lien d'invitation porte l'empreinte SHA-256 du certificat dans un nouveau paramètre `fp`. Le lien reste en `v=1` : `parse_invite` ignore déjà les paramètres inconnus, un ancien launcher le lit donc sans erreur.
- **Épinglage** : le launcher et `client-core` acceptent ce certificat seulement si son empreinte est celle du lien. Sans `fp`, la vérification habituelle s'applique.
- **Mods non modifiés** : ils échouent en wss sur le certificat auto-signé et retombent sur ws, comme aujourd'hui. L'interface le dit (« chiffré pour les joueurs Raftup, en clair pour les autres clients »).
- **Interface** : « Créer une partie » et « Héberger » prennent ce mode par défaut ; « Mon certificat (PEM) » et « En clair (ws) » restent dans « Options avancées ».

## Conséquences

- Plus aucun fichier à fournir : un débutant héberge en un clic, et les joueurs qui passent par Raftup sont chiffrés.
- Le chiffrement n'est pas garanti pour tous : un mod non modifié passe en clair, mot de passe de la salle compris. À afficher clairement ; ce n'est pas pire qu'aujourd'hui.
- Le serveur gagne une détection TLS / clair à tester (connexion lente, premier octet seul, client qui ne parle jamais).
- `client-core` gagne un vérificateur de certificat par empreinte (rustls), à relire avec soin.
- **Essai réel d'abord** (`docs/regles-agents.md`, découpage) : avant de coder l'interface, vérifier qu'un mod non modifié (Slimipelago) retombe bien sur ws face au port mixte. Si ce n'est pas le cas, revoir la décision.
- Renouvellement : certificat de longue durée (10 ans), régénéré seulement à la demande ; les anciens liens deviennent alors invalides pour l'épinglage.
