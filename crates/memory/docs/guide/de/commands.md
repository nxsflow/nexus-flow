# Befehle

Die vollständige `nxm`-Oberfläche: zwölf Befehle, in der Reihenfolge, in der eine Sitzung nach ihnen
greift. `nxm <befehl> --help` liefert die generierte Referenz (Flags, Typen, Vorgaben); diese
Anleitung ergänzt die Erzählung und die `--json`-Formen.

Drei Dinge gelten global:

- **`--json`** auf jedem Befehl — der Agenten-Vertrag. Datensätze tragen eine stabile, deklarierte
  Feldreihenfolge, die Bytes sind also vergleich- und diffbar.
- **`--db <pfad>`** (auch `NXM_DB`) — direkt auf eine Workspace-Datenbank zeigen, statt `.nxs/` vom
  Arbeitsverzeichnis aus nach oben zu suchen.
- **`NXM_ACTOR`** — der Autor, der bei jedem Schreibvorgang eingetragen wird (sonst `USER`, sonst
  `nxm`). `NXM_NOW` fixiert den Zeitstempel, den ein Schreibvorgang setzt — genau das macht die
  Beispiele in diesen Anleitungen byte-reproduzierbar.

Fehler sind ein strukturierter Umschlag, keine Prosa: `{"error":{"kind":…,"msg":…}}` mit einem
Exit-Code ungleich null. Die Arten, nach denen Sie tatsächlich verzweigen, sind `validation` (der
Aufruf war falsch), `not_found` (keine solche Erinnerung) und `no_workspace` (hier gibt es nichts zu
öffnen).

## Einrichten

### `nxm init`

memory im aktuellen Verzeichnis aktivieren: einen `.nxs/`-Arbeitsbereich sicherstellen, das Modul
`memory` registrieren und dann die gemeinsame Agentendatei und die
SessionStart-Hooks an den `nxs`-Assembler übergeben, der sie für *jedes* aktive Modul neu
zusammensetzt — einen zweiten Baustein zu aktivieren überschreibt also nie den Block des ersten.

```bash
nxm init                 # für Menschen, mit Banner
nxm init --json          # die maschinenlesbare Quittung
```

`--quiet` ist stiller Erfolg — es gibt überhaupt nichts aus, und genau das macht es steuerbar für
die Klammer und für einen Agenten:

```console
$ nxm init --quiet

```

`--from-beads` willigt in die Migration beads → nxs ein (Tickets *und* Erinnerungen) und läuft über
die Klammer, der sie gehört.

### `nxm agent-manifest`

Was memory als seinen Beitrag zur gemeinsamen Agentendatei deklariert — den prime-Befehl und
den Hook — als Daten statt als Prosa:

```console
$ nxm agent-manifest
nexus-memory agent manifest
  prime command:  nxm prime
  hook:           SessionStart → nxm prime

Run with --json for the machine contract the `nxs` umbrella assembles from.

```

`--json` ist dieser Vertrag (`{"prime_command":…,"hook":{"event":…,"command":…}}`); die Klammer liest
ihn von jedem aktiven Baustein und setzt daraus eine `AGENTS.md` und einen Hook zusammen.

## Schreiben

### `nxm remember <TEXT>`

Einen Fakt anlegen oder an Ort und Stelle aktualisieren. Der gesamte Schreibpfad ist offline und
deterministisch — nichts hier fragt ein Modell.

```console
$ nxm remember "auth uses JWT, not sessions" --key auth-jwt --introduction "auth is JWT, not sessions"
remembered auth-jwt

```

Flags:

- **`--introduction <zeile>`** — **Pflicht.** Eine Zeile, höchstens 200 Zeichen. Sie ist das
  Einzige an dieser Erinnerung, was ein Sitzungsstart je zu sehen bekommt; sie sagt also, was die
  Erinnerung SAGT, nicht wovon sie handelt — und bei `--category rules` SPRICHT sie die Regel AUS
  („Never run X while Y is running", nicht „Regeln zu X"), denn niemand schlägt ein Verbot nach,
  bevor er es bricht. Über der Grenze wird beim Schreiben abgewiesen, mit der gemessenen Länge in
  der Meldung. Die ganze Begründung steht in
  [Die Einleitung](nxm-core-concepts).
