# BRIEF — Lastrun (rev. 1, validé le 2026-10-17)

Module n° 7 et dernier de la famille Blackberrylabs. Verbe : **replay**. Une phrase :

> **Lastrun montre ce que les agents IA ont fait sur ce poste — commandes lancées, fichiers
> touchés, paquets installés, appels d'outils — session par session, relu après coup dans les
> transcriptions que les hôtes écrivent déjà.**

Tagline (anglais, carte de la tour) : *What your AI agents actually did on this machine —
commands, files, packages — session by session, read back from the hosts' own transcripts.*

Nom libre sur crates.io et npm (vérifié le 2026-10-17) ; un paquet PyPI `lastrun` 0.1 dormant
(« record the date a program was last run »), sans rapport. *Agentrail* écarté : paquet npm
homonyme (mars 2026) sur le même créneau, qui envoie les transcriptions brutes de Claude Code
à un serveur distant — le négatif de ce module (`00_cadrage/C2_agentrail_npm.md`).

## 1. Le problème

Après une session d'agent, personne n'a la chronologie. Les incidents sont documentés et datés
(`00_cadrage/B_douleurs_communaute.md`) : un `rm -rf ~` par Claude Code (#10077, 2025-10 :
« the command itself wasn't logged, only the output »), un `git reset --hard` au démarrage
d'une session, un sous-agent qui efface, Cursor qui vide un disque, un utilisateur qui **ne
sait pas quel hôte a agi**, un script écrit par l'agent dans une session puis exécuté dans une
autre. La demande est formulée telle quelle par la communauté : « What did the AI agent do,
and did a human authorize it? » (forum Cursor, 2026-03).

Or la matière existe. Douze hôtes écrivent sur le disque, appel d'outil par appel d'outil, ce
que l'agent a demandé et ce qu'il a obtenu (`00_cadrage/A_journaux_actions.md`) : Claude Code
garde la commande, `stdout`/`stderr`, le patch structuré de chaque édition, le fichier
original, la décision de permission et le mode de la session ; OpenCode garde l'entrée, le
code de retour, les fichiers touchés avec leur patch ; Codex, Gemini CLI, Qwen Code, Goose,
Cline, Roo, Copilot CLI, Zed, Kilo de même, chacun à sa façon. Mais c'est éparpillé (JSONL,
SQLite, JSON réécrit, Markdown), dans des formats déclarés internes et changeants, **purgé à
30 jours** chez Claude Code et Gemini CLI sans corbeille, et illisible à l'œil. Sur ta machine
(`D_sonde_machine.md`) : 240 appels d'outils Claude Code en 4 fichiers, 2 992 chez OpenCode.

Ce qui existe répond à côté (`C_outils_existants.md`) : des visionneuses de *chat* (dont une
multi-hôtes à 2 100 étoiles qui **supprime** des sessions), des enregistreurs par hooks qui ne
voient rien d'avant leur installation et écrivent leur propre base, des exports OpenTelemetry
qui partent vers un serveur, et les outils d'entreprise (Cursor, Copilot) qui excluent
explicitement commandes et fichiers. Les approbations sont le point faible de tous — et des
formats eux-mêmes : dans le JSONL de Claude Code, seul le **refus** laisse une trace sûre.

## 2. Ce que fait Lastrun

Pour chaque **hôte présent** qui enregistre ses actions, Lastrun relit les transcriptions et en
tire des **actions** — jamais le texte des échanges — projetées sur un schéma commun :

`{host, session, call_id, kind (shell | file | mcp | network | other), command?, path?,
diff?, timestamp?, cwd?, approval {mode?, decision?, source?}, result {ok, exit_code?,
error?}, model?, parent?}`

Trois lectures dans l'app : **Sessions** (par hôte et par projet, dans le temps, avec le
nombre d'actions, le mode de permission, l'âge et la purge à venir), **Actions** (la
chronologie d'une session : commandes, fichiers, paquets, MCP, approbations, résultats ;
filtres par sorte, recherche sur les commandes et les chemins seulement), et trois **vues de
premier rang** que personne ne propose : **Fichiers supprimés**, **Paquets installés**,
**Commandes refusées**. Le diff d'une édition s'ouvre à la demande ; les prompts et les
réponses de l'assistant ne sont jamais affichés : Lastrun n'est pas un lecteur de
transcriptions.

Fenêtre par défaut : 30 jours (comme Billfold), `--since <jours>` pour remonter, tout ce que
les hôtes ont gardé est lisible.

### Constats (`Kind`) et gravité

