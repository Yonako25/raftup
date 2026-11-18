# ADR 0025 — Adresse du webhook Discord mémorisée en clair

- **Statut** : accepté le 18 novembre 2026 (décision de la mainteneuse)

## Décision

L'adresse du webhook Discord est écrite en clair dans la configuration du launcher (`UserConfig`), pas dans le coffre Windows.

## Conséquences

- Qui lit le dossier de configuration peut poster dans le salon Discord : le champ reste masqué à l'écran, l'adresse n'est jamais loggée ni affichée par `Debug`, et elle n'est jamais incluse dans un lien d'invitation ni un export.
- Révocable à tout moment côté Discord (supprimer le webhook).
