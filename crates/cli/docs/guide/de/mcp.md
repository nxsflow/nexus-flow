# Einen MCP-Host verbinden

MCP-native Hosts — Claude Desktop, Claude Code, Cursor, Windsurf — können die `nxf`-CLI nicht
bedienen, deshalb liefert nexus-flow einen **MCP-Server** (`nxs mcp serve`), der die Lese- **und**
Schreiboperationen Ihres Boards als MCP-Tools über denselben geteilten `.nxs/`-Store bereitstellt.
Dieser Leitfaden verbindet einen Host damit — auch für den Fall, dass **noch nichts installiert ist**.

Eine Host-Konfiguration startet nur einen *Befehl*. Einen Host zu verbinden sind also zwei
Entscheidungen: **welcher Befehl** den Server startet und **welchen Workspace** er öffnet.

## Der schnellste Weg: der `npx`-Runner (ohne Installation)

Wenn Sie Node haben (die meisten Rechner haben es), müssen Sie
nichts vorab installieren. Verweisen Sie den Host auf den `@nexus-flow/mcp`-Runner — er lädt das
signierte `nxs`, verifiziert es, cached es und startet den Server beim ersten Start.

Für **Claude Desktop** fügen Sie dies in `claude_desktop_config.json` ein und starten neu:

```json
{
  "mcpServers": {
    "nxs": {
      "command": "npx",
      "args": ["-y", "@nexus-flow/mcp"]
    }
  }
}
```

Das bedient Ihr **Standard-Board** — einen nutzereigenen Workspace, den nexus-flow beim ersten Start
automatisch anlegt (denselben, den die nexflow.it-Desktop-App nutzt, sodass Sie in beiden dasselbe
Board sehen). Um ihn stattdessen auf ein bestimmtes Projekt zu richten, hängen Sie den Workspace
nach `--` an:

```json
{
  "mcpServers": {
    "nxs": {
      "command": "npx",
      "args": ["-y", "@nexus-flow/mcp", "--", "--workspace", "/absoluter/pfad/zu/ihrem/projekt"]
    }
  }
}
```

Alles nach `--` wird direkt an `nxs mcp serve` durchgereicht. Der erste Start lädt und verifiziert
`nxs` (ein paar Sekunden); spätere Starts nutzen den Cache und starten sofort, auch offline.

## Wenn Sie `nxs` bereits haben: `nxs mcp install`

Mit `nxs` auf Ihrem `PATH` registriert ein Befehl den Server in jedem installierten Host für Sie —
ohne JSON von Hand zu bearbeiten:

```bash
nxs mcp install
```

Es schreibt einen idempotenten Eintrag, der den **absoluten Pfad** Ihres `nxs`-Binaries plus
`mcp serve` nennt, und patcht Claude Desktop, Cursor und Windsurf, sofern gefunden (ein erneuter Lauf
ändert nichts). Mit `--setup` wird im selben Schritt auch das **Board angelegt** — ein Befehl, der
den Server registriert **und** den Workspace erzeugt, den er öffnet:

```bash
# Registrieren + personal-todo-Board am nutzereigenen Standard anlegen (Default für Wissensarbeit):
nxs mcp install --setup

# Registrieren + Coding-Board an dieses Projekt gepinnt anlegen:
nxs mcp install --setup --workspace "$PWD" --plugin issue-tracker
```

Um statt des direkten Binärpfads den portablen, maschinenunabhängigen `npx`-Eintrag von der
Kommandozeile aus zu schreiben, fügen Sie `--runner npx` hinzu.

## Den Workspace wählen

Da ein Host kein Arbeitsverzeichnis hat, kann der Server „dieses Projekt" nicht erraten — Sie sagen,
welches Board er öffnet:

- **`--workspace` weglassen** → der nutzereigene **Standard-Workspace**, beim ersten Start automatisch
  angelegt. Ideal für ein einzelnes persönliches Board, geteilt über Ihre Hosts und die Desktop-App.
- **`--workspace <absoluter-pfad>`** → das Board dieses Projekts. Legen Sie es zuerst an (`nxs mcp
  install --setup --workspace <pfad>` oder dort `nxs init`) — ein expliziter Pfad ohne Board ist ein
  Fehler, keine automatische Anlage.
- Ein einzelner laufender Server kann pro Tool-Aufruf auch auf ein **anderes** Board gerichtet werden,
  indem man einem beliebigen Tool ein `workspace`-Argument übergibt — ein Server, viele Projekte, ohne
  erneute Registrierung.

## `nxs` bootstrappen, wenn es nicht installiert ist

Zwei signierte Wege zu `nxs`, beide fail-closed:

- **Das Ein-Zeilen-Installationsskript** (installiert `nxf`/`nxm`/`nxs` unter `~/.local/bin`, ohne
  sudo):

  ```bash
  curl -fsSL https://nxsflow.com/nxs/install.sh | sh
  ```

- **Der `npx`-Runner** (oben) — er *ist* der Bootstrap für GUI-Hosts: er lädt und verifiziert `nxs`
  bei Bedarf, sodass die Host-Konfiguration das Einzige ist, was Sie hinzufügen.

## Sicherheit

Jeder Abrufpfad verifiziert, bevor er ausführt, und auf dem `npx`-Weg gibt es **keine** unsichere
Ausnahme:

- **sha256** belegt, dass die Bytes beim Transport nicht verändert wurden.
- **minisign** (eine Ed25519-Signatur über das Tarball, geprüft gegen einen im Installer und im Runner
  eingebackenen öffentlichen Schlüssel) belegt, dass die Bytes wirklich von uns stammen. Ein
  manipuliertes Artefakt, ein falscher Schlüssel oder eine fehlende Signatur bricht ab, bevor etwas
  läuft.

Nur Bytes, die **beide** Prüfungen bestehen, werden gecached und ausgeführt. `install.sh` schlägt auf
einem Host ohne verfügbaren Verifizierer fehl (statt unverifizierte Bytes zu installieren), und `nxs
self-update` verifiziert immer erneut.

## Andere Hosts

Dieselben Einträge funktionieren für **Cursor** und **Windsurf** (und jeden MCP-Host, der einen
stdio-Befehl startet). `nxs mcp install` erkennt und patcht alle drei; um einen gezielt anzusprechen,
übergeben Sie `--host claude-desktop | cursor | windsurf`. Sobald verbunden, sendet der Server beim
Verbinden Nutzungshinweise, und `flow_next` liefert Ihre bereite Arbeit — rufen Sie es in jeder
Sitzung zuerst auf.
