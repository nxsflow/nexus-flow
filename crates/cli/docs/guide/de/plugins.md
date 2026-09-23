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
