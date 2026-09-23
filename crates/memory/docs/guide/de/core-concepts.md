# Kernkonzepte

Sechs Ideen tragen das ganze `nxm`: ein Fakt und sein Schlüssel, die Einleitung, unter der er
gelesen wird, die drei Register, die ihn einordnen, die Abrufregel, die entscheidet, wo er gelesen
wird, die Lesereihenfolge und der Grabstein. Alles andere in [Befehle](nxm-commands) ist ein Weg,
eines davon zu setzen.

## Ein Fakt und sein Schlüssel

Eine **Erinnerung** ist ein kurzer Text unter einem **Schlüssel**. Der Schlüssel ist ihre Adresse —
der Griff, zu dem Sie zurückkehren, wenn sich der Fakt als falsch oder unvollständig herausstellt:

```console
$ nxm remember "auth uses JWT; the session table was dropped in v3" --key auth-jwt --introduction "auth is JWT; the session table went away in v3" --json
{"key":"auth-jwt","body":"auth uses JWT; the session table was dropped in v3","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT; the session table went away in v3"}

```

```console
$ nxm memories
auth-jwt  auth uses JWT; the session table was dropped in v3

```

Das ist *dieselbe* Erinnerung wie vorher, weiterentwickelt — keine zweite. **Einen Fakt unter einem
stabilen Schlüssel an Ort und Stelle weiterzuentwickeln ist die Gewohnheit, die sich lohnt**, denn
die Alternative ist ein Speicher, der drei halbwahre Fassungen derselben Regel hält und keinen Weg
zu erkennen, welche gilt.

`author` und `updated` datieren den **Fakt**: sie bewegen sich, wenn sich der Text bewegt, und nur
dann.

## Die Einleitung

`--introduction` ist **Pflicht**, und sie ist das Einzige an einer Erinnerung, was ein Sitzungsstart
jemals zu sehen bekommt. `nxs prime` spielt genau eine Zeile je Erinnerung ein — den Schlüssel und
diese Zeile — und sonst nichts; der Rumpf wird bei Bedarf mit `nxm recall <key>` geholt. Der Block
trägt so viele dieser Zeilen, wie sein Byte-Budget zulässt, und nennt für den Rest `nxm index` —
eine lange Zeile kostet also zuerst ihrer eigenen Erinnerung den Platz im Sitzungsstart. Die Zeile
ist also keine Beschriftung der Erinnerung, sie ist deren einzige Gelegenheit, überhaupt gelesen zu
werden.

Drei Regeln, alle beim Schreiben durchgesetzt:

- **eine Zeile, höchstens 200 Zeichen.** Eine Abweisung nennt die gemessene Länge, damit Sie nie
  von Hand Zeichen zählen müssen.
- **sagen Sie, was die Erinnerung SAGT, nicht wovon sie handelt.** „the export endpoint pages at 500
  rows" ist eine Zeile, nach der jemand handeln kann; „Notizen zum Export-Endpunkt" ist eine
  Ablagebeschriftung.
- **bei `--category rules` SPRICHT die Zeile die Regel AUS.** „Never run the lean build while the
  tests are running" — nicht „Regeln zum lean build". Das trägt echtes Gewicht: ein Verbot ist nur
  etwas wert, wenn es *vor* dem Fehler da ist, und niemand schlägt eine Regel nach, bevor er sie
  bricht. Unter einem Index muss die Zeile selbst das Verbot aussprechen. Eine Regel, die sich nicht
  in 200 Zeichen sagen lässt, ist keine Regel, sondern ein Aufsatz mit einer Regel darin.

Die Grenze gilt dem Schreibenden, nicht als Kürzung beim Lesen. Das ist Absicht: eine zu lange Zeile
beim Rendern abzuschneiden ergäbe dieselben Bytes und lehrte niemanden etwas, während eine Abweisung
genau den Moment erreicht, in dem jemand entscheidet, was die Erinnerung sagt.

Eine Erinnerung, die vor diesem Register geschrieben wurde, hat keine Zeile. Sie erscheint als
`_(no introduction written yet)_`, und der Block sagt, wie viele in diesem Zustand sind und was zu
tippen ist — die Lücke wird **benannt**, nie mit etwas Abgeleitetem gefüllt. Schließen Sie sie mit
`nxm classify <key> --introduction "<eine Zeile>"`; der Rumpf bleibt dabei unberührt.

### Auto-Schlüssel sind ein Inhaltshash

Ohne `--key` ist der Schlüssel `f-` plus 16 Hex-Zeichen von `sha256(body)` — eine reine Funktion des
Textes. Daraus folgen zwei Dinge, und die sind das ganze Verhalten:

```console
$ nxm remember "the export endpoint pages at 500 rows" --introduction "the export endpoint pages at 500 rows"
remembered f-bf8fe5f872591385

$ nxm remember "the export endpoint pages at 500 rows" --introduction "the export endpoint pages at 500 rows"
remembered f-bf8fe5f872591385

$ nxm remember "the export endpoint pages at 500 rows, so --since must page too" --introduction "the export endpoint pages at 500 rows, so --since must page too"
remembered f-f64a6f027ce71533

```

