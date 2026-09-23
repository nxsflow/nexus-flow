# Erste Schritte

`nxc` ist der Baustein **chat** der nexus-Suite: der dauerhafte Kanal zwischen Ihnen und Ihren
Agenten. Ein Binary liefert drei Werkzeuge — flow (`nxf`) führt die Arbeit, memory (`nxm`) behält
sie, und chat (`nxc`) trägt die Nachrichten darüber. Diese Anleitung führt Sie von einem leeren
Verzeichnis zu einer Persona, die Ihnen antwortet.

chat ist ein **Substrat**, keine Chat-App. Es gibt keinen Dienst zu starten und keinen Server
anzugeben: eine Nachricht ist eine Zeile in derselben `.nxs/`-Workspace-Datenbank, in die auch die
beiden anderen Bausteine schreiben, und die gesamte Oberfläche sind dreizehn Befehle — jeder mit
`--json`.

## Installieren und aktivieren

Installieren Sie die Suite (ein Binary, `nxs`, mit `nxf`/`nxm`/`nxc` daneben):

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

Aktivieren Sie chat dann in Ihrem Projekt:

```bash
nxc init
```

Der Befehl legt den `.nxs/`-Workspace an, falls es noch keinen gibt, registriert darin das Modul
`chat` und meldet, was er verdrahtet hat. Vier Dinge existieren jetzt, und es lohnt sich zu wissen,
was was ist:

- **`.nxs/`** — die Workspace-Datenbank. Nachrichten, Fäden und Kanäle liegen hier, neben den Items
  von flow und den Notizen von memory. Es ist EIN Speicher, deshalb bewegt ein `nxs sync` alles
  davon.
- **`.nxs-personas/`** — der Deklarationsordner. Hier steht Ihr Team. `nxc` liest ihn; niemand außer
  Ihnen schreibt hinein. **Checken Sie ihn ein** und behandeln Sie eine Änderung daran wie eine
  Änderung an Ihrem Code: er wird bei jedem Start frisch gelesen und liegt in genau der Arbeitskopie,
  in der Ihre Agenten arbeiten (siehe [Personas](nxc-personas)).
- **`AGENTS.md`** — ein kurzer, verwalteter Block, der jedem Agenten, der in diesem Repository
  landet, sagt, dass er `nxs prime` ausführen soll.
- **`.claude/settings.json`** — ein `SessionStart`-Hook je aktivem Modul, der von chat führt
  `nxc prime` aus, damit eine frische Sitzung erfährt, wen sie ansprechen kann und was fertig wurde,
  während sie weg war, ohne dass jemand daran denken muss.

Sie nutzen flow oder memory schon? `nxs init --module chat` fügt chat dem vorhandenen Workspace
hinzu — das Verzeichnis `.nxs/` wird geteilt, nie dupliziert. Umgekehrt richtet `nxs init` alle drei
auf einmal ein und ist der interaktive Einstieg.

## Jemanden deklarieren, mit dem man reden kann

**Es gibt kein `nxc agents register` und kein `nxc channels create`.** Ein Team wird *deklariert*, in
Dateien, die Sie lesen, prüfen und einchecken können — nicht zur Laufzeit in die Datenbank einer
einzelnen Maschine registriert. Der erste Schritt ist also, eine Persona aufzuschreiben:

```bash
cat > .nxs-personas/coder.yaml <<'YAML'
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
system_prompt: |
  You are the coder. Do what the trigger message asks. An answer from you means it is done; if
  you cannot get there, say what you are missing rather than answering.
tools: [Bash, Read, Write]
YAML
```

`handle` und `system_prompt` sind die einzigen Pflichtfelder; alles andere hat einen Vorgabewert.
Der vollständige Feldsatz steht unter [Personas](nxc-personas).

**Das durchgehende Beispiel.** Jedes ausgeführte Beispiel in diesen Anleitungen stammt aus einem
kleinen Workspace: dem `coder` von oben, zwei Reviewer-Personas (`general` und `integrity`), die nur
über einen Kanal erreichbar sind, und einer `channels.yaml`, die einen geordneten Kanal
`build-and-ship` und ein Quorum `review` deklariert. `nxc list` ist die Lesung über diesen Ordner —
das, was ein Mensch prüft, und das, was eine App als Verzeichnis darstellt:

