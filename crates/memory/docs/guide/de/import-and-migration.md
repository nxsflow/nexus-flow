# Import und Migration

Zwei Auffahrten, für zwei verschiedene Lagen. **Import** holt einen vorhandenen
Claude-Host-Gedächtnisspeicher herein. **Die urteilende Migration** ordnet ein, was dieser
Arbeitsbereich schon hält, und bewegt seine von Hand geschriebenen Kontextdokumente Abschnitt für
Abschnitt in das Gedächtnis.

Beide existieren, weil die Gedächtnisregel — dauerhaftes Projektwissen lebt in `nxm remember`, nie
in einer selbstgebauten `MEMORY.md` — für ein Projekt, das schon eine geschrieben hat, eine Auffahrt
sein muss und keine Sackgasse.

## Einen Claude-Host-Gedächtnisspeicher übernehmen

Claude Code führt ein Gedächtnisverzeichnis pro Projekt: einen `MEMORY.md`-Index plus eine
Markdown-Datei pro Fakt, jede mit einem kleinen Frontmatter-Block. `nxm import` liest es und
schreibt die Fakten in diesen Arbeitsbereich.

`nxm init` erkennt dieses Verzeichnis für das aktuelle Projekt und sagt es auf seinem Banner —
*Found 2 existing Claude memories* —, die Auffahrt wird also angeboten und nicht versteckt. Sie zu
nehmen ist ein Befehl:

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

Drei Eigenschaften machen das sorglos wiederholbar:

- **Der Schlüssel ist die stabile Id des Hosts.** Jeder Fakt kommt unter seinem Frontmatter-`name:`
  herein, wortwörtlich.
- **Es ist idempotent.** Weil der Schlüssel stabil ist, schreibt ein zweiter Lauf dieselben
  Erinnerungen an Ort und Stelle. Nichts dupliziert sich:

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

- **Es ist nicht-destruktiv.** Das Quellverzeichnis wird gelesen und nie geschrieben. Ihre
  Claude-Erinnerungen bleiben genau dort, wo sie waren.

`--from <verzeichnis>` zeigt ausdrücklich auf ein Verzeichnis; ohne `--from` wird das
Claude-Gedächtnisverzeichnis dieses Projekts automatisch erkannt. Eine Datei ohne Frontmatter und
der `MEMORY.md`-Index selbst sind keine Fakten und werden übersprungen.

```console
$ nxm import --json
{"imported":2,"keys":["auth-jwt","race-flag"],"ok":true,"source":"claude-memory"}

```

Übernommene Erinnerungen kommen als `unsorted` an, wie jede andere nicht eingeordnete Erinnerung —
und genau dafür gibt es die Migration unten.

Sie kommen von **beads**? Diese Migration gehört der Klammer, weil sie Tickets *und* Erinnerungen
zusammen bewegt: `nxm init --from-beads` willigt ein und läuft über `nxs`.

## Die urteilende Migration

Eine Erinnerung, die niemand eingeordnet hat, ist `unsorted`, und ein Arbeitsbereich, dessen
Erinnerungen alle `unsorted` sind, hat eine Liste statt einer Argumentation — nichts entscheidet,
welchem Fakt eine Leserin zuerst begegnet. Sie einzuordnen ist ein **Urteil**: ob ein Fakt eine
Regel ist oder eine Architekturnotiz, ob er überall gilt oder nur hier. Keine Heuristik fällt diese
Entscheidung, deshalb ist die Migration um einen Menschen herum gebaut (oder um ein Modell, das ein
Mensch gefragt hat).

Darum sind es zwei Befehle und nicht einer. `plan` liest und schlägt vor; jemand entscheidet; `apply`
schreibt. Ein Dokument reist zwischen ihnen, damit **das, was geprüft wurde, auch das ist, was
angewendet wird**.

### Wo es steht

```console
$ nxm migrate status
3 unfiled memories — run `nxm migrate plan --with-judge`

```

```console
$ nxm migrate status --json
{"unsorted":3}

```

`status` meldet außerdem, ob auf diesem Strom bereits eine Migration gelaufen ist — die Marke reist
mit dem Op-Log, ein zweites Gerät sieht also den früheren Lauf, statt einen zweiten zu beginnen.
`nxs prime` zeigt dieselbe Erinnerung, solange etwas nicht eingeordnet ist, und schweigt, sobald
nichts mehr offen ist.

### Plan

`plan` sammelt zweierlei Einträge: jede Erinnerung, die noch als `unsorted` abgelegt ist, und jeden
`##`-Abschnitt der von Hand geschriebenen Kontextdokumente dieses Arbeitsbereichs (`CLAUDE.md`,
`AGENTS.md`, `GEMINI.md`, `NEXUS_MEMORY.md`). Es schreibt überhaupt nichts.

```console
$ nxm migrate plan
migration plan: 2 unfiled memories, 0 document sections
  classify   f-f64a6f027ce71533       unsorted / project
  classify   since-cutoff             unsorted / item
  skipped    AGENTS.md (generated)
  skipped    NEXUS_MEMORY.md (generated)

Nothing is judged yet. Run `nxm migrate plan --with-judge --json > migration.json` to have a coding assistant propose the filing, review it, then `nxm migrate apply --plan migration.json`.

```