Ein **byte-gleicher** Text fällt mit der Erinnerung zusammen, die er schon ist; ein
**umformulierter** Text ist ein neuer Fakt unter einem neuen Schlüssel, und der alte bleibt. Das ist
richtig fürs Festhalten — eine Maschine, die notiert, was sie gerade gelernt hat, hat keinen Namen
zu vergeben und darf nichts anderes überschreiben — und falsch fürs Kuratieren. Wenn Sie „dieser
Fakt, korrigiert" meinen, geben Sie ihm einen Schlüssel.

## Die drei Register

Über ihren Text und ihre Einleitung hinaus trägt eine Erinnerung drei Register, und jedes wird
unabhängig gesetzt:

- **`category`** — in welchen Abschnitt des Projektgedächtnisses sie gehört. Ein Kleinbuchstaben-Slug;
  das Dokument beginnt mit `introduction`, `architecture` und `rules`, jeder andere Slug ist genauso
  gültig. Was noch nicht eingeordnet ist, ist `unsorted`.
- **`scope`** — wie weit sie reicht: `item`, `project` (die Vorgabe) oder `global`.
- **`refs`** — die Board-Items, um die es geht, z. B. `ab12.0001`. Die Menge ist kanonisch:
  getrimmt, dedupliziert und sortiert, damit zwei Schreiber, die dieselben Items benennen,
  zusammenlaufen statt zu streiten.

Setzen Sie sie beim Schreiben, oder ordnen Sie eine bestehende Erinnerung nachträglich mit
`classify` ein. **Das Einordnen fasst den Text nie an**, `updated` datiert also weiter den Fakt und
nicht die Ablage:

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

Ein **weggelassenes Flag lässt sein Register in Ruhe** — es ist keine Aufforderung, auf die Vorgabe
zurückzusetzen. `--refs ""` ist die ausdrückliche Art, „keine Referenzen" zu sagen; `--refs`
wegzulassen sagt gar nichts. Und `classify` ohne `--category`, `--scope`, `--refs` oder
`--introduction` wird abgelehnt, statt still nichts zu tun.

## Die Abrufregel

Die Reichweite entscheidet, **wo und wie tief** eine Erinnerung auftaucht. Eine deterministische
Regel, eine totale Funktion der gespeicherten Register — kein Embedding, kein Ähnlichkeitsmaß, kein
Netz:

| Reichweite | `nxs prime` | `nxf show <id>` | `nxf next` |
| --- | --- | --- | --- |
| `item` | — | **voller Text** | ein Hinweis, dass es welche gibt |
| `project` | **eine Verzeichniszeile** | — | — |
| `global` | **eine Verzeichniszeile** | — | — |

Eine `item`-Erinnerung, die Board-Items benennt, fehlt also absichtlich im Sitzungsstart: sie liest
sich dort, wo die Leserin ohnehin hinsieht. Ohne das würde jeder Sitzungsstart um jede jemals
geschriebene Ticket-Notiz wachsen, bis er in Details über ein einzelnes Item ertrinkt.

```console
$ nxm remember "the --since cutoff is exclusive" --key since-cutoff --scope item --refs ab12.0001 --introduction "the --since cutoff is exclusive" --json
{"key":"since-cutoff","body":"the --since cutoff is exclusive","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"item","refs":["ab12.0001"],"ordinal":null,"introduction":"the --since cutoff is exclusive"}

```

```console
$ nxm memories --scope item
since-cutoff  the --since cutoff is exclusive

```

Zwei Kanten lohnen sich zu kennen, denn beide sind die Regel, die Sie schützt, nicht die Regel, die
Sie überrascht:

- **Eine `item`-Erinnerung, die nichts benennt, wird trotzdem wiedergegeben.** Sie hat kein
  Board-Item, an dem sie lesbar wäre; sie dem Sitzungsstart vorzuenthalten machte sie überall
  unsichtbar — und eine unsichtbare Erinnerung ist stiller Datenverlust. `--scope item` vor
  `--refs …` ist der natürliche Zweischritt von Registern, die sich einzeln bewegen, und der Lesepfad
  bleibt darüber total.
- **Die Zugehörigkeit ist exakt und flach.** Eine Erinnerung erreicht ein Item nur, wenn sie
  `item`-Reichweite hat *und* diese Id benennt. Sie läuft nicht über das Board: eine Erinnerung über
  ein Epic sickert nicht auf dessen Kinder, und eine `project`-Erinnerung, die zufällig eine Id
  erwähnt, bleibt draußen — ihre Reichweite sagt, dass sie dem ganzen Arbeitsbereich gehört, und der
  Sitzungsstart ist der Ort, an dem der ganze Arbeitsbereich liest.

memory prüft die **Form** einer Referenz, nie ihre Existenz — es kennt kein flow-Vokabular. Eine
vertippte Id ist also eine Erinnerung, die gegen nichts abgelegt ist. Diese Grenze ist Absicht.

