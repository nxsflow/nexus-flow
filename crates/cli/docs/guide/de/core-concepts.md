# Kernkonzepte

Dies ist das **plugin-freie** Herz von nexus-flow. Alles hier ist universell — dasselbe Modell
liegt jedem Plugin zugrunde; nur die Worte darüber ändern sich ([Plugins](nxf-plugins)).

## Das Datenmodell

Es gibt zwei Arten von **Item**:

- Ein **Projekt** gruppiert Arbeit (ein Plugin nennt es vielleicht *Epic* oder *Project*).
- Ein **Task** ist eine Arbeitseinheit (ein *Issue*, ein *Todo*).

Items tragen eine kleine, feste Menge an Feldern auf Engine-Ebene — wörtlich sichtbar in jedem
`--json`-Datensatz: `id`, `type`, `title`, `description`, `design`, `status` (`open` / `in_progress` / `closed`),
`priority`, `due`, `defer_until`, `assignee`, `belongs_to` und ein `closing_comment`. Zwei
Beziehungen verbinden Items:

- **belongs-to**: ein Task oder Projekt gehört zu genau einem übergeordneten Projekt (gesetzt mit
  `--parent`). Das ist Containment — die Aufschlüsselung der Arbeit.
- **Abhängigkeiten**: eine gerichtete *muss-zuerst-fertig*-Kante zwischen zwei beliebigen Items
  (Projekt oder Task), unabhängig vom Containment.

Über Abhängigkeiten hinaus können Items **Erwähnungen** tragen — Freitext-Referenzen per Short-id,
die festhalten "dieser Text spricht über jenes Item", ohne es jemals zu blockieren. Jede Änderung
wird als **History** aufbewahrt, und das Schließen eines Items hält einen **Schließkommentar** fest
(das *Warum*, nicht nur das *Dass*).

## Die Ursprungs-Intention bewahren

**Titel** und **Beschreibung** werden kurz nach der Anlage festgezurrt und dann stabil gehalten —
sie sind die Aufzeichnung dessen, *was wir uns am Anfang vorgenommen haben*. Korrigiere sie einmal
direkt nach der Anlage (etwa um ein Review einzuarbeiten), und lass sie danach in Ruhe. Neuer
Kontext und alles, was du während der Abarbeitung lernst, gehen in den **Append-Only-Notizen**-
Stream (`nxf note add`), nicht in ein Umschreiben der Ursprungsfelder. (Das ist heute eine
Konvention; ein künftiges Release kann sie durch einen irreversiblen Per-Field-Lock erzwingen.)

Wenn ein Item wirklich keinen Sinn mehr ergibt, baue es **nicht** zu etwas Fremdem um — das würde
seine Historie auslöschen. Lege stattdessen ein **neues** Item an und **schließe das alte mit einer
Begründung, die auf das neue verweist** (Schließen ist der einzige Weg, auf dem ein Item das Board
verlässt; es gibt kein Hard-Delete). Das ursprüngliche Item bleibt, geschlossen, als Teil der
Aufzeichnung erhalten.

Warum das wichtig ist: So entsteht über die Zeit das Paar **„das wollten wir erreichen"**
(Beschreibung und Design) **↔ „so haben wir es am Ende abgeschlossen"** (Schließkommentar und
Notizen). Genau dieses Paar aus Absicht und Ergebnis ist das, woraus man später lernt. Überschreibt
man die Ursprungs-Intention, verliert man die eine Hälfte davon — und damit die Möglichkeit zu
lernen. (Intention zu bewahren nützt nur, wenn man sie wiederfindet: `nxf search` durchsucht auch
geschlossene Items, sodass das Paar auffindbar bleibt.)

## Abhängigkeiten und Blockieren

Eine Abhängigkeit besagt, dass das *from*-Item auf das *to*-Item warten muss. Angenommen,
`ab12.0002` ("Write the CLI") kann nicht beginnen, bevor `ab12.0003` ("Spec sign-off") erledigt
ist — fügen Sie die Kante hinzu:

