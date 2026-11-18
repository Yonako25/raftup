# 0020 — Rejoindre une partie par lien d'invitation (F04)

- Statut : accepté (validé par l'utilisatrice le 2026-11-18)
- Date : 2026-11-18
- Concerne F04 de `docs/SPEC.md` (« Rejoindre par lien »). `docs/etat-du-projet.md` l'appelait F06 par erreur : F06, ce sont les presets.

## Contexte

Aujourd'hui, l'hôte copie un texte (adresse, « demandez le mot de passe », liste des joueurs) et chaque joueur recopie l'adresse et son slot à la main dans la Bibliothèque. F04 demande un lien qui ouvre Raftup avec ces informations. Il faut choisir le format du lien, ce qu'il contient, comment Windows l'envoie au launcher, et ce que l'écran fait à l'ouverture, sans ouvrir de faille (lien malveillant, injection dans la ligne de commande d'un jeu).

## Décision

1. **Format : `raftup://join?v=1&addr=…&slot=…&game=…`.** Le même lien peut aussi se coller dans un champ « Rejoindre » du launcher. Pas de page `https://` au MVP : il faudrait un domaine et le hub. Discord ne rend pas cliquable un schéma inconnu (vérifié le 18 novembre 2026) : le collage est le vrai chemin depuis Discord. Chrome et Edge réécrivent le lien en `raftup://join/?…`, forme acceptée.
2. **Contenu : jamais le mot de passe.** `v` (version du format), `addr` obligatoire, `slot` et `game` (nom du jeu côté Archipelago) facultatifs. Un mot de passe dans un lien finirait dans l'historique Discord, dans la ligne de commande visible des autres programmes et dans les journaux (NF09) : le joueur le tape à l'écran.
3. **Un lien par joueur**, copié depuis sa ligne dans « Partie en cours », en plus du texte d'invitation général (qui gagne un lien sans slot).
4. **Schéma d'URL enregistré au premier lancement** sous `HKCU\Software\Classes\raftup`, sans droits administrateur (NF10), avec `winreg` (déjà une dépendance). La clé est vérifiée à chaque démarrage et réécrite si le chemin de l'exécutable a changé.
5. **Instance unique par fichier verrou et « boîte aux lettres ».** La première instance verrouille un fichier (`std::fs::File::try_lock`, Rust 1.89 : relever `rust-version`, aujourd'hui 1.88). Une deuxième instance ouverte par un lien écrit le lien dans `inbox/<horodatage>-<pid>.link` (fichier `.tmp` puis renommage) et quitte. La première relit ce dossier à chaque seconde. Pas de port, pas de nouvelle dépendance, pas d'`unsafe`.
6. **Validation stricte.**
   - Lien : 2 048 octets au plus, schéma `raftup` exact, version connue (sinon « mettez Raftup à jour »), paramètre en double refusé, paramètre inconnu ignoré.
   - Adresse : `ws` ou `wss` seulement (`ws` avec un avertissement, NF06) ; hôte ASCII (`[A-Za-z0-9.-]` ou IPv6 entre crochets), pas d'IDN, ni `user@`, ni chemin, ni requête ; port de 1 à 65535, 38281 par défaut.
   - Slot : longueur maximale d'Archipelago (16 caractères, à vérifier dans 0.6.8), sans caractère de contrôle ni de sens d'écriture (U+202E…), sans `-` au début (un jeu Unity le lirait comme une option).
7. **À l'ouverture : rien d'automatique.** Un écran « Rejoindre » montre l'hôte en clair et le slot (à saisir s'il manque). « Continuer » mène :
   - jeu du catalogue installé : à sa fiche dans la Bibliothèque, formulaire de connexion rempli ;
   - jeu du catalogue non installé : à la même fiche, avec « Installer » ;
   - jeu hors catalogue : copie de l'adresse et du slot, avec les consignes.

## Contrat

Fonctions pures dans `crates/launcher-core/src/invite.rs` :

```rust
pub const FORMAT_VERSION: u32 = 1;
pub struct ServerAddress { pub secure: bool, pub host: String, pub port: u16 }
impl ServerAddress {
    pub fn parse(text: &str) -> Result<ServerAddress, InviteError>;
    pub fn to_url(&self) -> String;
}
pub struct Invite { pub address: ServerAddress, pub slot: Option<String>, pub game: Option<String> }
pub fn parse_invite(text: &str) -> Result<Invite, InviteError>;
pub fn make_invite(invite: &Invite) -> Result<String, InviteError>;
pub enum InviteError { TooLong, NotAnInvite, UnsupportedVersion(u32), MissingAddress,
    BadAddress(&'static str), BadSlot(&'static str), BadGame, BadEncoding, Duplicate(String) }
```

Encodage avec `percent-encoding` (déjà dans `Cargo.lock`) ; propriété testée : `parse_invite(&make_invite(x)?) == x`.

Ailleurs :
- `launcher-core/src/instance.rs` : `acquire`, `forward`, `take_inbox` (10 fichiers au plus, 4 Kio chacun, triés par nom) ;
- `launcher-core/src/url_scheme.rs` : `command_line(exe)` (pure), `ensure_registered(exe)` (Windows seulement).

## Conséquences

- Aucun secret ne circule dans un lien ; en échange, le joueur tape le mot de passe de la salle.
- Un lien malveillant ne peut que pré-remplir un écran : rien ne se connecte ni ne se lance sans un clic après affichage de l'hôte.
- Un autre programme peut s'approprier `raftup://` : Raftup réécrit la clé à son démarrage, sans plus.
- Le verrou est libéré par le système si Raftup plante ; la boîte aux lettres est bornée et chaque lien repasse par `parse_invite`.
- `rust-version` passe à 1.89.
