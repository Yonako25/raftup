# ADR 0021 — Mods sans licence dans le catalogue

- **Statut** : accepté (décision de la mainteneuse, le 18 novembre 2026)

## Contexte

Slimipelago (Slime Rancher) n'a aucune licence. L'ADR 0015 exigeait une autorisation écrite (`license = "permission"` + `permission_url`). La mainteneuse a décidé de s'en passer pour ce mod.

## Décision

- Nouvelle valeur `credits.license = "unlicensed"` : le mod n'a pas de licence, `permission_url` est facultatif.
- Raftup ne redistribue rien : les fichiers sont téléchargés chez l'auteur, sur le PC du joueur, après vérification du sha256 (règle inchangée : jamais de jeu, de ROM ni d'asset).
- Si l'auteur demande le retrait, le profil est supprimé du dépôt.

## Conséquences

- `profiles/slime-rancher.toml` entre dans le catalogue ; `docs/profils-en-attente/` ne garde que le brouillon de demande.
