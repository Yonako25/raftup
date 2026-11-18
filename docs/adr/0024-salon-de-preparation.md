# 0024 — Salon de préparation, côté hôte (F08)

- Statut : proposé
- Date : 2026-11-18
- Concerne F08 de `docs/SPEC.md`. Périmètre MVP : sans hub (le hub est après le MVP).

## Décision (le plus petit choix simple)

1. **Le salon est un onglet « Salon » de « Partie en cours »**, visible seulement quand la partie est hébergée. Par joueur : jeu, lien d'invitation (ADR 0020), bouton « Tester », statut « prêt ? ». Un bouton « Tout tester » lance un test par joueur.
2. **« Prêt » = le serveur accepte une vraie connexion sur ce slot**, ouverte par `client-core` via `diagnose::check_connection_with_timeout` (F05), avec l'adresse publique de la partie, puis refermée. Résultat typé `launcher_core::Readiness` (Prêt, Refusé, TLS, Délai dépassé, Mauvaise version, Mauvais slot, Injoignable) avec une phrase en français.
3. **Limites assumées** : le test ne prouve pas que le joueur a installé son mod ni qu'il est en ligne. Il laisse une connexion brève dans le journal du serveur. Le statut n'est pas rafraîchi tout seul (l'hôte reclique), et il est perdu à l'arrêt de la partie.
4. **Mot de passe** : absent de `HostStatus` (NF09) ; l'hôte le retape dans le champ du salon, gardé en mémoire de l'écran seulement.
5. **Plus tard (hub)** : le « prêt ? » déclaré par chaque joueur depuis son propre launcher, la présence et le choix du jeu par les joueurs relèvent du hub et ne sont pas traités ici.

## À valider

- Un onglet plutôt qu'un écran séparé.
- Test depuis l'adresse publique (voit les problèmes que verraient les joueurs, mais échoue en TLS si le certificat est auto-signé, ce qui est voulu).