| Kind | Ce que c'est | Gravité |
|---|---|---|
| `destructive` | Une commande destructrice **exécutée avec succès**, non refusée : `rm -rf` hors d'un dossier temporaire, `git reset --hard`, `git clean -fdx`, `git push --force`, `DROP TABLE/DATABASE`, `mkfs`, `dd of=`, `chmod -R 777`, `> fichier` sur un fichier suivi par git… Motifs sourcés (règles publiques réutilisées, `C_outils_existants.md`), liste dans `rules/destructive.toml` | **1 act now** |
| `secret-in-transcript` | Un secret en clair dans une transcription (clé renvoyée par une commande, `.env` lu et recopié) : le fichier de transcription est devenu un magasin de secrets — valeur masquée, fichier et ligne désignés | **1 act now** |
| `outside-project` | Écriture ou suppression **hors du projet de la session** (dotfiles du home, `~/.ssh`, `/etc`, autre projet), hors les dossiers propres à l'hôte | **2 review** |
| `unattended-destructive` | Session en mode sans demande (`bypassPermissions`, `--dangerously-skip-permissions`, Codex `approval_policy: never`, Qwen `yolo`, Cline auto-approve) **et** au moins une commande destructrice ou hors projet | **2 review** |
| `pipe-to-shell` | `curl … \| sh`, `wget … \| bash`, exécution de code téléchargé | **2 review** |
| `deferred-script` | Un fichier écrit par un agent dans une session, **exécuté** dans une session ultérieure (le cas `fs.rmSync` de Cursor) | **2 review** |
| `package-install` | Installation de paquets par l'agent (`npm/pnpm/yarn add`, `pip install`, `cargo add`, `apt`, `brew`, `go get`) — listée, avec le paquet et la session | **3 good to know** |
| `pruning-soon` | Des sessions à moins de 7 jours de la purge de l'hôte (Claude Code `cleanupPeriodDays`, Gemini CLI `sessionRetention.maxAge`, 30 j par défaut) : l'historique va disparaître | **3 good to know** |
| `denied` | Un appel refusé par l'utilisateur ou par une règle — le seul état d'approbation sûr chez la plupart des hôtes | **4 information** |
| `partial-record` | Un hôte lisible dont la transcription **n'a pas** : code de retour (Claude Code, Gemini, Goose, Cline, Zed), décision d'approbation par appel (Codex, Gemini, OpenCode, Kilo, Goose, Zed, Roo), diff (Goose, Copilot), horodatage par message (Zed) — dit une fois par hôte | **4 information** |
| `no-action-log` | Un hôte présent qui **n'enregistre pas ses actions** de façon lisible : Aider (Markdown sans sortie ni diff), Amp (format non documenté), Cursor (transcript sans résultats ni horodatages, base non documentée), Claude Desktop chat, Devin, JetBrains AI, Kiro (schéma non publié), Trae, Warp, Windsurf — avec ce qui existe quand même | **4 information** |

Approbation, trois états et pas plus : **refusé** (sûr), **autorisé** (sans savoir si par la
personne ou par une règle, sauf chez Claude Code et Copilot CLI qui donnent la source),
**non enregistré par cet hôte**. Jamais « approuvé manuellement » déduit.

Hors périmètre v0.1 : juger l'**intention** (pourquoi l'agent a fait ça) ; demander à l'agent
ce qu'il a fait ; les hôtes sans journal ; le « en direct » (hooks, offset) ; comparer au
disque réel au-delà d'une seule question — le fichier supprimé existe-t-il à nouveau (R7) ;
les actions des connecteurs de Claude Desktop chat.

### Chiffres du résumé (cartes de la tour)

`sessions_30d`, `commands_30d`, `files_deleted_30d`, `findings`.

### Actions (affichées, jamais exécutées)

`open-file` sur le fichier touché, ou sur la transcription (fichier et ligne) ; `command`
pour rouvrir la session dans son hôte (`claude --resume <id>`, `codex resume <id>`,
`opencode --session <id>`) ; `hint` (« set `cleanupPeriodDays` higher », « this file no
longer exists ») ; `open-url` vers la page de doc de la règle. Et pour `secret-in-transcript` :
« rotate the key ; the transcript keeps it until the host prunes it ».

## 3. Hôtes (v0.1)

Douze lisibles, chacun avec sa fiche sourcée (code amont lu et cité, ou documentation) :
Claude Code (JSONL, + `subagents/`), Codex CLI (rollouts JSONL et `.zst`, `state_5.sqlite`),
Gemini CLI, Qwen Code (`~/.qwen/projects/<cwd>/chats/`), OpenCode et Kilo Code (SQLite,
JSON dans `data`), Goose (`sessions.db`), Cline (tâches JSON et SDK `sessions.db`), Roo Code,
GitHub Copilot CLI (`events.jsonl`), Zed (`threads.db`, zstd), Kiro CLI (héritage Amazon Q,
marqué `observed`). Neuf montrés en `no-action-log`. Les chemins et formats viennent de
`bbl-hostrules` (0.5.0 : JSONL, SQLite, Markdown déjà décrits) ; Billfold a les lecteurs de
jetons des mêmes fichiers, Lastrun ajoute les lecteurs d'actions à côté, dans la crate.

