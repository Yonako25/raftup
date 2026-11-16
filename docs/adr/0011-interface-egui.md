# 0011 — Interface du launcher en egui, à la place de Tauri

- Statut : accepté (décision de la porteuse du projet, 16 novembre 2026)
- Date : 2026-11-16
- Remplace : la partie « interface » de l'ADR 0010 (Tauri v2, HTML/CSS/TypeScript)

## Contexte

L'ADR 0010 prévoyait une interface Tauri : HTML et TypeScript dans la webview du système.
Tauri est léger face à Electron, mais reste un moteur de navigateur à côté du launcher, et ajoute Node.js et npm aux outils du projet.
La suite ArtCraft (PhotoCraft, LightCraft…) montre qu'une interface soignée est possible en Rust pur avec egui.

## Décision

- `launcher-ui` est une application **egui / eframe** (rendu wgpu), écrite entièrement en Rust : plus de webview, plus de Node.js ni de npm.
- `launcher-core` ne change pas : l'interface l'appelle directement, dans le même process.
- Le style de la maquette (acier et bleu électrique, angles de 2 px, panneaux sur fond sombre, Chakra Petch et Inter) est reproduit par un thème egui ; les polices TTF (licence OFL) sont intégrées au binaire.
- Le reste de l'ADR 0010 tient : serveur dans le launcher, suivi par photo et fil d'événements, journal en mémoire.

## Conséquences

- Un seul langage, un seul outil de compilation (`cargo`) ; la CI n'a plus besoin de Node.js.
- Binaire plus léger et moins de mémoire qu'avec une webview.
- Le style se code à la main en Rust : dégradés et effets de transparence sont plus limités qu'en CSS.
- egui redessine l'écran à chaque image : il faut demander un rafraîchissement régulier (une fois par seconde) pour le tableau de bord, sans faire tourner le processeur pour rien.
