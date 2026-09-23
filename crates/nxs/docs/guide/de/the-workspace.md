# Der Arbeitsbereich

`nxs init` hinterlässt Zustand an drei Stellen. Diese Seite sagt, was jede davon ist — damit klar
ist, was du bearbeiten darfst, was generiert wird und was diese Maschine nie verlässt.

## `.nxs/` — der Arbeitsbereich selbst

```
.nxs/
  config.toml    welche Bausteine aktiv sind, plus deren eigene Einstellungen
  db.sqlite      der eine Speicher, in den jeder aktive Baustein schreibt
  replica.toml   die Identität dieser Maschine: Id-Präfix und Site-Id
  signing.key    der Schlüssel, mit dem jede Op dieses Arbeitsbereichs unterschrieben wird — nur für dich lesbar
  .gitignore     ein einzelnes `*` — das Verzeichnis ignoriert sich selbst
```

`config.toml` ist die einzige Datei hier, die für menschliche Augen gedacht ist:

```toml
active_modules = ["flow", "memory", "chat"]

[flow]
plugin = "issue-tracker"
```

Über `active_modules` fächern `nxs prime` und `nxs guide` auf, und jeder Baustein steuert seine
eigene `[<baustein>]`-Tabelle bei. Du darfst die Datei bearbeiten, aber `nxs init` ist der sicherere
Weg — es prüft die Namen und führt die Einrichtung jedes Bausteins mit aus.

**Der Speicher ist gerätelokal und git-ignoriert.** `.nxs/.gitignore` ist ein einzelnes `*`, der
Arbeitsbereich landet also nie in einem Commit: dein Brett wandert nicht mit deinem Code. Das ist eine
bewusste Entscheidung und kein Versäumnis — geteilt wird Zustand über einen Sync-Strom (`nxs sync`)
und eine Entscheidung über den Pull Request. Wer den Tracker in git erwartet hat, sollte genau das
zuerst wissen.

**`signing.key` ist ein Geheimnis, und es ist die Identität dieses Arbeitsbereichs.** Jede Op, die
der Arbeitsbereich schreibt, wird damit unterschrieben, und eine andere Maschine, die seinem Schlüssel
vertraut (`nxs sync key`, dort dann `nxs sync trust add`), lässt ihre Agenten auf das handeln, was
dieser Arbeitsbereich anstößt. Unter macOS und Linux ist sie nur für dich lesbar; unter Windows ist
sie so privat wie der Ordner, in dem der Arbeitsbereich liegt. Kopiere sie nicht auf eine andere Maschine — ein zweiter Arbeitsbereich
ist ein zweites Replikat mit eigenem Schlüssel — und lösche sie nicht: Ein neuer Schlüssel ließe die
früheren Ops dieses Arbeitsbereichs für Maschinen, die dem alten vertrauten, fremd aussehen.

Ein vorhandenes `.nexusflow/`-Verzeichnis aus einer älteren Version wird beim Öffnen in `.nxs/`
umbenannt — idempotent und nur dort, wo der Arbeitsbereich wirklich aufgelöst wird. So bleibt nichts
zurück, und ein altes Verzeichnis mehrfach zu öffnen ist harmlos.

## Die geteilte Agenten-Datei

Jeder aktive Baustein steuert einen verwalteten Abschnitt zu `AGENTS.md` bei, zusammengesetzt zu
einem Dokument statt zu dreien, die sich widersprechen. Diese Datei wird **committet**: Sie ist der
Weg, auf dem ein Beitragender — oder ein Agent auf einer anderen Maschine — die Konventionen dieses
Projekts erfährt.

Die `CLAUDE.md` ist bewusst keine zweite Kopie. Sie gehört dem Wirt, der die Sitzungs-Hooks weiter
unten *ausführt* — ein Block dort wäre eine zweite Zustellung dessen, was der Hook der Sitzung
bereits übergeben hat, und die Speicher-Bindung darin käme als Volltext jeder Erinnerung an statt
als gekürzter Index. `nxs init` nimmt einen ausgemusterten Block deshalb nur noch aus der
`CLAUDE.md` *heraus*; es schreibt nie einen hinein.

## Die Sitzungs-Hooks

`nxs init` verdrahtet **einen SessionStart-Hook je aktivem Baustein** in `.claude/settings.json`,
jeder führt das `prime` seines eigenen Bausteins aus. **Memorys** Eintrag — und nur seiner — trägt
zusätzlich ein `|| cat NEXUS_MEMORY.md`, damit auch jemand ohne installierte Suite das Gedächtnis des
Projekts bekommt: Die Settings-Datei ist committet, und auf seiner Maschine sind die
Baustein-Binaries schlicht keine Befehle. Es hängt an einem Eintrag statt an allen, weil drei
Einträge dieselbe Datei dreimal in dieselbe Sitzung liefern würden.

Ein fehlschlagender Hook darf laut fehlschlagen, statt mit einem `echo` übertüncht zu werden. Ein
Hook, der mit Null endet und nichts geliefert hat, ist schlimmer als einer, der sagt, dass das
Werkzeug fehlt.

## Prüfen, ob alles gesund ist

```bash
nxs doctor
```

`doctor` — `nxs status` ist derselbe Befehl — meldet die aktiven Module, die Schema-Version und ihren
Stand, die Replica-Identität, den Sync-Zustand, die Zahl der Ops und eine Integritätsprüfung der
Datenbank. Dazu kommt eine Warnzeile, wenn noch eine halbfertige Migration von beads verdrahtet ist,
und eine, wenn zwei Ops dieselbe `(lamport, site)`-Koordinate tragen. Das kommt nur in einem Log vor,
das geschrieben wurde, bevor nxs es abweisen konnte, und dort entscheidet bei einer gleichzeitigen
Änderung die Ankunftsreihenfolge statt der CRDT-Ordnung. Ein sauberer Arbeitsbereich gibt keine der
beiden aus. Er arbeitet allein auf dem Fundament: unabhängig davon, welche Bausteine aktiv sind, und
ohne eine Produktsicht zu öffnen — er antwortet also auch dann noch, wenn ein Baustein unglücklich
ist.

```bash
nxs migrate
```

Das Schema wird beim Öffnen ohnehin angehoben; `migrate` ist der ausdrückliche Hebel für CI und
Reparatur. Es sagt, welcher der beiden Fälle eingetreten ist, statt still erfolgreich zu sein —
`schema v3 → v4`, wenn wirklich migriert wurde, und `workspace db already current (schema v4)`,
wenn nichts zu tun war, was in CI die übliche Antwort ist.

## Weiterführend

- Einen Arbeitsbereich zwischen Maschinen oder Menschen teilen: `nxs sync` und
  [einen Relay betreiben](nxf-running-a-relay) für die Serverhälfte.
- Was jeder Baustein mit dem Speicher tut: [Module](nxs-modules).
