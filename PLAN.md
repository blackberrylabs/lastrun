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
- [ ] Passage reel 2 : les corrections verifiees sur le poste

## Etape 2. Maquette validee, puis app (Sessions, Actions, Supprimes, Paquets, Refuses)
## Etape 3. Paquets, depot, release 0.1.0 — sur accord ; `modules/lastrun.toml` + `latest` dans la tour ; hubs le jour meme ; `github_meta.sh`