```console
$ nxf dep add ab12.0002 ab12.0003 --json
{"msg":"ab12.0002 -> ab12.0003","ok":true}

```

Solange der Blocker offen ist, ist das abhängige Item **blocked**:

```console
$ nxf blocked --json
[{"archived":null,"assignee":null,"belongs_to":"ab12.0001","blockers":[{"id":"ab12.0003","status":"open"}],"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Build the command-line tool","design":null,"due":"2026-12-31","id":"ab12.0002","priority":"1","priority_label":"P1","status":"open","title":"Write the CLI","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

## Ableitung: blocked und next

`blocked` und `next` sind **abgeleitet**, nie gespeichert. Sie sind eine deterministische
Berechnung über die Items und ihre Kanten — es gibt kein *ready*-Flag zu setzen oder zu vergessen.
Ein Item ist *ready*, wenn es offen ist und keinen offenen Blocker hat; das Projekt und der nicht
blockierte Task sind ready, `next` führt sie also auf — während der blockierte Task fehlt:

```console
$ nxf next --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Cut the first release","design":null,"due":null,"id":"ab12.0001","parent":null,"priority":"1","priority_label":"P1","status":"open","title":"Ship v1","type":"epic","type_label":"epic","updated_at":"2026-06-23T00:00:00Z"},{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":null,"deleted":null,"description":"Approve the final spec","design":null,"due":null,"id":"ab12.0003","parent":null,"priority":"2","priority_label":"P2","status":"open","title":"Spec sign-off","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

`next` ist genau diese ready-Menge, in die Reihenfolge gebracht durch die Ranking-Policy des
aktiven Plugins — dieselbe Ableitung, ein Schritt mehr. *Ready* ist ein Zustand, in dem ein Item
**ist**, keine Spur, die man abfragen kann: `next` und `blocked` sind die beiden Fragen, die es
gibt. Da alles berechnet ist, macht das Schließen des Blockers das abhängige Item bei der nächsten
Abfrage sofort ready; nichts muss neu geflaggt werden.

## Zeit: due und defer

Zwei Datumsfelder formen ein Item über die Zeit. `due` ist ein Zieldatum (es kann das Ranking
beeinflussen). `defer_until` verbirgt ein Item, bis ein Datum eintritt — nützlich für Arbeit, die
Sie noch nicht beginnen können. Die Ableitung ist zeitabhängig, aber dennoch deterministisch:
Übergeben Sie `--now`, um den Referenzzeitpunkt zu fixieren. In einem frischen Workspace ein
zurückgestellter Task:

```console
$ nxf create --type feature --title "Pay quarterly taxes" --description "File the quarterly tax return" --priority P3 --defer 2026-07-01 --json
{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":"2026-07-01","deleted":null,"description":"File the quarterly tax return","design":null,"due":null,"id":"ab12.0001","priority":"3","status":"open","title":"Pay quarterly taxes","type":"feature"}
```

Vor dem Defer-Datum ist er **nicht** ready:

```console
$ nxf next --now 2026-06-15T00:00:00Z --json
[]

```

Am oder nach dem Datum liefert genau dieselbe Abfrage den Task — nur `--now` hat sich geändert:

```console
$ nxf next --now 2026-08-01T00:00:00Z --json
[{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"created_at":"2026-06-23T00:00:00Z","defer_until":"2026-07-01","deleted":null,"description":"File the quarterly tax return","design":null,"due":null,"id":"ab12.0001","parent":null,"priority":"3","priority_label":"P3","status":"open","title":"Pay quarterly taxes","type":"feature","type_label":"feature","updated_at":"2026-06-23T00:00:00Z"}]
```

Dieser Determinismus — gleiche Eingaben, gleiches `--now`, byte-identische Ausgabe — macht
nexus-flow sicher steuerbar für Agenten. Als Nächstes: Sehen Sie, wie ein Plugin all das darstellt
in [Plugins](nxf-plugins), oder gehen Sie den vollen Befehlssatz durch in [Befehle](nxf-commands).