## Lesereihenfolge

Ein Gedächtnisdokument ist eine Argumentation: was der Arbeitsbereich *ist*, dann die Idee, die ihn
trägt, dann die harten Regeln. Diese Argumentation ist die gespeicherte Reihenfolge, und sie hat
zwei Ebenen.

**Zwischen Kategorien** ist der Rang fest: `introduction`, `architecture`, `rules`, dann jede weitere
Kategorie alphabetisch, und `unsorted` zuletzt — was niemand eingeordnet hat, gehört ans Ende und
nicht dorthin, wo sein Slug zufällig einsortiert. **Innerhalb einer Kategorie** kommen ausdrücklich
platzierte Erinnerungen zuerst, in der Position, die `reorder` ihnen gegeben hat, dann alles nie
Umsortierte in der Reihenfolge, in der es geschrieben wurde.

`reorder` ist der eine Befehl, der eine Position schreibt, und er ist für seltenen, bewussten
Einsatz gedacht — alltägliche Schreibvorgänge fassen die Reihenfolge nie an:

```console
$ nxm reorder auth-jwt since-cutoff
reordered 2 memories
  1  auth-jwt
  2  since-cutoff

```

```console
$ nxm memories --ordered
auth-jwt  auth uses JWT; the session table was dropped in v3
since-cutoff  the --since cutoff is exclusive
f-bf8fe5f872591385  the export endpoint pages at 500 rows
f-f64a6f027ce71533  the export endpoint pages at 500 rows, so --since must page too

```

Eine Position hebt eine Erinnerung also nie aus ihrem Abschnitt heraus: `auth-jwt` führt, weil es
unter `architecture` eingeordnet ist, und `since-cutoff` führt den `unsorted`-Rest an, weil es
platziert wurde. Das ist die Reihenfolge, die `nxs prime` wiedergibt und in der `NEXUS_MEMORY.md`
erzeugt wird.

## Vergessen ist umkehrbar

`forget` schreibt einen **Grabstein**. Die Erinnerung fällt aus `memories` und `recall` heraus — sie
zu erfragen ist ein `not_found`, keine leere Antwort — und ein späteres `remember` unter demselben
Schlüssel holt sie zurück:

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

Nichts wird gelöscht, weil hier nichts als Zustand gespeichert ist, den man löschen könnte. Womit
wir bei der letzten Idee sind.

## Ein Speicher, ein Log

Eine Erinnerung ist keine Zeile, die jemand überschreibt; sie ist eine Faltung über **Operationen**
in demselben gemeinsamen `.nxs/`-Op-Log, in das auch flow und chat schreiben. Daraus folgen drei
Dinge, und sie sind der Grund, warum das Modell oben auch in der Zusammenarbeit hält:

- **Die Register sind unabhängig versioniert.** Eine Kategorie zu schreiben kann eine gleichzeitige
  Textänderung nicht zurückrollen, und ein seltenes `reorder` kann niemandes Formulierung anfassen.
- **Konflikte lösen sich auf, sie beschädigen nichts.** Jedes Register ist ein
  keep-if-beats-Last-Writer-Wins-Register; der gewinnende Schreibvorgang entscheidet, und
  `author`/`updated` reisen mit dem Text.
- **Ein `nxs sync` bewegt alles.** Es gibt keine Synchronisation pro Modul, weil es kein Log pro
  Modul gibt.

Und der gesamte Schreibpfad ist offline und deterministisch: kein Befehl in dieser Anleitung fragt
ein Modell. Der eine, der es darf, ist `nxm migrate plan --with-judge`, und den rufen Sie mit
Absicht auf — siehe [Import und Migration](nxm-import-and-migration).

## Das erzeugte Dokument

Jeder Schreibvorgang erzeugt `NEXUS_MEMORY.md` im Wurzelverzeichnis neu, in genau der Reihenfolge von
oben:

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

Es ist eine **Projektion, keine Quelle**: der Speicher ist die Wahrheit, die Historie liefert git.
Es existiert, damit der Kontext für eine Leserin ohne installierte Suite erhalten bleibt und der
`SessionStart`-Hook etwas hat, worauf er zurückfallen kann. Eine Änderung in der Datei wird vom
nächsten Schreibvorgang überschrieben, und `nxm doc --check` ist die Wache, die rot wird, sobald
Datei und Speicher auseinanderlaufen.

Weil das Dokument eine Projektion dessen ist, was `nxs prime` ohnehin wiedergibt, **muss es niemand
aus `CLAUDE.md` einbinden** — das stellte jede Erinnerung zweimal in Ihren Kontext.

## Weiter

- [Befehle](nxm-commands) — jeder Befehl, mit seiner `--json`-Form.
- [Agenten und MCP](nxm-agents-and-mcp) — dasselbe Modell über die MCP-Naht.
- [Import und Migration](nxm-import-and-migration) — einordnen, was Sie schon haben.
