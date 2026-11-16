# 0014 — Sauvegarde au format `.raftup` en JSON lignes

- Statut : accepté
- Date : 2026-11-16

## Contexte

La spec demande (NF07) d'écrire chaque check et chaque objet sur disque avant d'acquitter la réponse au client. Un arrêt brutal ne doit faire perdre aucune donnée. La spec prévoyait le format `.raftup` en MessagePack compressé avec zstd.

Le stockage de la partie contient du JSON libre (les `Set` et `Get` des clients), avec de grands entiers. Le MessagePack compressé a trois défauts ici :

- il n'est pas lisible à l'œil nu, ce qui gêne le débogage ;
- ajouter un événement oblige à réécrire un bloc, ou à gérer des blocs à la suite, ce qui complique la reprise ;
- la compression zstd n'a pas d'intérêt à cette taille : une partie à 4 joueurs tient en moins de 1 Mio.

## Décision

Le format `.raftup` est du JSON lignes : une ligne = un objet JSON complet, séparé par un saut de ligne.

### Contenu du fichier

1. **Ligne 1, en-tête** :
   ```json
   {"format_version":1,"kind":"raftup-save","raftup_version":"0.1.0","seed_name":"...","seed_sha256":"...","created":1794853200.0}
   ```
2. **Ligne 2, instantané complet** de la partie à ce moment.
3. **Lignes suivantes, deltas** : une ligne par changement de l'un de ces éléments : `checks`, `items`, `status`, `hints`, `hints_used`, `alias`, `group_collected`, `store`.

### Écriture

- Les événements sont écrits par lot, puis le fichier est synchronisé sur le disque (`sync_data`, fsync) **avant** l'envoi des réponses aux clients. C'est l'acquittement de NF07.
- La tâche « salle » (ADR 0006) fait cette écriture sur un thread système dédié, pour que le fsync ne bloque pas le runtime `tokio`.
- Disque plein ou erreur d'écriture : arrêt net du serveur, sans acquitter les événements en attente. Un client ne reçoit jamais une réponse pour un événement qui n'est pas sur disque.

### Compactage

Le fichier est réécrit en entier (un instantané, puis ses deltas) dans un fichier `.tmp`, puis renommé sur l'original. Le compactage a lieu :

- au démarrage ;
- au-delà d'environ 10 000 lignes ;
- à l'arrêt propre.

Le renommage remplace le fichier d'un coup : une coupure au milieu laisse soit l'ancien fichier, soit le nouveau, jamais un mélange.

### Reprise

Au démarrage avec un fichier existant :

- vérifier `format_version` (un numéro inconnu fait échouer le démarrage) ;
- vérifier l'empreinte SHA-256 de la seed (`seed_sha256`) : si elle ne correspond pas, refus de démarrer ;
- ignorer une dernière ligne tronquée (coupure pendant une écriture) ;
- refuser une ligne corrompue au milieu du fichier : pas de reprise partielle silencieuse.

DÉCISION : la seed n'est **pas** incluse dans le `.raftup`. Seule son empreinte l'est. Le `.raftup` vit à côté de la seed, dans le même dossier. Ça évite de copier une seed (qui peut être lourde) dans chaque sauvegarde.

### Mot de passe

Le mot de passe de la salle n'est jamais écrit dans le fichier (NF09).

### Options

- `--savefile <chemin>` : chemin du fichier `.raftup` (par défaut à côté de la seed).
- `--disable_save` : pas de sauvegarde, comme l'option du serveur officiel.

## Conséquences

- Le fichier se lit à la main et se lit ligne par ligne : un débogage ou une restauration partielle sont simples.
- Ajouter un événement ne réécrit rien : une ligne est ajoutée, puis synchronisée.
- Un `.raftup` de 4 joueurs reste sous 1 Mio, sans compression.
- Le format est versionné (`format_version`), comme la spec le demande.
- La seed doit rester à côté de sa sauvegarde : une seed déplacée sans son `.raftup` ne se reprend pas, et une seed modifiée est refusée.
- Le format JSON est plus gros que le MessagePack. Ça ne gêne pas à la taille visée. Si une partie dépasse largement cette taille, on pourra revoir ce choix dans un nouvel ADR.
- Le fsync à chaque lot coûte du temps d'écriture : les lots doivent rester groupés pour ne pas ralentir les clients.