- **`--key <key>`** — ein stabiler, selbst gewählter Schlüssel, unter dem an Ort und Stelle
  geschrieben wird. Ohne ihn ist der Schlüssel ein Inhaltshash: gleicher Text fällt zusammen,
  umformulierter Text legt einen neuen Fakt an ([Kernkonzepte](nxm-core-concepts)).
- **`--category <slug>`**, **`--scope item|project|global`**, **`--refs <id,id>`** — die Erinnerung
  im selben Schreibvorgang einordnen. Ein weggelassenes Flag lässt sein Register bei einer neuen
  Erinnerung auf der Vorgabe und bei einer bestehenden *unangetastet*.

Auch eine Aktualisierung ist von `--introduction` nicht ausgenommen: der Rumpf hat sich geändert,
also ist genau die Frage, ob die Zeile ihn noch beschreibt.

`--json` gibt den ganzen Datensatz zurück — dieselbe Form, die auch `recall` und `classify`
liefern:

```console
$ nxm remember "auth uses JWT; the session table was dropped in v3" --key auth-jwt --introduction "auth is JWT; the session table went away in v3" --json
{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT; the session table went away in v3"}

```

Ein leerer Text wird abgelehnt statt gespeichert:

```console
$ nxm remember "" --introduction "an empty body" --json
? 1
{"error":{"kind":"validation","msg":"a memory body must not be empty"}}

```

### `nxm classify <SCHLÜSSEL>`

Eine bestehende Erinnerung einordnen — Kategorie, Reichweite, Referenzen und/oder ihre Einleitung —
**ohne ihren Text anzufassen**, damit `updated` weiter den Fakt datiert und nicht die Ablage.
`--introduction` ist hier auch der Weg, mit dem eine vor diesem Register geschriebene Erinnerung
ihre Zeile bekommt.

```console
$ nxm classify auth-jwt --category architecture
classified auth-jwt
  category: architecture
  scope:    project
  refs:     none
  intro:    auth is JWT; the session table went away in v3

```

```console
$ nxm memories --category architecture
auth-jwt  auth uses JWT; the session table was dropped in v3

```

Mindestens eines der vier ist Pflicht — still nichts zu tun wäre schlimmer, als es zu sagen — und
eine Kategorie muss ein Kleinbuchstaben-Slug sein:

```console
$ nxm classify auth-jwt --json
? 1
{"error":{"kind":"validation","msg":"classify needs at least one of --category, --scope, --refs or --introduction"}}

```

```console
$ nxm classify auth-jwt --category "Not A Slug" --json
? 1
{"error":{"kind":"validation","msg":"invalid category 'Not A Slug': use a lower-case slug like 'introduction' (letters, digits, '-' and '_')"}}

```

`--refs` ersetzt die ganze Menge; `--refs ""` ist das ausdrückliche „keine Referenzen":

```console
$ nxm classify since-cutoff --refs "" --json
{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":[],"ordinal":null,"introduction":"the --since cutoff is exclusive"}

```

### `nxm reorder <SCHLÜSSEL>...`

Eine ausdrückliche Lesereihenfolge speichern: der erste Schlüssel wird Position 1, der zweite 2, und
so weiter. Jeder Schlüssel muss existieren — eine abgelehnte Folge lässt die gespeicherte Reihenfolge
genau so, wie sie war. Bewusst und selten; alltägliche Schreibvorgänge fassen die Reihenfolge nie an,
und eine Erinnerung, die niemand platziert hat, sortiert hinter jeder platzierten, in der Reihenfolge
ihrer Entstehung.

```console
$ nxm reorder auth-jwt since-cutoff
reordered 2 memories
  1  auth-jwt
  2  since-cutoff

```

