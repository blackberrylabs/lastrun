# PLAN — Lastrun
<!-- Project plan (French). -->

## Etape 0. Cadrage — faite le 2026-10-17
- [x] Sources A (journaux d'actions, 21 hotes, code amont lu), B (douleurs de la communaute),
      C (outils existants), C2 (`agentrail` npm lu dans son paquet), D (sonde de la machine)
      ; `00_cadrage/synthese_sources.md`
- [x] Nom verifie (crates.io, npm libres ; PyPI dormant sans rapport) et valide : Lastrun
- [x] BRIEF rev. 1 valide (R1–R6)
- [x] Depot local, identite noreply, commit 1 (`40901c7`, 2026-10-17)

## Etape 1. Coeur et CLI
- [x] Fiches hotes `hosts/*.toml` : 12 lisibles (lecteur, retention, trous), 9 sans journal
      d'actions (raison, ce qui existe quand meme), chacune sourcee
- [x] Lecteurs d'actions : `claude-jsonl`, `codex-jsonl` (+ `.zst`), `gemini-jsonl`,
      `qwen-jsonl`, `opencode-sqlite` (OpenCode, Kilo ; schemas v1 `part` et v2
      `session_message`), `goose-sqlite`, `cline-tasks` (Cline, Roo), `cline-sessions`,
      `copilot-jsonl`, `zed-sqlite`, `kiro-sqlite`
- [x] Schema commun, masquage des commandes, regles `rules/*.toml` sourcees (destructif,
      paquets, pipe-to-shell)
- [x] Les onze constats ; chiffres du resume ; rapport `bbl-report/1`
- [x] CLI : `lastrun`, `sessions`, `actions`, `deleted`, `packages`, `denied`, `hosts`,
      `report`, `--since`, `--json`, `--home`
- [x] Tests : promesses (rien d'ecrit, rien de reseau, secret plante jamais ressorti, aucun
      texte d'echange dans la sortie), faux HOME avec un constat de chaque sorte ; clippy —
      24 tests, clippy propre (2026-10-17)
- [x] Passage reel 1 (2026-10-17) : 27 sessions, 1 294 commandes, 4 hotes presents ; cinq
      defauts corriges (faux secrets, paquets lus dans un `$(…)`, identifiants OpenCode
      confondus, sous-agents non signales, `| head` en erreur) — 27 tests
- [x] Passage reel 2 (2026-10-17) : corrections verifiees ; trois restes corriges (`cacheKey`
      pris pour un secret, un paquet Python compte sous deux ecritures, UUID coupes trop long)
- [x] Passage reel 3 (2026-10-17, commit `f4449bc`) : zero faux constat — 7 constats, tous
      justes ; moteur valide

## Etape 2. Maquette validee, puis app (Sessions, Actions, Supprimes, Paquets, Refuses)
- [x] Maquette A1 (constats, onglet Sessions, detail d'un constat) et A2 (onglet Supprimes,
      chronologie d'une session) et icone — validees le 2026-10-17, avec R7 (existe a nouveau)
- [x] App : le gabarit de la famille (titre-phrase, 4 cartes, constats, detail, pied),
      quatre onglets a gauche, a droite le constat ou la session choisis ; diff a la demande
      (`core/src/diff.rs`) ; EN/FR, clair/sombre ; icones 16–512 — 30 tests, clippy propre
- [x] Passage reel de l'app sur le poste d'Alexandre — validee le 2026-10-17 (commit `e911506`)
## Etape 3. Paquets, depot, release 0.1.0 — sur accord
Suivant `Blackberrylabs/docs/new-module.md` (§0–2 deja tenus : gabarit, `bbl-hostrules`,
rapport `bbl-report/1` verifie par les tests, gravites dans le CHANGELOG) :
- [x] §3 `--select <id du constat>` tenu par l'app reelle
- [ ] §4 dans la tour : `modules/lastrun.toml`, l'entree de `spec.rs`, le rapport de Lastrun
      dans `tests/fixtures/reports/` (chemins `/home/me`), les comptes de modules des tests ;
      essai a blanc fait le 2026-10-17 : la fiche se charge, le rapport passe
      `the_real_reports_conform_and_open_their_module`, seuls les comptes 5 → 6 sont a changer
- [ ] §5 passage reel du module et de `bbl refresh` avec Lastrun installe ; README du module
      et de la tour, profil de l'organisation ; `github_meta.sh`
- [ ] Release du module d'abord (« Get it » doit tomber sur un paquet), la tour ensuite
      (`latest = "0.1.0"`)
