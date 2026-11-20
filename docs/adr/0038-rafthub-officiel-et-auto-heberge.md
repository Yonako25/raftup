# ADR 0038 — Rafthub officiel et Rafthub auto-hébergé : moyens de connexion

- **Statut** : accepté le 20 novembre 2026 (demande de l'utilisatrice ; mise en œuvre choisie en autonomie, à relire)
- Complète l'ADR 0033 (connexion Discord), l'ADR 0035 (client du hub) et l'ADR 0036 (console d'admin)

## Contexte

L'utilisatrice veut deux versions de Rafthub :
- le **Rafthub auto-hébergé**, que chacun installe chez soi : connexion par pseudo ou e-mail et mot de passe, **sans Discord** ;
- le **Rafthub officiel**, sur son propre serveur : la même chose, **plus** la connexion Discord.

Jusqu'ici, tout hub pouvait activer Discord en réglant l'application Discord dans la console, et la connexion ne se faisait que par l'adresse e-mail.

## Décision

### Discord réservé à la version officielle
- Option de compilation `officiel` de la crate `hub` (`cargo build -p hub --release --features officiel`). Le binaire officiel est compilé avec ; les binaires des releases (auto-hébergé) sans.
- Sans l'option, `HubConfig::discord_allowed` vaut faux :
  - la connexion Discord répond `discord_disabled`, même si des réglages Discord existent en base ou dans l'environnement ;
  - la console n'affiche pas le groupe « Connexion Discord » des réglages, ni la ligne Discord du tableau de bord ; un formulaire Discord forgé est refusé ;
  - aucun avertissement « Discord non réglé » au démarrage.
- Le code Discord reste compilé dans les deux versions : un seul code, testé partout (les tests activent Discord). On ne le retire pas du binaire : l'option dit ce que la version **propose**, ce n'est pas une protection. Quelqu'un qui recompile Rafthub avec `--features officiel` obtient un hub avec Discord, ce qui ne gêne personne (il lui faut sa propre application Discord).

### Connexion par pseudo ou e-mail
- `POST /v1/sessions` et la page de connexion de la console acceptent l'adresse e-mail **ou** le pseudo dans le même champ. Une saisie avec `@` est une adresse (un pseudo ne peut pas contenir `@`), sinon c'est un pseudo, comparé sous sa forme normalisée (NFKC, minuscules).
- Le champ JSON reste `email` pour les launchers déjà diffusés ; `login` est accepté aussi.
- Même réponse `invalid_credentials` pour un pseudo inconnu, une adresse inconnue ou un mauvais mot de passe, et même temps de réponse (hash bidon). Le limiteur d'essais ratés compte par compte visé (clé `e:<adresse>` ou `u:<pseudo>`).
- L'inscription demande toujours une adresse e-mail (vérification, mot de passe oublié).
- Le launcher écrit « E-mail ou pseudo » à la connexion.

## Conséquences

- Deux binaires à construire au moment des releases : le binaire public sans option, celui du serveur officiel avec `--features officiel`.
- Le launcher n'a pas encore de bouton Discord ; quand il en aura un, il devra demander au hub si Discord est proposé (pas encore de route pour ça).
- Quelqu'un qui connaît un pseudo peut tenter des mots de passe sur ce compte : c'était déjà possible avec l'adresse, et le limiteur s'applique pareil.
