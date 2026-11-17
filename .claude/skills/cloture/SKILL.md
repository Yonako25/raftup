---
name: cloture
description: Clôture une session de travail sur Raftup : écrit l'entrée du journal, met à jour l'état du projet si l'avancement a changé, et propose le commit. À utiliser à la fin d'une tâche ou d'une étape, ou quand l'utilisatrice tape /cloture.
---

# Clôture de session (Raftup)

Réponds en français, court. Ne commite **que** si l'utilisatrice l'a demandé ou le demande après ta proposition.

## Étapes

1. **Rassemble les faits sans relire tout l'historique** : `git log --oneline -10`, `git status --short` et ce que tu sais de ta session. Ne relis pas les fichiers de code.
2. **Écris l'entrée dans `docs/journal.md`** (ajout en bas de « Entrées », format fixe, **5 lignes au plus**) :

   ```
   ### AAAA-MM-JJ · <nom de la session>
   - **Fait** : <ce qui est fait, avec les hash de commit>
   - **Pas vérifié** : <ce qui n'a pas été testé ou reste une hypothèse>
   - **Ne pas toucher** : <fichiers ou dossiers d'une autre session ou en attente>
   - **Suite** : <la prochaine étape, une seule>
   ```

   Pas de code, pas de liste de fichiers, pas de détail déjà présent dans un ADR.
3. **Retire de « En cours »** la ligne de ta session, s'il y en a une.
4. **Mets à jour `docs/etat-du-projet.md`** seulement si l'avancement a changé : ligne de la crate concernée, case de « À faire » cochée ou ajoutée, date de mise à jour. Ne recopie pas le journal.
5. **Propose** : « Je commite le journal (et l'état) ? » — un seul commit, uniquement ces fichiers ; jamais les fichiers d'une autre session. Message en français, avec la ligne de co-auteur demandée par l'environnement.
6. **Conseille de repartir à neuf** si la session est longue : « Étape commitée : une nouvelle session repartira avec `etat-du-projet.md` et le journal, pour moins de contexte. »

## Règles

- Une entrée par session, pas par message. Si la session a déjà une entrée, complète-la au lieu d'en ajouter une.
- Reste factuel : ne marque « fait » que ce qui a été vérifié (commande lancée) ; le reste va dans « Pas vérifié ».
- Les décisions d'architecture vont dans `docs/adr/`, pas dans le journal.
