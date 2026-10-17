# JOURNAL — Lastrun
<!-- Project journal (French), newest first. One entry = a date, a title, what was decided, by whom, what was set aside, the proof. -->

---

## 2026-10-17 — maquette validee, puis l'app

- **Rappel d'Alexandre** : suivre le dossier des nouveaux modules de la tour
  (`Blackberrylabs/docs/new-module.md`). Lastrun en suit deja les §0–2 (parti de `Template`,
  rapport conforme et teste, gravites dans le CHANGELOG) ; essai a blanc dans une copie de la
  tour : la fiche se charge, le rapport passe le test de la tour, seuls les comptes de
  modules (5 → 6) sont a changer. Le reste (§3–5) est ecrit dans le PLAN, etape 3.
- **Maquette A1/A2 et icone validees** (« oui je valide ») avec un point au-dela du BRIEF :
  dire si un fichier supprime existe a nouveau — devenu **R7**, une lecture de metadonnees.
- **L'app** : le gabarit de Billfold et Linstruct, quatre onglets a gauche (Sessions,
  Supprimes, Paquets, Refuses), la chronologie d'une session a droite, filtres par sorte,
  recherche sur les commandes et les chemins seulement. Le **diff a la demande** (R2) :
  `diff.rs` relit la transcription en lecture seule, retrouve l'appel par son identifiant
  et ne garde que sa modification, secrets masques — Claude Code (`structuredPatch`, sinon
  l'entree), Codex (l'enveloppe apply_patch, la section du fichier), OpenCode et Kilo
  (`metadata.diff`), Gemini et Qwen (`fileDiff`). Les autres hotes le disent.
- **Verifie a l'ecran** sur le faux HOME (Xvfb) : constat choisi, session ouverte, action
  choisie, diff affiche, francais et sombre. 30 tests, clippy propre.

---

## 2026-10-17 — premier passage reel

- **Ce que le poste a montre** : 27 sessions (26 OpenCode, 1 Claude Code), 1 294 commandes,
  25 fichiers supprimes en 30 jours ; Claude Desktop et Cursor presents, sans journal lisible.
  Huit constats, dont un « act now » de 25 secrets, en grande partie faux.
- **Cinq defauts, corriges** :
  1. *Faux secrets* — 25 « secrets » dans la base OpenCode : des sorties Python
     (`OPENAI_API_KEY=None\n…`) lues dans le JSON brut, ou le `\n` echappe collait deux
     valeurs ; des identifiants (`KEY_BY_PROVIDER_…`). Desormais le JSON est lu dechiffre,
     un nom de secret doit *finir* par KEY/TOKEN/SECRET/PASSWORD…, et une valeur sous un tel
     nom n'est plus montree du tout (`NOM=••••••••`) : six premiers caracteres d'un mot de
     passe, c'etait trop. Le constat distingue « forme connue » et « sous un nom de secret ».
  2. *Base partagee* — le conseil « supprimez la transcription » visait `opencode.db`, qui
     tient toutes les sessions : pour une base, le constat nomme les sessions et dit que la
     supprimer les perdrait toutes.
  3. *Paquets* — `pip install "x==$(grep … | cut …)"` donnait `&&`, `|`, `cut` comme
     paquets : les arguments etaient redecoupes aux espaces. Lus maintenant tels que le
     decoupage les a laisses ; versions, extras et chemins retires ; chaque nom une fois.
  4. *Identifiants* — les identifiants OpenCode commencent par l'heure : douze sessions
     ouvertes a la minute s'affichaient toutes `ses_f227`. Le prefixe montre est le plus
     court qui les distingue (8 au moins), et les sous-agents sont signales.
  5. *`| head`* — « Error: Broken pipe » : le lecteur parti n'est plus une erreur.
- **Preuve** : 27 tests (le faux HOME rejoue les cinq cas), clippy propre.
- **Passage 2** (commit `6286572`) : plus qu'un « secret », `cacheKey=…` — une cle qui indexe
  un cache, pas un identifiant. Les noms qui finissent comme un secret sans en etre un sont
  ecartes (cle d'index, cle publique, jeton de pagination, `key` nu). `typing_extensions`
  et `typing-extensions` comptaient pour deux : un paquet Python est un seul nom, quelle que
  soit l'ecriture. Les UUID de Claude Code etaient coupes a 16 caracteres pour suivre ceux
  d'OpenCode : chaque identifiant a maintenant sa propre longueur. Le compte des
  transcriptions Claude Code (1 en 30 jours) confirme la session unique.

---

## 2026-10-17 — naissance : la boite noire des agents

- **D'ou vient l'idee** : piste « boite noire des agents » d'`IDEES.md` (16/10, « fichiers
  modifies, commandes, paquets, par session »). Alexandre, le 17/10 au soir : « pour moi, ca
  va etre le dernier module, la boite noire des agents ». Elle ferme la boucle : Permitt dit ce
  que l'agent *peut* faire, Lastrun ce qu'il *a fait*.
- **Nom** : Alexandre voulait Agentrail ; ecarte apres verification — paquet npm `agentrail`
  (mars 2026) sur le meme creneau, qui pose des hooks dans Claude Code et envoie les lignes
  brutes des transcriptions a un serveur (lu dans son paquet, `00_cadrage/C2`). Lastrun choisi
  parmi Lastrun / Retrail / Didit (registres verifies).
- **Sources** : A (21 hotes, 12 depots amont clones et lus), B (~45 items sources : `rm -rf ~`,
  `git reset --hard`, secrets dans les transcriptions, demande d'audit), C (~60 liens ; rien ne
  relit apres coup, sans hook, multi-hotes, les actions, hors ligne), D (sonde de la machine :
  240 appels Claude Code, 2 992 OpenCode, noms et comptes seulement).
- **BRIEF rev. 1 valide en bloc** : jamais le texte des echanges (R2), onze constats (R3),
  approbation en trois etats (R4), 30 jours (R5), trois vues de premier rang (R6).
- **Ecarte** : le « en direct » (hooks, offset), juger l'intention, les hotes sans journal
  (montres comme constat), l'export.