Divergences à absorber (A §3.2) : code de retour numérique seulement chez OpenCode/Kilo et
Codex, en texte (« Exit code: N ») ailleurs, absent chez Claude Code ; suppressions
identifiables chez Codex, OpenCode/Kilo, Cline seulement, à inférer de la commande ailleurs ;
paquets et réseau jamais étiquetés, toujours inférés de `command` ; horodatages ISO, ms, s ou
absents ; fichiers JSON réécrits en place et états `running` à tolérer ; `.zst` à décompresser.

## 4. Promesses (celles de la famille)

- **Lecture seule** : rien d'écrit hors `~/.config/lastrun/` (préférences). Les bases SQLite
  s'ouvrent en `mode=ro` ; jamais de copie ni d'index propre.
- **Aucune sortie réseau.** Liens affichés, jamais suivis.
- **Jamais le texte des échanges** : ni prompts, ni réponses, ni raisonnement — les actions
  seulement ; le diff à la demande. Pas d'export de transcription, ni entière ni partielle.
- **Secrets masqués** à la lecture (`bbl-hostrules::secrets`) dans les commandes, les sorties,
  les diffs ; jamais écrits.
- **Un état, pas un jugement** : « a exécuté `rm -rf build/` dans le projet, succès » — jamais
  « a voulu », jamais « dangereux » sans la règle sourcée qui le dit.
- **Pas de surveillance des personnes** : Lastrun lit la machine de celui qui le lance, ne
  distingue pas qui a tapé le prompt, n'agrège rien vers l'extérieur.
- **Chaque règle cite sa source** (doc, code source ou `observed`), sa date, sa citation.

## 5. Architecture

Gabarit `template` : `crates/core` (fiches hôtes, lecteurs d'actions par format, projection
sur le schéma commun, règles `rules/*.toml` — destructif, paquets, réseau, hors projet —,
constats, rapport `bbl-report/1`), `crates/cli` (`lastrun` : `sessions`, `actions <session>`,
`deleted`, `packages`, `denied`, `hosts`, `report`, `--since`, `--json`), `crates/app`
(`lastrun-app`, iced, `bbl-ui`, `--select`). `bbl-hostrules` 0.6.0 si des lecteurs y sont
ajoutés (JSONL en flux, SQLite `ro`, zstd). Tests : `tests/promises.rs` (rien d'écrit, rien de
réseau, un secret planté dans une fausse transcription jamais ressorti, aucun texte de prompt
dans le rapport ni dans la sortie CLI) ; `tests/scan.rs` (faux HOME avec une transcription par
hôte lisible, un constat de chaque sorte, les hôtes sans journal présents).

## 6. Étapes

0. Sources — fait (`00_cadrage/A…D`, `C2`).
1. BRIEF validé ; décisions R1–R6 ; dépôt `blackberrylabs/lastrun`.
2. Fiches hôtes (12 + 9) ; lecteurs d'actions ; règles ; `scan` + `model` ; tests.
3. CLI + `report` conforme.
4. Maquette de l'app validée, puis l'app.
5. Passage réel sur la machine d'Alexandre (Claude Code, OpenCode, Cursor en `no-action-log`).
6. Release 0.1.0 par le rituel ; `modules/lastrun.toml` + `latest` dans la tour ; hubs le
   jour même ; `github_meta.sh`.

## 7. Décisions prises (2026-10-17)

Validées par Alexandre en bloc (« Oui, je valide. Tu peux y aller. »), avec la recommandation
à chaque fois.

- **R1 — Le nom : Lastrun** (binaires `lastrun`, `lastrun-app` ; dépôt `blackberrylabs/lastrun`).
- **R2 — Jamais le texte des échanges**, même dans l'app, même à la demande : les actions, et
  le diff d'une édition, relu à la demande dans la transcription.
- **R3 — Les onze constats dès la 0.1**, `deferred-script` compris.
- **R4 — Approbation en trois états** (refusé / autorisé / non enregistré), sans jamais
  déduire « manuel ».
- **R5 — Fenêtre 30 jours par défaut**, `--since` pour le reste.
- **R6 — Les vues « Fichiers supprimés », « Paquets installés », « Commandes refusées »** en
  premier rang de l'app, à côté de Sessions et Actions.
- **R7 — Un fichier supprimé dit s'il existe à nouveau** (« exists again » / « the file does
  not exist today ») : une simple lecture des métadonnées, rien d'autre comparé au disque.
  Validé avec la maquette le 2026-10-17.
- **Maquette validée le 2026-10-17** (A1, A2, icône) : le gabarit de la famille ; à gauche
  quatre onglets (Sessions, Supprimés, Paquets, Refusés) ; au milieu les constats ; à droite
  le constat ou la session choisis, avec la chronologie de ses actions et le diff à la
  demande.

Détail de mise en œuvre (non une décision) : les lecteurs d'actions vivent dans
`lastrun-core`, à côté des fiches, comme les lecteurs de jetons vivent dans Billfold ; ils
remonteront dans `bbl-hostrules` le jour où un troisième module en aura besoin.