`--json` gibt das Array der Datensätze in der gerade geschriebenen Reihenfolge zurück. Die Kategorie
entscheidet weiterhin über den Abschnitt — eine Position ordnet *innerhalb* eines Abschnitts, sie
hebt eine Erinnerung nie heraus.

### `nxm forget <SCHLÜSSEL>`

Eine Erinnerung vergessen: ein umkehrbarer Grabstein. Sie fällt aus `memories` und `recall` heraus,
und ein späteres `remember` unter demselben Schlüssel holt sie zurück.

```console
$ nxm forget since-cutoff
forgot since-cutoff

```

```console
$ nxm recall since-cutoff --json
? 1
{"error":{"kind":"not_found","msg":"no memory 'since-cutoff'"}}

```

```console
$ nxm remember "the --since cutoff is exclusive" --key since-cutoff --introduction "the --since cutoff is exclusive"
remembered since-cutoff

```

Die `--json`-Quittung ist eine schlichte Bestätigung, nicht der Grabstein-Datensatz — Vergessen ist
eine Handlung, kein Zustand, den man zurückliest:

```console
$ nxm forget f-bf8fe5f872591385 --json
{"key":"f-bf8fe5f872591385","ok":true}

```

Einen unbekannten Schlüssel zu vergessen ist ein `not_found`, ein Skript kann also „ist jetzt weg"
von „war nie da" unterscheiden.

## Lesen

### `nxm recall <SCHLÜSSEL>`

Der volle Text einer Erinnerung; `--json` liefert den Datensatz.

```console
$ nxm recall auth-jwt
auth uses JWT, not sessions

```

```console
$ nxm recall auth-jwt --json
{"key":"auth-jwt","body":"auth uses JWT, not sessions","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT, not sessions"}

```

Ein vergessener oder nie geschriebener Schlüssel ist ein `not_found`, nie eine leere Antwort:

```console
$ nxm recall ghost --json
? 1
{"error":{"kind":"not_found","msg":"no memory 'ghost'"}}

```

### `nxm memories [SUCHE]`

Die aktive Menge, nach Schlüssel sortiert, oder die Teilmenge, auf die eine
**Groß-/Kleinschreibung ignorierende Teilzeichenkette über Schlüssel und Text** passt. Es gibt kein
Ranking und keinen Relevanzwert: eine Teilzeichenkette kommt vor oder nicht.

```console
$ nxm memories
auth-jwt  auth uses JWT; the session table was dropped in v3
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too
since-cutoff  the --since cutoff is exclusive

```

Filter, die sich kombinieren lassen:

- **`--category <slug>`** — nur Erinnerungen aus diesem Abschnitt.
- **`--scope item|project|global`** — nur Erinnerungen mit dieser Reichweite.
- **`--ordered`** — die gespeicherte Lesereihenfolge (Kategorie, dann Position) statt der
  Schlüsselreihenfolge.

```console
$ nxm memories --ordered
auth-jwt  auth uses JWT; the session table was dropped in v3
since-cutoff  the --since cutoff is exclusive
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too

```

`--json` ist das Array, aus dem eine App die Liste baut — dieselbe Datensatzform, einmal pro
Erinnerung:

```console
$ nxm memories --json
[{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"architecture","scope":"project","refs":[],"ordinal":1,"introduction":"auth is JWT; the session table went away in v3"},{"key":"f-bf8fe5f872591385","body":"the export endpoint pages at 500 rows","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"the export endpoint pages at 500 rows"},{"key":"f-f64a6f027ce71533","body":"the export endpoint pages at 500 rows, so --since must page too","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"the export endpoint pages at 500 rows, so --since must page too"},{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":[],"ordinal":2,"introduction":"the --since cutoff is exclusive"}]

```

### `nxm index`

Das **ganze** Verzeichnis: eine geschriebene Zeile je Erinnerung — ihr Schlüssel und die Einleitung,
die ihr Autor geschrieben hat — in der Lesereihenfolge, die der Sitzungsstart wiedergibt.
`nxm prime` zeigt von diesem Verzeichnis so viel, wie sein Byte-Budget zulässt, und nennt für den
Rest diesen Befehl; nichts, was der Sitzungsstart weglassen musste, bleibt damit unerreichbar.