```console
$ NXC_ACTOR=alice nxc list
## Who you can address

Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the message from STDIN, and a one-line message may be an argument instead.

**Coder** (handle: `coder`) — Implements a work order on a branch and merges it.

**Build-And-Ship** (handle: `build-and-ship`, members: coder, review) — the declared order a work order runs through

**Review** (handle: `review`, members: general, integrity) — the review quorum — one round asks both reviewers and hands back one verdict

```

Dass die beiden Reviewer in dieser Liste fehlen, ist Absicht: jeder deklariert
`addressable: none`, also führt der Weg zu ihnen über den Kanal, der sie besetzt. Ein Verzeichnis
zeigt, wen Sie ansprechen können, nicht jeden, den es gibt.

## Die erste Nachricht senden

```console
$ NXC_ACTOR=alice nxc send --to coder --ref nxf_ids=ab12.0007 "Add a --since flag to the export command."
-> coder · thread m-00000000000000000000000001

  The answer lands in this thread.
    nxc threads show    m-00000000000000000000000001    — the conversation
    nxc status --thread m-00000000000000000000000001    — where it stands

  To watch instead of coming back: add --stream next time.

```

**Der Block unter der Zeile sagt, wie Sie die Antwort erfahren**, und mehr braucht ein Auftraggeber
nicht: einen Menschen am Terminal weckt niemand, also sind die beiden Lesebefehle das, wozu Sie
zurückkommen. In `--json` kommt derselbe Inhalt als Felder unter `await` — `poll` ist das exakte
Argv zum Ausführen, `done_when` ist der Zustand, der „fertig" heißt (und zwar VOLLSTÄNDIG:
`complete: true` UND ein leeres `outstanding`), `stopped_when` nennt die beiden Marker, von denen
JEDER heißt, dass keine Antwort mehr kommt (`stale: true` ODER `escalated: true`), `answer_at` sagt,
wo in dieser Ausgabe die Antwort steht, und `deadline` sagt, wie geduldig man sein muss. Eine
Schleife, die nur auf `done_when` wartet, hängt ewig, wenn die Runde gestoppt ist.

Eine registrierte Persona bekommt im selben Feld den umgekehrten Rat: `"how": "resume"`,
`"poll": null`. Sie wird vom Koordinator geweckt, wenn die Antwort ankommt — zusammen mit allem
anderen, was eintraf, während sie gearbeitet hat —, also wäre Pollen eine Sitzung, die Züge für eine
Frage verbrennt, deren Antwort sie gleich gereicht bekommt.

Drei Dinge sind in diesem einen Aufruf passiert: die Nachricht wurde gepostet, ein **Faden** darauf
gestempelt, und die Persona `coder` wurde auf einer frischen Sitzung gestartet, mit Ihrer Nachricht
als Auftrag. Die Faden-ID ist der einzige Wert, den Sie behalten müssen — sie ist die Adresse des
Gesprächs.

`--ref` sagt, *worum* es in dem Gespräch geht, und ist in einem bestimmten Sinn Pflicht: nichts zu
sagen ist eine der drei möglichen Antworten und die einzige, die einen Hinweis auslöst. Benennen Sie
den Gegenstand mit `--ref nxf_ids=<item>` (wiederholbar), `--ref branch=…`, `--ref pr=…` — oder
antworten Sie `--no-ref`, wenn es wirklich keinen gibt. Sagen Sie keines von beidem, wird trotzdem
gesendet — die Nachricht ist gepostet mehr wert als verloren —, aber die Quittung trägt den Hinweis
als Feld, damit auch eine App ihn sieht und nicht nur ein Terminal, auf das niemand schaut.

Die Tafel des Fadens sagt, wer eine Antwort schuldet:

```console
$ NXC_ACTOR=alice nxc threads show m-00000000000000000000000001
thread m-00000000000000000000000001 in dm:9445dbc93dfdad401585c42b
  expects:     ab12/coder
  replied:     
  outstanding: ab12/coder
  complete:    false
  working tree: holding
  · ab12/alice Add a --since flag to the export command.

```

`ab12/coder` ist ein **qualifiziertes Handle** — `<origin>/<agent>`, wobei `origin` das eigene
Replika-Präfix dieses Workspace ist. Absender, Erwartungen und Ansprüche werden alle in dieser Form
geschrieben; siehe [Kernkonzepte](nxc-core-concepts).

## Die andere Seite

