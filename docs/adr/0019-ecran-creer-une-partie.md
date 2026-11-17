# 0019 — Choix de l'écran « Créer une partie »

- Statut : accepté
- Date : 2026-11-17
- Complète l'ADR 0017 (décisions 9 et 12) et ferme les « Décisions à prendre » de `docs/contrat-ui-generation.md`.

## Contexte

Le contrat de l'écran « Créer une partie » laissait trois questions ouvertes avant de coder : que faire des anciennes parties dans `generations/`, comment choisir les fichiers YAML, et faut-il couper le réseau du générateur par défaut.

## Décision

1. **Conservation de `generations/` : purge manuelle.** Rien n'est supprimé automatiquement après une partie réussie (la sauvegarde `.raftup` vit à côté de la seed). L'interface proposera un bouton « Supprimer » par partie, avec confirmation. Le nettoyage après un échec ou une annulation (ADR 0017, décision 9) ne change pas.
2. **YAML : sélecteur de fichiers seul.** Un bouton « Ajouter des YAML » ouvre la boîte de fichiers ; la liste se modifie dans l'écran. Pas de dossier surveillé.
3. **Réseau du générateur bloqué par défaut.** `PythonBackend::new` met `block_network: true`, qui passe `RAFTUP_BLOCK_NETWORK=1` au worker. Le champ reste public pour le couper si un apworld légitime en a besoin.

## Conséquences

- Aucune sauvegarde perdue par surprise ; en échange, `generations/` grossit tant que l'utilisatrice ne fait pas le ménage.
- Moins de code : pas de surveillance de dossier. Les grosses parties demandent de sélectionner plusieurs fichiers d'un coup.
- Le blocage réseau gêne un apworld malveillant naïf, mais ce n'est **pas** un bac à sable (ctypes ou un sous-process le contournent) : l'avertissement sur les apworlds reste obligatoire.
- Un apworld qui a vraiment besoin du réseau échouera à la génération ; il faudra alors une option dans l'interface (pas prévue au MVP).
