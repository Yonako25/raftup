# 0008 — Tests différentiels contre le serveur officiel

- Statut : accepté
- Date : 2026-11-15

## Contexte

NF02 demande des « vecteurs de référence produits par le serveur Python » et un « test différentiel ».
Les vecteurs existent : 6 scénarios capturés une fois sur le serveur officiel 0.6.8 et rejoués par `cargo test`.
Il manque le test différentiel : faire jouer les mêmes échanges aux deux serveurs, y compris des échanges auxquels personne n'a pensé, et comparer.
L'ADR 0005 demande aussi de mesurer la bande passante perdue faute de compression.

## Décision

- Un pilote en Python, `crates/server/tests/differentiel/differentiel.py`, lancé à la main :
  - il démarre le serveur officiel (`MultiServer.py --use_embedded_options`) et `raftup-server` sur la seed MIT, sur deux ports ;
  - il joue chaque scénario sur les deux serveurs et compare les trames, normalisées comme dans `tests/connexion.rs` ;
  - il écrit chaque écart en JSON, au format des vecteurs, prêt à devenir un test de `cargo test`.
- Les scénarios fixes vivent dans un module commun, `scenarios.py`, utilisé aussi par les scripts de capture.
- Un générateur tire des scénarios au hasard (plusieurs clients, paquets valides et mal formés), à partir d'une graine : un écart se reproduit avec la même graine.
- Le générateur évite les écarts déjà connus et acceptés (liste commentée dans le code), et ce qui n'est pas encore fait (commandes `!`, indices : J2).
- Synchronisation : après chaque étape, chaque client envoie un `GetDataPackage` vide qui sert de « barrière ». Sa réponse prouve que le serveur a fini l'étape ; elle est retirée avant la comparaison. C'est plus sûr et plus rapide qu'une attente fixe.
- Les deux serveurs gardent leur état d'un scénario à l'autre : ils ont reçu exactement la même histoire. Après un écart, les deux sont relancés.
- Le pilote mesure la taille des trames reçues, dont le `DataPackage` complet, sans et avec compression zlib (une estimation de `permessage-deflate`).
- Pas dans la CI : il faut Python et la source d'Archipelago. Les chemins viennent de `RAFTUP_PYTHON` et `RAFTUP_ARCHIPELAGO`.

## Conséquences

- On trouve des écarts sans les avoir imaginés ; chacun devient un vecteur fixe, donc un test permanent.
- Le Python autonome et la source d'Archipelago doivent être récupérés à la main (archives et empreintes de l'étape 0).
- Les limites propres à Raftup (ADR 0007 : débit, taille du chat) ne sont pas testées ici : le serveur officiel n'en a pas.
- La barrière ajoute deux ou trois paquets par client et par étape ; une étape dure donc au moins 0,1 s pour rester sous les 50 paquets/s.
- Première mesure (ADR 0005) : le `DataPackage` complet de la seed MIT fait 26 298 octets, 5 732 une fois compressé. Sans compression, Raftup envoie environ 4,6 fois plus, une fois par client et par jeu : acceptable pour l'instant.