Die Persona fragt diese Nachricht nie ab: `send` hat ihre Sitzung mit der Nachricht im Prompt
gestartet. Sie erledigt die Arbeit und schließt ab, indem sie im Faden antwortet. Will sie das
Gespräch drumherum — etwa nach einer Kontextverdichtung —, ist `nxc threads show <faden>` die
Lesung, und es ist dieselbe Tafel, die auch der Auftraggeber sieht:

```console
$ NXC_ACTOR=coder nxc threads show m-00000000000000000000000001
thread m-00000000000000000000000001 in dm:9445dbc93dfdad401585c42b
  expects:     ab12/coder
  replied:     
  outstanding: ab12/coder
  complete:    false
  working tree: holding
  · ab12/alice Add a --since flag to the export command.

```

```console
$ NXC_ACTOR=coder nxc reply --thread m-00000000000000000000000001 "Added the flag and a test; branch feat/export-since." --json
{"posted":true,"message_id":"m-00000000000000000000000002","thread_id":"m-00000000000000000000000001","resumed":false,"warnings":[],"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000001","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

Diese Antwort ist es, die alles weiterbewegt: sie löst die Schuld der Persona ein, beendet deren
Sitzung und gibt den Zug an den zurück, der den Faden geöffnet hat. Es gibt kein zweites Verb für
"fertig" und sonst nichts zu merken — genau das macht die Schleife in einem Satz erklärbar, auch
für einen Agenten.

## Wo es steht

`nxc status` liest einen **Vorgang** als Ganzes: den Fadenbaum von seiner Wurzel abwärts, über
Kanalgrenzen hinweg. Die Runde oben ist beantwortet, die schlichte Auflistung hat also nichts zu
melden — sie zeigt, was noch läuft, und `--all` ist der Ort für einen abgeschlossenen Vorgang:

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

„Awaiting you" ist das normale Ende eines Vorgangs — die Agentenseite ist fertig, ein Mensch hat
noch nicht reagiert. Das liest sich bewusst anders als ein Faden, der stehen geblieben ist: diese
beiden auseinanderhalten zu können, ist der ganze Grund für das Merkmal
([Grenzen und Sicherheit](nxc-limits-and-safety)).

## Was `nxs prime` beisteuert

Zu Beginn einer Sitzung läuft das `prime` jedes aktiven Moduls, jedes aus seinem eigenen Hook.
`nxs prime` ist derselbe Fächer, wenn Sie ihn von Hand anfordern. So oder so trägt der Block von
chat drei Dinge:

1. **Die Abstimmungsregel** — Abstimmung zwischen Agenten läuft in diesem Workspace über `nxc`;
   Ad-hoc-Notizen und Kladden werden nicht zugestellt, nicht synchronisiert und nie wiedergegeben.
2. **Die Kernverben**, samt ihrer Pflichtflags, damit eine Sitzung die Form von `send --to` oder
   `reply --thread` nicht raten muss.
3. **Was dieser Workspace deklariert** — wer hier ansprechbar ist und wofür — und, für einen
   Aufrufer, dessen vorige Sitzung dieses Gerät enden sah, die von ihm eröffneten Aufträge, die
   inzwischen fertig wurden. Kein Nachrichtentext: alles, was eine Sitzung bekommt, wird ihr
   geschoben, eine Wiedergabe beim Start wäre also die zweite Kopie (bis nxf 6j6v.4mmk trug der
   Block das gesamte Ungelesene, nxf 6j6v.4d2z hat es entfernt).

Sie tippen den Befehl nie selbst: seit nxf n2m6 + a2a1 verdrahtet der Wirt einen
`SessionStart`-Hook je aktivem Modul, `nxc prime` wird also direkt ausgeführt und nicht über einen
Fächer der Klammer. Führen Sie `nxs prime` nach einer
Kontext-Verdichtung von Hand erneut aus — diese Anleitung ist jetzt der Ort für diese Erinnerung,
nicht mehr der Block von chat selbst (nxf h4d3, Task 3 hat die Zeile "Context Recovery" aus dem
gerenderten Block entfernt, um ins Sitzungsstart-Budget zu passen).

## Weiter

- [Kernkonzepte](nxc-core-concepts) — das Substrat, Identität, Fäden und Zustellung.
- [Personas](nxc-personas) — alles, was eine `.nxs-personas/<handle>.yaml` deklarieren kann.
- [Kanäle](nxc-channels) — einen Kanal deklarieren, und der geordnete Kanal, der *der* Ablauf ist.
- [Befehle](nxc-commands) — die vollständige Referenz, mit `--json`.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — was ein Agent nicht selbst entscheidet.
