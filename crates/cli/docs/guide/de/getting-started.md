# Erste Schritte

`nxf` ist die nexus-flow Agent-CLI: eine schlanke, deterministische, offline-first Schnittstelle
über dem Engine-Kern. Dieser Leitfaden führt Sie von einem leeren Verzeichnis zu einer geplanten,
nachverfolgten Arbeitseinheit. Jeder Befehl unterstützt `--json` für maschinenlesbare, byte-stabile
Ausgabe — das ist der Vertrag, auf dem Agenten aufbauen.

## Installation

Laden und installieren Sie das neueste Release mit dem Installationsskript:

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

Es erkennt Ihre Plattform, verifiziert den Download (sha256 + Signatur) und legt eine Binary in
Ihren `PATH`, **`nxs`** (das Umbrella, das die Tools zusammenführt), mit **`nxf`** (dem
Issue-Tracker), **`nxm`** (dauerhaftem Agenten-Gedächtnis) und **`nxc`** (dem Kanal) als Links
darauf. Sie installieren einmal und entscheiden **pro Workspace**, welche Tools Sie aktivieren.
Prüfen Sie es:

```bash
nxf --version
```

## Einen Workspace initialisieren

Ein Workspace ist ein `.nxs/`-Verzeichnis in Ihrem Projekt — ein geteilter Speicher, in den jedes
aktivierte Tool schreibt. Am schnellsten richten Sie ihn über das Umbrella ein, das fragt, welche
Tools Sie nutzen wollen, und alles verdrahtet:

```bash
nxs init
```

Es assembliert die geteilten Agent-Dateien und verdrahtet einen SessionStart-Hook je aktivem Tool
(`nxf prime`, `nxm prime`, `nxc prime`), sodass Ihrem Agenten ab dann zu Sitzungsbeginn der Kontext
jedes aktiven Tools übergeben wird (siehe
[Das nxs-Umbrella](#das-nxs-umbrella)). Für einen Agenten oder ein Skript ist es nicht-interaktiv —
`nxs init --module flow --module memory` (oder `--json`) richtet denselben Stand ohne Prompt ein.

Wenn Sie nur den Issue-Tracker wollen, initialisieren Sie ihn direkt. Das ist eine **bewusste
Plugin-Wahl** — es gibt keine Voreinstellung, denn das Plugin bestimmt das Vokabular, das Sie
lesen und schreiben (siehe [Plugins](nxf-plugins)). Für Softwarearbeit wählen Sie `issue-tracker`:

```bash
nxf init --plugin issue-tracker
```

Beide Wege landen auf demselben `.nxs/`-Workspace, denselben Hooks je aktivem Modul und demselben
Eintrag beim Hintergrunddienst — die Liste, die er abarbeitet, ist das, was einer hier erklärten
Frist oder einem Fenster überhaupt jemanden gibt, der hinsieht. Jede id in diesem
Workspace wird unter einem kurzen, stabilen Namensraum namens `prefix` vergeben
(die Beispiele unten verwenden `ab12`). Führen Sie `nxf init --help` aus, um die verfügbaren
Plugins und ihre Beschreibungen zu sehen.

## Ihre ersten Items erstellen

Items sind **Projekte** und **Tasks**. Erstellen Sie ein Projekt, um die Arbeit zu gruppieren,
dann die Tasks darunter. Der `--json`-Datensatz ist die kanonische Form — beachten Sie die
Feldnamen auf Engine-Ebene (`type`, `belongs_to`, `status`), die sich unabhängig vom aktiven
Plugin nie ändern:

```console
$ nxf create --type epic --title "Ship v1" --description "Cut the first release" --priority P1 --json
{"archived":null,"assignee":null,"belongs_to":null,"closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Cut the first release","design":null,"due":null,"id":"ab12.0001","priority":"1","status":"open","title":"Ship v1","type":"epic"}
```

```console
$ nxf create --type feature --title "Write the CLI" --description "Build the command-line tool" --priority P1 --due 2026-12-31 --parent ab12.0001 --json
{"archived":null,"assignee":null,"belongs_to":"ab12.0001","closed_at":null,"closing_comment":null,"completion_criterion":null,"defer_until":null,"deleted":null,"description":"Build the command-line tool","design":null,"due":"2026-12-31","id":"ab12.0002","priority":"1","status":"open","title":"Write the CLI","type":"feature"}
```

Der neue Task `belongs_to` das Projekt (über `--parent`), hat ein Fälligkeitsdatum und trägt
Priorität `1`.

## Sehen, woran zu arbeiten ist

Zustände wie *ready* sind **abgeleitet**, nie gespeichert (siehe
[Kernkonzepte](nxf-core-concepts)). Da nichts es blockiert, ist die Arbeit ready, und `next` ordnet
sie nach der Policy des Plugins. Die menschliche Ansicht spricht das issue-tracker-Vokabular —
`P1`, `open`:

```console
$ nxf next

0001  P1  open  [epic]  Ship v1
0002  P1  open  [feature]  Write the CLI
    ↳ 0001 · Ship v1
```

Das ist die gesamte Schleife: einmal `init`, Arbeit `create`, dann `next` Ihnen sagen
lassen, wohin es geht. Lesen Sie von hier aus [Kernkonzepte](nxf-core-concepts) für das Modell, oder
[Befehle](nxf-commands) für eine vollständige Arbeitssitzung.

## Das nxs-Umbrella

`nxs` ist die eine Binary unter der Suite; `nxf`, `nxm` und `nxc` sind Links darauf.
`nxs` führt zusammen, was Sie in einem Workspace aktivieren — über den einen geteilten
`.nxs/`-Speicher:

- `nxs init` — die Suite einrichten: wählen, welche Tools genutzt werden (interaktiv, oder
  `--module …`/`--json` für einen Agenten), die geteilten Agent-Dateien assemblieren und einen
  SessionStart-Hook je aktivem Modul verdrahten (`nxf prime`, `nxm prime`, `nxc prime`).
- `nxs prime` — das Sitzungs-Bootstrap, das Sie von Hand ausführen: es fächert an das `prime` jedes
  aktiven Tools mit einer geteilten Uhr aus und hängt die Ergebnisse aneinander. Ein flow-only-
  Workspace liefert genau flows prime; memorys kommt dazu, sobald Sie es hinzufügen.
- `nxs sync bind` / `nxs sync run` — den geteilten Speicher mit einem Relay synchronisieren (ein
  Op-Log, also ist Synchronisieren eine Suite-Operation, keine pro-Tool-Operation).
- `nxs migrate` — den Workspace auf das aktuelle Schema heben und eine veraltete
  SessionStart-Verdrahtung auf die aktuelle bringen: ein Eintrag je aktivem Modul. Es hebt beide
  früher ausgelieferten Formen — den vor-v0.6.0-Hook `nxf prime` und den einzelnen
  `nxs prime`-Umbrella-Hook — und ergänzt den Eintrag eines Moduls, das seither dazugekommen ist.
  Ein Workspace ohne Hook von uns bleibt unangetastet; `migrate` repariert eine Verdrahtung, es
  entscheidet nicht, dass Du eine willst. Idempotent.
- `nxs doctor` (Alias `nxs status`) — eine tool-übergreifende Diagnose: aktive Module,
  Schema-Version, Replica-Identität, Sync-Status und Speicher-Integrität.

Jedes Tool behält seinen eigenen gebrandeten Init, wenn Sie es direkt tippen (`nxf init`,
`nxm init`); alle landen auf demselben `.nxs/`-Speicher und denselben Hooks je aktivem Modul.
