# Plugins

Ein Plugin bildet den universellen [Kern](nxf-core-concepts) auf die Sprache und Policy eines
Konsumenten ab. Nichts in der CLI kodiert Vokabular fest — alles ergibt sich aus der aktiven
Plugin-Konfiguration, einmal bei `nxf init` gewählt. Ein Plugin legt vier Dinge fest:

- **Vokabular**: die Worte für die beiden Item-Typen und die drei Status.
- **Prioritätslabels**: die Namen für die Prioritätsstufen 0–4.
- **Ranking**: wie `next` die ready-Menge ordnet.
- **Darstellung**: welche Felder `list` und `show` anzeigen.

## Die zwei mitgelieferten Plugins

- **issue-tracker** — Software-Issue-Tracking: Epics und Issues, P0–P4-Prioritäten,
  abhängigkeitsbewusstes Ranking.
- **personal-todo** — Persönliche To-do-Liste: Listen und Todos, now/soon/later-Prioritäten für
  alltägliche Aufgaben.

Ihre Vokabulare unterscheiden sich:

- **issue-tracker**: ein Projekt ist ein `epic`, ein Task ist ein `issue`; die Status sind
  `open` / `in progress` / `closed`; die Prioritäten sind `P0`–`P4`; `next` rankt nach Priorität,
  dann Fälligkeitsdatum, dann id.
- **personal-todo**: ein Projekt ist ein `project`, ein Task ist ein `todo`; die Status sind
  `todo` / `doing` / `done`; die Prioritäten sind `now` / `soon` / `later` / `someday` / `icebox`;
  `next` rankt nach Priorität, dann id (kein Fälligkeitsdatum als Tiebreak).

## Named Variants auflösen: types und priorities

`type` und `priority` sind beide *named variants* — eine kleine, plugin-definierte Menge mit
fester Ordnung. `nxf schema --json` exponiert beide als **gekeyte Map**, sodass eine Regel jedes
der beiden Felder auf jedem Item auflöst:

```text
schema.types[item.type]           # "task" -> "issue"
schema.priorities[item.priority]  # "0"    -> "P0"
```

Der auf einem Item gespeicherte Wert ist der *Handle*, nicht das Label. Für `priority` ist dieser
Handle ein **Ordinal** — `"0"` ist die höchste Priorität, aufsteigend gezählt — bewusst
plugin-unabhängig, damit `next` numerisch ranken kann und gesyncte Items keine Fremd-Labels
tragen. Die `schema.priorities`-Map übersetzt das Ordinal zurück in ein menschliches Label.

**Ordinal-Stabilitätsvertrag.** Innerhalb einer Schema-Version ist die Bindung Ordinal→Bedeutung
**stabil und append-only**: `priority.labels` darf am Ende wachsen, aber **Umsortieren oder
Einfügen** verschiebt die Bedeutung bereits gespeicherter Items und ist daher ein **breaking**
Schema-Change — ein Schema-Version-Bump mit Migration, keine In-place-Änderung.

## Gleiche Daten, nebeneinander

Die Naht ist am klarsten, wenn Sie die **gleichen** Befehle auf den **gleichen** Daten unter jedem
Plugin ausführen. Beide Workspaces unten enthalten drei Tasks: zwei mit Priorität `1` (einer fällig
2026-02-01, einer fällig 2026-03-01) und einer mit Priorität `2`.

`next` zeigt zwei Unterschiede auf einmal — das **Vokabular** und das **Ranking**. Unter
issue-tracker gewinnt das früher fällige P1-Issue (Book venue) den Tiebreak:

```console
$ nxf next

0002  P1  open  [feature]  Book venue
0001  P1  open  [feature]  Draft proposal
0003  P2  open  [feature]  Order badges
```

Unter personal-todo stehen dieselben beiden `soon`-Todos bei der Priorität gleich und fallen auf
die id zurück, sodass das zuerst erstellte (Draft proposal) zuerst kommt — und die Labels sind nun
now/soon/later, todo/doing/done:

```console
$ nxf next

0001  soon  todo  [todo]  Draft proposal
0002  soon  todo  [todo]  Book venue
0003  later  todo  [todo]  Order badges
```

Dennoch ist der gespeicherte Datensatz derselbe — `list --json` ist id-geordnet und hängt nicht
vom aktiven Plugin ab. Das Plugin legt Darstellung und Ranking darüber; es ändert nie die Daten.
Das eine Feld, das ein Plugin besitzt, ist `type`: Das Typ-Set ist plugin-deklariert, also speichert
issue-tracker `feature`, wo personal-todo `todo` speichert. Lässt man dieses eine Feld weg, sind die
Datensätze **byte-für-byte dieselben**:

```console
$ nxf list --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Outline the event proposal","design":null,"due":"2026-03-01","id":"ab12.0001","priority":"1","priority_label":"P1","status":"open","title":"Draft proposal","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Reserve the event space","design":null,"due":"2026-02-01","id":"ab12.0002","priority":"1","priority_label":"P1","status":"open","title":"Book venue","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Print attendee name badges","design":null,"due":null,"id":"ab12.0003","priority":"2","priority_label":"P2","status":"open","title":"Order badges","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

`show` macht den Vokabularunterschied an einem einzelnen Item konkret. Issue-tracker:

```console
$ nxf show ab12.0002

0002 P1 Book venue
==================

TYPE: feature
STATUS: open

DESCRIPTION
-----------