**Ein erzeugtes Dokument wird ganz übersprungen**, und die Regel hängt am Inhalt, nicht am
Dateinamen: eine Datei, deren Kopf einen Generiert-Hinweis trägt, und jeder Abschnitt, der einen vom
Assembler verwalteten Block überlappt, bleiben unangetastet. Eine Projektion zurück in den Speicher
zu holen erklärte die Projektion zur Quelle — genau die Schleife, die `nxm doc --check` verhindert.
Anderswo ist `AGENTS.md` handgeschrieben; geprüft wird deshalb, was die Datei über sich selbst sagt,
nicht wie sie heißt.

**Ein Abschnitt ist eine Erinnerung.** Geschnitten wird bei `##`, ein `###`-Unterabschnitt bleibt
also bei der Argumentation, zu der er gehört. Eine zusammenhängende Einleitung muss als EIN Gedanke
überleben.

`--json` gibt das Plandokument aus — die Einträge plus die Anweisungen und das Vokabular
(`categories`, `scopes`), das ein Urteil braucht. Dieses Dokument ist es, das `apply` zurücknimmt.

Jeder Eintrag trägt außerdem eine `introduction`: die eine Zeile, die der Sitzungsstart für diese
Erinnerung einspielt (6j6v.xbnh). Für eine Erinnerung, die schon eine hat, wird sie mitgeführt; für
eine ohne bleibt sie leer, und ein Urteil füllt sie. Ein Eintrag mit `action: remember` wird ohne
sie abgewiesen, mit Nennung des Eintrags und bevor eine einzige Op geschrieben ist.

### Urteil

`--with-judge` schickt den Plan durch einen Coding-Assistenten, statt jede Entscheidung auf ihrem
Status quo zu belassen. Blankes `--with-judge` nimmt `claude`; `--with-judge <assistent>` nennt
einen anderen.

```bash
nxm migrate plan --with-judge --json > migration.json
```

**Das ist der eine Befehl in memory, der ein Modell fragen darf, und er schwächt das
Offline-Versprechen nicht.** Verboten ist ein Modellaufruf im *Schreib*pfad — er machte jedes
`nxm remember` unbestimmt und netzabhängig —, und das ist strukturell durchgesetzt: die Bibliothek
bindet überhaupt keinen HTTP-Client ein. Der Urteilende hier ist ein **externer Prozess**, den
dieser Befehl startet, weil Sie ihn darum gebeten haben; die Engine bindet also weiterhin nichts ein.

`--judge-timeout` ist großzügig voreingestellt (`30m`), geschrieben als `<n>` plus `s`/`m`/`h` oder
`0`, um beliebig lange zu warten: ein Urteilender, der einen großen Plan liest, denkt berechtigt
minutenlang nach, und ein Limit, das ihn unterbricht, kostet das ganze Urteil.

Lesen Sie den Plan, bevor Sie ihn anwenden. Ihn von Hand zu bearbeiten ist vorgesehen — genau dafür
existiert das Dokument.

### Apply

```bash
nxm migrate apply --plan migration.json
```

`apply` ordnet die Erinnerungen ein, schreibt die gewählten Dokumentabschnitte als neue Erinnerungen
hinein, übernimmt die Reihenfolge des Plans als Lesereihenfolge, bewegt die migrierten Abschnitte aus
ihren Quelldokumenten heraus und hinterlässt die Marke. `--plan -` liest das Dokument von stdin.

Vier Dinge lohnt es sich vorher zu wissen:

- **Es lehnt einen Plan ab, den niemand beurteilt hat.** Ein Eintrag, der noch auf `unsorted` steht,
  ist keine Entscheidung, und ihn als eine abzulegen erklärte den Status quo zum Urteil.
- **Es ist ein Verschieben, kein Kopieren.** Die migrierten Abschnitte werden aus ihrem
  Quelldokument entfernt, denn eine Kopie hinterlässt zwei Quellen für eine Wahrheit.
  `--keep-sources` schaltet das ab.
- **`--dry-run` meldet, was geschähe, und schreibt überhaupt nichts.** Führen Sie es zuerst aus.
- **Es läuft einmal pro Strom.** Ein zweiter Lauf — auf einem anderen Gerät oder aus einem Plan, der
  vor dem ersten gespeichert wurde — wird abgelehnt, weil zwei unabhängige Einordnungen derselben
  Erinnerungen per Last-Writer-Wins zu einer Mischung zusammenlaufen, die keine von beiden
  entschieden hat. `--again` setzt sich darüber hinweg, wenn Sie es so meinen.

Danach meldet `nxm migrate status` `0` offene Einordnungen, `nxs prime` hört auf zu mahnen, und
[`nxm memories --ordered`](nxm-core-concepts) liest sich als die Argumentation, in die der Plan sie
gebracht hat.

## Weiter

- [Kernkonzepte](nxm-core-concepts) — Kategorien, Reichweite und die Lesereihenfolge, die ein Plan
  entscheidet.
- [Befehle](nxm-commands) — die Flags, die diese Verben nehmen.
