# Erste Schritte

`nxm` ist der Baustein **memory** der nexus-Suite: das dauerhafte Projektwissen, das ein Agent zu
Beginn der nächsten Sitzung immer noch hat. Ein Binary liefert drei Werkzeuge — flow (`nxf`) führt
die Arbeit, chat (`nxc`) trägt die Nachrichten darüber, und memory (`nxm`) ist das, was Ihre Agenten
morgen noch wissen.

Dies ist memorys eigene Einleitung: wofür das Modul da ist, was es voraussetzt, und der erste
Befehl, der Ihnen gehört. Die Suite zu installieren und einen Arbeitsbereich einzurichten ist für
alle drei Bausteine dasselbe und steht genau einmal, bei `nxs`.

## Wofür es da ist

Eine Arbeitssitzung sammelt Fakten an, die mehr wert sind als die Sitzung: eine Konvention, eine
Falle, der Grund, warum eine Entscheidung so ausgegangen ist. Im Gesprächsverlauf gelassen sind sie
bei der nächsten Kompaktierung weg; in eine Notizdatei geschrieben liest sie nie wieder etwas.

`nxm` ist der eine dauerhafte Kanal für genau diese Klasse. Eine **Erinnerung** ist ein kurzer Text
unter einem **Schlüssel**, im Arbeitsbereich gespeichert und von `nxs prime` in jede neue Sitzung
zurückgespielt. Auf dem Schreibpfad steht kein Modell, kein Embedding und keine Ähnlichkeitssuche:
was zurückkommt, entscheiden Register, die Sie setzen — derselbe Speicher antwortet heute und in
sechs Monaten gleich.

**Die Regel, die daraus folgt, ist hart, und `nxs prime` sagt sie in jeder Sitzung:** dauerhaftes
Projektwissen gehört in `nxm remember` und nirgendwo sonst. Legen Sie niemals eine `MEMORY.md` oder
eine andere selbstgebaute Gedächtnisdatei an — nichts liest sie zurück, sie wird also nie
wiedergegeben und das Wissen geht still verloren. Wenn Ihr Projekt schon eine hat, ist das keine
Sackgasse: siehe [Import und Migration](nxm-import-and-migration).

## Was es voraussetzt

Einen `.nxs/`-**Arbeitsbereich** — den einen gemeinsamen Speicher, in den alle drei Bausteine
schreiben. Hat Ihr Projekt noch keinen, legt `nxs init` ihn an und fragt, welche Bausteine aktiv
sein sollen. Gibt es ihn schon (weil dort flow oder chat läuft), wird memory hinzugefügt, nie
danebengestellt:

```bash
nxs init --module memory   # memory nicht-interaktiv zum Arbeitsbereich hinzufügen
```

`nxm init` tut dasselbe von memorys Seite aus und ist der Einstieg, wenn memory der einzige
Baustein sein soll, den Sie wollen:

```bash
nxm init
```

Beide Wege enden bei denselben drei Dingen: dem `.nxs/`-Speicher, einer `AGENTS.md` mit einem
kurzen, verwalteten Block, der jedem Agenten, der in diesem Repository landet, sagt, dass er
`nxs prime` ausführen soll, und einem `SessionStart`-Hook je aktivem Baustein — der von memory
führt `nxm prime` für Sie aus.
`nxm init --quiet` richtet dasselbe ohne Banner ein — der nicht-interaktive Einstieg für einen
Agenten oder ein Skript.

## Ihre erste Erinnerung

Schreiben Sie einen Fakt unter einem Schlüssel, den Sie wählen — mit der einen Zeile, unter der ein
Sitzungsstart ihn liest:

```console
$ nxm remember "auth uses JWT, not sessions" --key auth-jwt --introduction "auth is JWT, not sessions"
remembered auth-jwt

```

```console
$ nxm memories
auth-jwt  auth uses JWT, not sessions

```

Der Schlüssel ist die Adresse des Fakts. Verwenden Sie ihn wieder, wird der Fakt **an Ort und Stelle
aktualisiert** — eine Erinnerung, die besser geworden ist, nicht zwei, die einander widersprechen.
Lassen Sie `--key` weg, ist der Schlüssel ein Hash des Textes; das ist die richtige Wahl, wenn eine
Maschine etwas festhält und keinen Namen dafür hat.

`--introduction` ist Pflicht, und es ist keine Beschriftung: es ist das *einzige* an dieser
Erinnerung, was ein Sitzungsstart je zu sehen bekommt. Eine Zeile, höchstens 200 Zeichen, die sagt,
was die Erinnerung sagt. Der Rumpf ist ein `nxm recall <key>` entfernt, und diese Zeile entscheidet,
ob ihn jemand holt. Die Regeln dazu stehen in [Kernkonzepte](nxm-core-concepts) — darunter die
wichtigste: eine unter `rules` abgelegte Erinnerung muss ihre Regel AUSSPRECHEN statt sie
anzukündigen.