Erst die Zeilen lesen, dann mit `nxm recall <key>` den Eintrag öffnen, auf den es ankommt. Das ist
die vorgesehene Reihenfolge: das Verzeichnis entscheidet, welcher Volltext die Token wert ist.

```console
$ nxm index
# nexus-memory — the whole index (3)

**One line per memory — this is an index, not the memories.** Each line is the introduction its author wrote for it. Read one in full with `nxm recall <key>`, or search their bodies with `nxm memories <text>`.

- **auth-jwt**: auth is JWT; the session table went away in v3
- **since-cutoff**: the --since cutoff is exclusive
- **f-f64a6f027ce71533**: the export endpoint pages at 500 rows, so --since must page too

```

`--json` spiegelt `nxm prime --json` — dieselben Datensätze, dazu ihre Anzahl:

```console
$ nxm index --json
{"count":3,"memories":[{"active":true,"author":"alice","body":"auth uses JWT; the session table was dropped in v3","category":"architecture","introduction":"auth is JWT; the session table went away in v3","key":"auth-jwt","ordinal":1,"refs":[],"scope":"project","updated":"2026-06-20T10:00:00Z"},{"active":true,"author":"alice","body":"the --since cutoff is exclusive","category":"unsorted","introduction":"the --since cutoff is exclusive","key":"since-cutoff","ordinal":2,"refs":[],"scope":"item","updated":"2026-06-20T10:00:00Z"},{"active":true,"author":"alice","body":"the export endpoint pages at 500 rows, so --since must page too","category":"unsorted","introduction":"the export endpoint pages at 500 rows, so --since must page too","key":"f-f64a6f027ce71533","ordinal":null,"refs":[],"scope":"project","updated":"2026-06-20T10:00:00Z"}]}

```

Es antwortet über dieselben Erinnerungen wie der Sitzungsstart, und genau das macht die Rechnung
„N weitere nicht aufgeführt" im Block wahr: eine Erinnerung mit Reichweite `item`, die Brett-Items
nennt, liest auf diesen Items (`nxf show`) und taucht in keinem von beiden auf. Für alles, was der
Arbeitsbereich hält — jene eingeschlossen —, gibt es `nxm memories`.

### `nxm doc`

Das erzeugte Projektgedächtnis-Dokument ausgeben — die `NEXUS_MEMORY.md`-Projektion, die jeder
Schreibvorgang neu erzeugt. Hier schreibt nichts.

```console
$ nxm doc
<!-- generated by nexus-flow — do not edit; run `nxm remember` instead -->
# Project memory

> **Generated file — do not edit.** Every entry below is a memory in this project's `nxm` store, projected here so the context survives without nexus-flow installed. Change it with `nxm remember` / `nxm forget`; an edit made here is overwritten by the next write. `nxm doc --check` reports drift.

> **You have already been given the index below** — all of it, or as much of it as a session start could carry. It is the same index `nxm prime` replays, which stops at the byte budget the host delivers and says so when it does (`nxm index` prints the whole of it), so there is nothing to gain by reading it again here. Underneath it stands the FULL TEXT of each memory — that is what `nxm recall <key>` serves, and it is meant to be read one memory at a time, when the index tells you a particular one matters. Reading this file end to end is the expensive way to obtain what you already have.

## Index (3)

- **auth-jwt**: auth is JWT; the session table went away in v3
- **since-cutoff**: the --since cutoff is exclusive
- **f-f64a6f027ce71533**: the export endpoint pages at 500 rows, so --since must page too

## Full text

### `auth-jwt`

auth uses JWT; the session table was dropped in v3

---

### `since-cutoff`

the --since cutoff is exclusive

---

### `f-f64a6f027ce71533`

the export endpoint pages at 500 rows, so --since must page too

```

