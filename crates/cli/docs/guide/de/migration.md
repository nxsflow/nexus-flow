# Migration

Dieser Leitfaden behandelt zwei Arten von Migration: **bestehende Arbeit** nach nexus-flow zu
bringen und zwischen **Hauptversionen** von nexus-flow selbst zu wechseln.

## Einen bestehenden Tracker einbringen

nexus-flow hat keinen Massenimporter — und braucht keinen. Items werden über dasselbe
`nxf create` erstellt, das Sie tagtäglich verwenden, also ist eine Migration ein kurzes Skript,
das Ihren alten Tracker durchläuft und `nxf` einmal pro Item aufruft, wobei jeder Datensatz auf das
[Kernmodell](nxf-core-concepts) abgebildet wird: ein `--type` (Projekt oder Task), ein `--title`,
optional ein `--parent`, `--priority`, `--due` und `--description`. Erstellen Sie Abhängigkeiten danach
mit `nxf dep add` neu. Da jeder Befehl `--json` annimmt und deterministisch ist, ist das Skript
leicht zu schreiben und zu verifizieren:

```bash
# sketch: one create per legacy ticket, then wire dependencies
nxf create --type project --title "Imported backlog" --json
nxf create --type task --title "Legacy #1234" --parent ab12.0001 --priority P2 --json
nxf dep add ab12.0002 ab12.0003
```

Wählen Sie bei `nxf init` das [Plugin](nxf-plugins), dessen Vokabular zum Quell-Tracker passt — das ist
die einzige Vorab-Entscheidung, von der der Import abhängt.

## Einen Sync-Stream binden

nexus-flow ist offline-first: Sie arbeiten lokal gegen `.nxs/`, und ein **Sync-Stream**
bringt jede Replik auf der dauerhaften Server-Wahrheit zur Konvergenz. Binden Sie einen Stream
einmal, dann führen Sie Sync aus, wann immer Sie sich wieder verbinden:

```bash
nxs sync bind --create
nxs sync run --remote https://your-relay.example.com
```

Die Arbeit geht zwischen den Läufen offline weiter; `sync run` tauscht Änderungen aus und führt sie
deterministisch zusammen (die Engine ist eine CRDT, sodass gleichzeitige Bearbeitungen ohne
Koordinator konvergieren). Der Server ist der dauerhafte Datensatz, kein Lock — niemand muss online
sein, damit Sie Fortschritte machen.

## Migration über Hauptversionen hinweg

nexus-flow folgt einfachem SemVer. Innerhalb einer Hauptversion sind Upgrades (`nxs self-update`)
nahtlos einspielbar. Ein Sprung der **Hauptversion** (z. B. `1.x` → `2.0`) ist die einzige Stelle,
an der eine Breaking Change landen darf, und jede solche Änderung wird mit einer Migrationsnotiz
ausgeliefert, die beschreibt, was automatisch ist und was Ihre Aufmerksamkeit braucht. Diese
Notizen werden pro Hauptversion zu einem einzigen Upgrade-Leitfaden zusammengefasst.

Lesen Sie die zusammengefassten Notizen für eine Hauptversion, bevor Sie darauf upgraden — sie
liegen neben diesem Leitfaden auf der Website unter `/docs` und in den Release-Notes der Version.
Innerhalb einer `0.x`-Serie gibt es noch keine Stabilitätsgarantie, prüfen Sie also bei jedem
Update das Changelog.
