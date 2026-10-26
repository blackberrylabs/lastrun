# AGENTS.md (Lastrun)
<!-- Règles de travail pour tout agent IA (OpenCode, Claude Code, autre). Source unique : CLAUDE.md n'est qu'un pointeur. Français : pilotage. Anglais : code, docs, commits, README. -->

Lastrun : ce que les agents IA ont fait sur ce poste (commandes, fichiers, paquets, appels d'outils), relu après coup dans les transcriptions que les hôtes écrivent déjà. Module de la tour Blackberrylabs, le plus récent.

## Au démarrage de chaque session

1. Avec ton outil de lecture, lis `PLAN.md` et les cinq premières entrées de `JOURNAL.md`. Lis `BRIEF.md` si ton travail touche au périmètre.
2. Lance `git status -sb` et `git log --oneline -5` : sache où en est le dépôt avant d'y toucher.
3. Lis la dernière section du `CHANGELOG.md` si tu changes un comportement visible.

## Promesses du module

Lecture seule sans exception. Aucune sortie réseau. Secrets masqués, jamais écrits, jamais présents dans une sortie. Rien d'écrit hors `~/.config/lastrun/`. Un état, pas une note : jamais « trop cher » ni « dangereux » sans le fait qui le fonde. Actions affichées, jamais exécutées. Chaque règle cite sa source (documentation, code source lu, ou `observed`), sa date et sa citation : rien de mémoire. Les tests (voir Repères) verrouillent ces promesses : rien d'écrit, rien de réseau, secret planté jamais ressorti, rapport conforme. Ne les affaiblis jamais pour faire passer un changement. Les bases SQLite s'ouvrent en `mode=ro`, sans copie ni index. **Jamais le texte des échanges** : ni prompts, ni réponses, ni sorties de commandes dans l'inventaire, la CLI, l'app ou le rapport, seulement les actions ; le diff d'une édition se relit à la demande. L'approbation a trois états (refusé, autorisé, non enregistré) : « manuel » n'est jamais déduit.

## Obligations d'un module de la tour

1. `lastrun report` répond un document `bbl-report/1` valide : la crate `bbl-report` fait foi.
2. Chaque sorte de constat a sa place sur les quatre gravités (agir maintenant, à regarder, bon à savoir, information) dans le code (`severity_of` dans `crates/core/src/report.rs`, ou `model.rs`), et le tableau est recopié dans le CHANGELOG. Toute nouvelle sorte de constat y passe le jour même.
3. L'app accepte `--select <id>` et s'ouvre sur l'élément demandé.
4. Une fiche déclarative `modules/lastrun.toml` existe dans la tour (`Blackberrylabs/`). Quand le contrat bouge, le module est publié avant la tour.

La marche complète est dans `Blackberrylabs/docs/new-module.md`. Un module ne met dans son rapport que ce qui concerne cette machine : ce qu'il calcule pour des hôtes absents reste dans son app.

## Repères

- `crates/core/src/` : `model.rs`, `sheets.rs`, `rules.rs`, `command.rs`, `scan.rs`, `findings.rs`, `report.rs`, `diff.rs` (le diff d'une édition, relu à la demande), `read/<format>.rs` (un lecteur par format de journal).
- `hosts/*.toml` et `rules/*.toml` (destructif, paquets, pipe-to-shell). Schéma dans `docs/hosts.md`. Constats et gravités dans `model.rs` et le CHANGELOG.
- Tests : `crates/core/tests/{promises,scan}.rs`. Pour écrire le faux HOME sur disque : `LASTRUN_FIXTURE_OUT=<dossier> cargo test -p lastrun-core --test scan write_fixture`.
- CLI `lastrun | sessions | actions | deleted | packages | denied | hosts | report`, avec `--since`, `--json`, `--home`. App `lastrun-app --select <id> --session <id> --tab sessions|deleted|packages|refused --since <jours> --home <dir>`.

## Commandes (sur le PC d'Alexandre, depuis la racine du dépôt)

- Tester le cœur sans compiler iced : `cargo test -p lastrun-core` ; un seul test : `cargo test -p lastrun-core <nom>` ; un fichier de tests (`promises`, `scan`) : `cargo test -p lastrun-core --test <fichier>`.
- Avant un commit de code : `cargo fmt --check` puis `cargo clippy --all-targets`, sans avertissement.
- CLI : `cargo run -p lastrun-cli -- <sous-commande>`. Rapport pour la tour : `cargo run -p lastrun-cli -- report | python3 -m json.tool`.
- App : `cargo run -p lastrun --bin lastrun-app --release -- --select <id>`.

## Avant une release

1. `CHANGELOG.md` daté et version de `Cargo.toml` à jour.
2. Passage réel sur le poste d'Alexandre, sur ses vrais projets, sortie collée dans la conversation : il révèle les défauts que les fixtures ne voient pas.
3. `./packaging/build.sh` (avec accord) produit `dist/` : `.deb`, `.rpm`, AppImage et `SHA256SUMS`.
4. Le paquet publié est retéléchargé, sa somme et sa version vérifiées, `lastrun report` contrôlé.
5. Dans la tour : `latest` mis à jour dans `modules/lastrun.toml`, `packaging/github_meta.sh` relancé si la description ou les topics changent.
6. Tag, push et release : avec son accord, un par un.

<!-- BLOC COMMUN v1, 2026-10-26. Identique dans les douze dépôts de la série. Le modifier partout ou nulle part (Logiciels/PASSATION.md, section « Maintenir »). -->

## Pour qui tu travailles

Alexandre Do pilote et décide, tu exécutes. Réponds-lui en français courant et va au fait : pas de préambule, pas de rappel de sa demande, pas de récapitulatif de ce qu'il vient de voir passer. Si la réponse tient en deux phrases, deux phrases suffisent.

- Quand la demande est claire, fais-la et montre le résultat. Les questions sont réservées aux vrais arbitrages : une à la fois, chacune avec ta recommandation et sa raison.
- Pour une décision de conception, donne d'abord ton point de vue argumenté, ou un exemple concret (maquette, extrait de code). Une question ouverte sans avis ne l'aide pas.
- Dis-lui quand il a tort, ou quand une demande contredit une décision déjà actée, avec la raison. Ne réécris pas ce qu'il n'a pas demandé de réécrire : ni refactorisation au passage, ni renommage spontané.
- Toute commande que tu lui donnes est complète et copiable telle quelle, chemin absolu compris : `cd /home/nushoko/Bureau/Logiciels/<Dépôt> && …`. Dis où la taper, ce qu'elle produit, comment voir qu'elle a marché et comment revenir en arrière. Un terme technique se définit à sa première occurrence.
- N'ouvre pas de chantier de ton chef : propose-le avec ton point de vue et attends son accord.
- Ce qui dépasse quelques lignes va dans un fichier, au bon endroit du dépôt.

## Ce qui exige son accord explicite, à chaque fois

L'accord est un « go » ou un « oui » écrit dans la conversation, que tu consignes au JOURNAL avec la date. Un silence n'est pas un oui, et l'accord donné pour une chose ne vaut pas pour la suivante. Tu prépares, tu montres la commande exacte et ce qu'elle fera, puis tu attends.

1. `git push`, sous toute forme (branche, tags, `--force`).
2. Créer ou pousser un tag, publier une release, modifier un dépôt GitHub ou ses métadonnées (`gh`, `packaging/github_meta.sh`), `cargo publish`.
3. Supprimer un fichier ou un dossier, sorties générées comprises (`target/`, `dist/`, `00_cadrage/`, fichiers temporaires). Cela couvre `rm`, `git clean`, `git reset --hard`, `git branch -D` et `find -delete`. Dis quoi, combien et pourquoi, puis attends.
4. Créer, modifier ou arrêter ce qui tourne tout seul : minuterie systemd, cron, service.
5. Réécrire l'historique d'un commit déjà poussé (`rebase`, `commit --amend`).
6. Installer un paquet système, lancer `sudo`, lancer `packaging/build.sh` (il efface ses dossiers temporaires et réécrit `dist/SHA256SUMS`).
7. Modifier une release en service ou un BRIEF validé.
8. Écrire hors du dossier du dépôt.

Restent libres : lire, chercher, calculer par script, écrire dans le dépôt, `git add` et `git commit` en local, mettre à jour PLAN, JOURNAL et ce fichier. Le fichier `opencode.json` à la racine fait demander confirmation sur ces commandes : ne l'assouplis pas sans son accord.

## Méthode

- Les dépôts de code portent quatre fichiers de pilotage à la racine, en français : `BRIEF.md` (le cadrage), `PLAN.md` (l'avancement, source de vérité), `JOURNAL.md` (une entrée datée par décision, avec sa preuve, la plus récente en haut) et `AGENTS.md`. Le BRIEF validé précède le code : sans BRIEF validé pour ce que tu t'apprêtes à faire, écris le cadrage et soumets-le.
- Une case de PLAN ne se coche qu'avec une preuve (commit, tag, sortie de commande, test), la date en regard. Une case devenue fausse se corrige avec sa date.
- En fin de session substantielle, mets à jour PLAN et JOURNAL, et ce fichier si la forme du dépôt a changé.
- `00_cadrage/` (sources lues, sorties de passages réels, brouillons de messages) est hors git et le reste : ne le commite pas, ne le publie pas, ne le supprime pas. `.gitignore` s'édite sur place, jamais recopié d'ailleurs.
- Une information a un seul domicile. Ce qu'un script peut produire, un script le produit : ce qu'on recopie finit par mentir.
- Ne relance que ce qui a changé : le test visé (`cargo test -p <crate> <nom>`), pas toute la suite à chaque commit en phase de conception. Ne lance jamais les tests en arrière-plan : lance, attends, lis.
- Les tests tournent sur le PC d'Alexandre. Le quota GitHub Actions est de 2 000 minutes par mois : n'ajoute aucun workflow déclenché à chaque push sans son accord.

## Doctrine « jamais faux »

- Une valeur est sourcée et datée, ou elle n'est pas écrite. Ce qu'on ne sait pas s'écrit `?` ou `[à compléter]`, bien visible.
- Ta mémoire de modèle n'est jamais une source. Version d'un outil, format d'un fichier, tarif, règle d'un hôte : lis la documentation, le code source ou la machine, cite-les et date la lecture.
- Un calcul qui finit dans un livrable passe par un script que l'on peut relire.
- Une règle d'hôte cite sa source (documentation, code source lu, ou `observed`), sa date et sa citation. Sans source lue, la règle n'entre pas.
- Ce que tu ne peux pas trancher, dis-le. Un arbitrage silencieux coûte plus cher qu'une question franche.
- Ne dis pas « vérifié » sans l'avoir vérifié. Si tu n'as pas pu lancer quelque chose (pas de `cargo`, pas de réseau), dis-le plutôt que de supposer le résultat.
- Les clés d'API et les fichiers `.env` ne se lisent pas, ne s'affichent pas et ne se commitent pas.
- Une clé factice dans un test n'a jamais la forme exacte d'une vraie clé (GitHub a déjà bloqué un push pour cela) : la chaîne est assemblée à l'exécution.

## Langue, commits, textes publiés

- Anglais : code, commentaires, messages de commit, README, CHANGELOG, CONTRIBUTING, `docs/`, interface (anglais par défaut, français en option). Français : BRIEF, PLAN, JOURNAL, ce fichier, `CLAUDE.md` et tes réponses à Alexandre. Pour cette série, cette règle prime sur « tout en français » de ses notes personnelles.
- Un commit, un changement. Message en anglais, une ligne, sans point final : `zone: ce qui change` (`docs: …`, `hosts: …`, `report: …`, `assets: …`), ou le nom de la chose changée. Une sortie de version porte son numéro (`0.2.0`). Regarde `git status` et `git diff --stat` avant de commiter.
- Texte publié (README, CHANGELOG, descriptions de dépôt, posts) : prose par défaut, concrète (chiffre, date, nom). Ni tiret cadratin ni demi-cadratin, ni point d'exclamation, ni points de suspension, ni émoji. Pas de liste de trois adjectifs ou de trois exemples, pas de « ce n'est pas X, c'est Y », pas de superlatif de brochure (crucial, essentiel, puissant, robuste). Exception : la phrase de famille, validée telle quelle, se reproduit à l'identique : « Read-only tools that show what AI coding agents do to your machine — and one screen that gathers them. »
- Un texte publié ne dit rien de la vie, du métier, du niveau ou de l'entourage d'Alexandre. Le seul nom cité est Alexandre Do. S'il faut parler de lui, rédige une proposition et soumets-la ; ne l'écris pas dans un fichier du dépôt.
- Chiffres publiés : seulement ceux mesurés sur sa machine, datés. Captures d'écran : sur le jeu de données de démonstration, jamais sur ses vrais projets.

## Git

- Branche `main`, remote `origin` en SSH sous `blackberrylabs/<dépôt>`.
- Avant le premier commit dans un dépôt, vérifie l'identité locale : `git config user.name` doit afficher `Alexandre Do` et `git config user.email` doit afficher `277202463+Nimeria2026@users.noreply.github.com`. Sinon, corrige en config locale (sans `--global`). La config globale de son terminal est une autre identité, et GitHub refuse alors le push. Avant de lui proposer un push : `git log --format='%an <%ae>' origin/main..HEAD`.
- Si git répond qu'un fichier `.lock` existe, ne le supprime pas toi-même. Vérifie qu'aucun autre git ne tourne (`pgrep -a git`), puis donne-lui `cd /home/nushoko/Bureau/Logiciels/<Dépôt> && find .git -name '*.lock' -delete`.
- Le `.gitignore` exclut `target/`, `dist/` et `00_cadrage/` : n'utilise jamais `git add -f`.
- Les dépendances `bbl-*` sont fixées par tag git dans `Cargo.toml`. Changer une fondation : la corriger à sa source, la tagger (avec accord), puis faire passer les modules à ce tag un par un, en vérifiant que leurs sorties restent identiques.

## La série en un coup d'œil

Dépôts frères dans `/home/nushoko/Bureau/Logiciels/`, publics sous `github.com/blackberrylabs`. Rust, iced 0.13.1, licence MIT. Un outil, un verbe : un nouveau constat va dans l'outil existant, un nouveau verbe fait un nouvel outil. La tour rassemble sans absorber : elle lance les modules, lit leurs rapports `bbl-report/1`, assemble, trie, ouvre. Chaque module reste complet et installable seul.

| Dossier | Rôle | Fondations |
|---|---|---|
| `Bbl_hostrules` | crate : fiches d'hôtes sourcées, lecteurs de fichiers, masquage des secrets | aucune |
| `Bbl_report` | crate : le contrat `bbl-report/1` entre la tour et les modules | aucune |
| `Bbl_ui` | crate : palette, briques iced, réglages, textes EN/FR | aucune |
| `Cleanator` | module : ce que l'IA laisse sur le disque, avec un script à relire | report, ui |
| `Mcproster` | module : les serveurs MCP déclarés et leur coût en contexte | hostrules, report, ui |
| `Permitt` | module : ce que chaque agent peut faire sans demander | hostrules, report, ui |
| `Linstruct` | module : les fichiers d'instructions tels que l'hôte les lira | hostrules, report, ui |
| `Billfold` | module : comptes, clés et jetons dépensés | hostrules, report, ui |
| `Lastrun` | module : ce que les agents ont fait, session par session | hostrules, report, ui |
| `Blackberrylabs` | la tour, binaire `bbl` | report, ui |
| `Template` | gabarit d'un nouveau module (`rename.sh`) | report, ui |
| `Blackberrylabs_github` | profil de l'organisation (dépôt `.github`) | aucune |

L'état daté de la série (versions, pushes en attente, travail ouvert) est dans `/home/nushoko/Bureau/Logiciels/PASSATION.md`, hors des dépôts : OpenCode te demandera l'autorisation de le lire.
<!-- FIN BLOC COMMUN -->