`--check` vergleicht die Datei auf der Platte mit dem Speicher, statt sie auszugeben, und **endet
mit einem Exit-Code ungleich null**, wenn beide auseinandergelaufen sind — die Wache für eine von
Hand geänderte oder nie neu erzeugte Datei. Unter `--json` meldet es `{"path":…,"status":"in_sync"}`
und passt damit direkt in einen Pre-Commit-Hook oder in CI.

### `nxm guide [THEMA]`

Eine eingebettete, offline verfügbare Anleitung ausgeben — ohne Argument die Themenliste. Es braucht
keinen Arbeitsbereich, weil der Inhalt ins Binary kompiliert ist; deshalb antwortet es, bevor
irgendetwas eingerichtet ist.

```console
$ nxm guide --json
[{"summary":"What memory is for, what it needs, and your first remembered fact.","topic":"getting-started"},{"summary":"Keys and auto-keys, the three registers, the retrieval rule, reading order, and the tombstone.","topic":"core-concepts"},{"summary":"Every shipped verb, with its `--json` shape and the errors it can answer with.","topic":"commands"},{"summary":"The `memory_*` MCP tools, what `nxs prime` contributes, and where an agent writes.","topic":"agents-and-mcp"},{"summary":"Take a Claude-host memory store in, and file what a workspace already accumulated.","topic":"import-and-migration"}]

```

`nxs guide` fächert über jeden aktiven Baustein und listet alle drei. Ein Thema, das mehrere
Bausteine tragen — `getting-started`, `core-concepts` und `commands` gibt es je dreimal — wird nie
für Sie ausgewählt: die Klammer nennt `nxf guide …` / `nxm guide …` / `nxc guide …` und überlässt
Ihnen die Wahl.

## Arbeit hereinholen

### `nxm import`

Einen vorhandenen Claude-Host-Gedächtnisspeicher übernehmen. Jeder Fakt kommt unter seinem
Frontmatter-`name:` als stabilem Schlüssel herein, ein erneuter Lauf schreibt also an Ort und Stelle
und die Quelle wird nie verändert. Ohne `--from` wird das Claude-Gedächtnisverzeichnis dieses
Projekts automatisch erkannt.

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

Den ganzen Weg — samt der Frage, was mit dem geschieht, was Sie schon haben — beschreibt
[Import und Migration](nxm-import-and-migration).

### `nxm migrate <plan|apply|status>`

Die urteilende Migration: die `unsorted`-Erinnerungen dieses Arbeitsbereichs einordnen und seine von
Hand geschriebenen Kontextdokumente Abschnitt für Abschnitt hineinbewegen. `status` meldet den
Stand:

```console
$ nxm migrate status
3 unfiled memories — run `nxm migrate plan --with-judge`

```

```console
$ nxm migrate status --json
{"unsorted":3}

```

`plan` schlägt vor und schreibt nichts; `apply` führt einen entschiedenen Plan aus. Beides steht in
[Import und Migration](nxm-import-and-migration) — dort wird auch der eine Befehl dieses Produkts
erklärt, der ein Modell fragen darf.

## Was bewusst fehlt

- **Kein Löschen.** `forget` ist ein Grabstein, und das Log behält die Historie. Hier radiert nichts.
- **Kein Such-Ranking, keine Embeddings, kein Netz.** `memories <suche>` ist ein Teilstring-Treffer.
  Was zurückkommt, entscheiden Register, die Sie setzen — derselbe Speicher antwortet in sechs
  Monaten gleich.
- **Kein `nxm sync`.** Es gibt ein gemeinsames Op-Log für die ganze Suite, Synchronisation ist also
  eine Suite-Operation: `nxs sync`.
- **Kein `nxm prime` in Ihrer Hand.** Es existiert; der eigene `SessionStart`-Hook von memory führt
  es aus, und der `nxs prime`-Fan-out ruft es ebenfalls auf. Der Einstieg für Benutzer ist der der
  Klammer, damit ein Befehl von Hand jeden aktiven Baustein liefert.