Gelesen wird mit `recall` für eine einzelne Erinnerung und `memories` für die Menge, und `--json`
gibt es auf jedem Befehl:

```console
$ nxm recall auth-jwt
auth uses JWT, not sessions

```

```console
$ nxm recall auth-jwt --json
{"key":"auth-jwt","body":"auth uses JWT, not sessions","author":"alice","updated":"2026-06-20T10:00:00Z","active":true,"category":"unsorted","scope":"project","refs":[],"ordinal":null,"introduction":"auth is JWT, not sessions"}

```

Dieser Datensatz ist die Form, die auch alles andere zurückgibt, Feld für Feld: die CLI unter
`--json`, die MCP-Werkzeuge und das erzeugte Dokument projizieren dieselbe gespeicherte Erinnerung.

## Was `nxs prime` beiträgt

Zu Beginn einer Sitzung läuft das `prime` jedes aktiven Bausteins, jedes aus seinem eigenen Hook;
`nxs prime` ist derselbe Fächer, wenn Sie ihn von Hand anfordern. memorys Anteil trägt fünf Dinge:

1. **Die Gedächtnisregel** — dauerhaftes Wissen gehört in `nxm remember`, nie in eine selbstgebaute
   Datei.
2. **Die Korrekturregel** — wenn Sie eine Erinnerung anwenden und das Projekt widerspricht,
   korrigieren Sie sie an Ort und Stelle (`nxm remember "<der korrigierte Fakt>" --key <key>`) oder
   vergessen Sie sie mit `nxm forget <key>`. Die Sitzung, die eine Erinnerung liest, ist der am
   besten informierte Korrektor, den diese Erinnerung je bekommt.
3. **Wie eine Einleitung geschrieben wird**, samt der Auflage für eine `rules`-Zeile — zu lesen,
   bevor geschrieben wird, denn nur dann hilft sie.
4. **Die Befehle**, mit ihren Flags ausgeschrieben, damit eine Sitzung sie nicht raten muss.
5. **Je eine Zeile für jede Erinnerung, die für diesen Arbeitsbereich gilt**, in der gespeicherten
   Lesereihenfolge: der Schlüssel und die Einleitung, die ihr Autor geschrieben hat. Nicht die
   Rümpfe — ein Sitzungsstart, der die mitführte, maß auf den Arbeitsbereichen dieses Projekts
   90 KB, und das liegt jenseits des Punktes, an dem der Wirt den Block überhaupt noch ausliefert.
   Das Verzeichnis selbst endet an derselben Grenze: hat ein Arbeitsbereich mehr Zeilen, als der
   Block tragen kann, lautet die Überschrift `## Memories (showing 34 of 80)`, und eine Zeile
   darunter nennt `nxm index`, das das Ganze ausgibt.

Sie tippen ihn nie selbst — die Hooks tun es, einer je aktivem Baustein. Führen Sie `nxs prime`
nach einer
Kontext-Kompaktierung von Hand erneut aus; memorys Block sagt das oben genau deshalb.

Erinnerungen mit der Reichweite `item` stehen absichtlich *nicht* in dieser Ausgabe: sie lesen sich
stattdessen an den Board-Items, die sie benennen. Das ist die Abrufregel, und sie ist das eine, das
man verstanden haben sollte, bevor man etwas einordnet — siehe [Kernkonzepte](nxm-core-concepts).

## Wo die Erinnerungen außerdem liegen

Jeder Schreibvorgang erzeugt `NEXUS_MEMORY.md` im Wurzelverzeichnis neu: das Gedächtnis des
Projekts als **Bauprodukt**, damit der Kontext auch für eine Leserin ohne installierte Suite und für
jeden, der das Repository auf einer Forge durchblättert, erhalten bleibt. Es ist eine Projektion,
keine Quelle — der Speicher ist die Wahrheit, und eine Änderung in der Datei wird vom nächsten
Schreibvorgang überschrieben. `nxm doc` gibt sie aus; `nxm doc --check` meldet Abweichungen.

## Weiter

- [Kernkonzepte](nxm-core-concepts) — Schlüssel, die drei Register, die Abrufregel, Lesereihenfolge.
- [Befehle](nxm-commands) — die vollständige Referenz, mit `--json`.
- [Agenten und MCP](nxm-agents-and-mcp) — die memory-Werkzeuge eines MCP-Hosts und die prime-Naht.
- [Import und Migration](nxm-import-and-migration) — einen vorhandenen Gedächtnisspeicher übernehmen.
