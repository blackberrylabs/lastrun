# CLAUDE.md — Lastrun
<!-- Working rules for Claude in this repository (French). Code and docs are in English. -->

Lastrun : ce que les agents IA ont fait sur ce poste — commandes, fichiers, paquets, appels
d'outils — relu apres coup dans les transcriptions des hotes. Module n° 7 et dernier de la
famille Blackberrylabs ; fiche dans `Projets/00_moi/PROJETS.md`.

## Regles de travail

- BRIEF valide avant le code ; questions a Alexandre une par une, decisions une par une
  avec une recommandation ; PLAN et JOURNAL tenus a chaque etape ; `00_cadrage/` hors git.
- Lecture seule sans exception (bases SQLite en `mode=ro`, jamais de copie ni d'index),
  aucune sortie reseau, secrets masques et jamais ecrits, rien d'ecrit hors
  `~/.config/lastrun/`. **Jamais le texte des echanges** : ni prompts, ni reponses, ni
  sorties de commandes dans l'inventaire, la CLI, l'app ou le rapport — les actions
  seulement ; le diff d'une edition relu a la demande. Approbation en trois etats
  (refuse / autorise / non enregistre), jamais « manuel » deduit. Un etat, pas une note. Actions affichees,
  jamais executees. Chaque regle cite sa source (doc, code source ou `observed`), sa date,
  sa citation : rien de memoire.
- Commits locaux libres ; `git push`, suppression de fichiers, tags, timers : accord
  explicite. Commandes donnees completes, chemin absolu
  (`cd /home/nushoko/Bureau/Logiciels/Lastrun && …`). Avant tout git lance par Alexandre :
  `find .git -name '*.lock' -delete` (le pont laisse des verrous).
- Anglais : code, commentaires, commits, README, interface par defaut. Francais : BRIEF,
  PLAN, JOURNAL, CLAUDE.md, option de langue.
- Seul nom cite : Alexandre Do.
- Un passage reel sur la machine d'Alexandre avant chaque release ; chaque release
  verifiee depuis l'exterieur (paquet telecharge, somme, version, `report` conforme).

## Identite git

Avant le **premier commit** : `git config user.name "Alexandre Do"` et
`git config user.email "277202463+Nimeria2026@users.noreply.github.com"` en config locale ;
verifier par `git log --format='%an <%ae>'` avant tout push.

## Fondations communes — ne pas recopier, completer a la source

- `bbl-hostrules` (0.5.0) : fiches, resolution des chemins, masquage. Les lecteurs d'actions
  sont dans `lastrun-core` (comme ceux de jetons dans Billfold).
- `bbl-ui` : palette, briques iced, reglages, i18n. Meme visage que les autres outils.
- `bbl-report` : le contrat de la tour. `lastrun report` produit un `bbl-report/1` ;
  chaque `Kind` a sa place sur les quatre gravites (`model.rs`), ecrite dans le CHANGELOG.

## Reperes

- `crates/core/src/{model,sheets,rules,command,scan,findings,report}.rs`, `read/<format>.rs`
  (un lecteur par format) ; `hosts/*.toml` (21), `rules/*.toml` ; schema `docs/hosts.md`.
- Tests : `tests/promises.rs`, `tests/scan.rs` (faux HOME, un constat de chaque sorte ;
  `LASTRUN_FIXTURE_OUT=dir cargo test --test scan write_fixture` l'ecrit sur disque).
- `crates/cli` (`lastrun`, `sessions`, `actions`, `deleted`, `packages`, `denied`, `hosts`,
  `report`), `crates/app` (`lastrun-app --select`).
- Les onze constats et leur gravite : `model.rs` et CHANGELOG.
- `docs/new-module.md` de la tour : la marche a suivre complete.