Reserve the event space

DEFINITION OF DONE
------------------



DESIGN
------



NOTES
-----
```

personal-todo, dasselbe `ab12.0002`:

```console
$ nxf show ab12.0002

0002 soon Book venue
====================

TYPE: todo
STATUS: todo

WHY
---

Reserve the event space

DONE WHEN
---------



PLAN
----



LOG
---
```

Diese Verdopplung ist keine Redundanz — sie *ist* die Erklärung. Das `--json` ist die Bedeutung;
das Plugin ist, wie ein bestimmtes Publikum sie liest und rankt. Ein Plugin bei `init` zu wählen
ist daher eine echte Entscheidung; führen Sie `nxf init --help` aus, um die Optionen zu sehen,
bevor Sie sich festlegen.

## Ein Ranking deklarieren

Ein Plugin deklariert unter `[ranking.next]`, wie `next` rankt, und unter `[ranking.list]`, wie
`list --sort rank` rankt. Beide nehmen dieselben Schlüssel und dieselben Regeln:

- `order = [...]` ist die Standardreihenfolge: eine Liste von Schlüsseln, der Reihe nach
  verglichen, mit der Id als letztem Entscheider.
- `[ranking.next.<typ>]` (oder `[ranking.list.<typ>]`) gibt einem Typ eine eigene `order`.
- `types = [...]` sagt, wie die Typen zueinander stehen, sobald einige eine eigene Reihenfolge
  haben.

Ein Schlüssel ist ein **Feldschlüssel**, etwa `{ field = "priority", dir = "asc" }`. Mit einer
`precedence`-Liste rankt ein Feldschlüssel stattdessen nach der Position des Werts in dieser
Liste. In einer Reihenfolge pro Typ, oder in der Standard-`order` von `[ranking.list]`, kann ein
Schlüssel auch ein **abgeleiteter Schlüssel** sein, der ein Item nach seinen Nachbarn im Graphen
rankt:

```toml
[ranking.next]
types = ["project", "action"]
order = [ { field = "priority" } ]

# related: ein Wert, gelesen an den Nachbarn über eine ausgehende Kante (parent, contributes_to,
# dep, mentions). `of = "rank"` liest die Position des Nachbarn in der Reihenfolge seines eigenen
# Typs; `of = { field = "..." }` liest ein Feld, auch ein eigenes Feld. `pick` ist min (Standard)
# oder max.
[ranking.next.action]
order = [
  { related = "parent", type = "project", of = "rank", nulls = "last" },
  { field = "priority" },
]

# count: wie viele Nachbarn passen. children, contributors, dependents und mentioned_by zählen die
# eingehenden Kanten parent, contributes_to, dep und mentions; eine ausgehende Art zählt in die
# andere Richtung. `type`, `status` (eine Liste) und `label` filtern die gezählten Nachbarn.
[ranking.next.project]
order = [
  { related = "contributes_to", type = "goal", of = { field = "order" }, nulls = "last" },
  { count = "children", type = "action", status = ["open", "in_progress"], dir = "desc" },
]

# sum: mehrere Zählungen, mit gleichem Gewicht addiert. `dir` gehört zur Summe.
[ranking.list]
types = ["person"]
[ranking.list.person]
order = [
  { sum = [ { count = "mentioned_by", type = "project" },
            { count = "mentioned_by", type = "action", label = "waiting" } ], dir = "desc" },
]
```

Abgeleitete Schlüssel zählen nur lebende Nachbarn: ein gelöschter fällt weg, ein geschlossener
oder archivierter zählt. Ein Rang wird über die Kandidaten des Typs unter dem Verb berechnet, das
ihn rankt, und Items, die in jedem Schlüssel gleich sind, teilen sich einen Rang. Für `next` sind
diese Kandidaten seine umsetzbaren Items, für `list` die lebenden, nicht archivierten Items. Ein
Nachbar außerhalb dieser Menge, etwa ein geschlossenes Projekt unter `next`, hat keinen Rang und
liest sich als null. Ein `list`-Schlüssel darf den Rang eines Typs lesen, den nur `next` rankt;
die Kandidaten von `next` werden dann zur aktuellen Zeit genommen (`NXF_NOW` legt sie fest). Nur
`list --sort rank` liest diese Zeit.

Das Plugin wird beim Laden abgelehnt, mit dem Schlüssel benannt, wenn:

- ein Schlüssel Arten mischt oder keine nennt;
- ein Schlüssel eine unbekannte Kantenart, einen unbekannten Typ, Status oder ein unbekanntes Feld
  nennt;
- ein Schlüssel einen Teil trägt, den seine Art nicht nimmt, etwa `pick` an einer Zählung oder
  `nulls` an einer Zählung oder Summe (eine Zählung ist nie null);
- `of = "rank"` einen Typ ohne eigene Reihenfolge nennt;
- die Typen sich im Kreis gegenseitig ranken.

Ein Schlüssel `related ... of = "rank"` ohne `type` liest den Rang jedes Typs mit einer
Reihenfolge, auch des eigenen. Das ist ein Kreis, also nennen Sie den `type` des Nachbarn; die
Standard-`order` von `[ranking.list]` lehnt einen solchen Schlüssel ebenfalls ab. Abgeleitete Schlüssel sind in der Standard-`order` von `[ranking.next]` nicht erlaubt, weil diese
Reihenfolge auch `blocked` und `--sort rank` der Lanes rankt.
