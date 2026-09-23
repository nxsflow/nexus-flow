# Befehle

Die vollständige Oberfläche von `nxc`: dreizehn Befehle, in der Reihenfolge, in der eine Sitzung
üblicherweise nach ihnen greift. `nxc <befehl> --help` liefert die generierte Referenz (Flags, Typen,
Vorgabewerte); diese Anleitung ergänzt die Erzählung und die `--json`-Formen. Zwei Flags sind global
— `--json` an jedem Befehl, und `--db <pfad>` (auch `NXC_DB`), um eine Workspace-Datenbank direkt
anzugeben, statt `.nxs/` vom Arbeitsverzeichnis aus zu suchen.

Die Oberfläche ist absichtlich klein. Es gibt genau **zwei Arten, etwas zu sagen** — `send` beginnt
ein Gespräch, `reply` antwortet darin — dazu `withdraw`, mit dem der Mensch, der eine Beauftragung
abgeschickt hat, sie zurücknimmt, wartend oder laufend. Alles andere liest. Wenn Sie ein Verb
suchen, das einen Kanal anlegt, einen Agenten registriert, einen Workflow-Schritt weiterschaltet
oder eine gehaltene Arbeitskopie namentlich zurückgibt, siehe [Was bewusst fehlt](#was-bewusst-fehlt)
am Ende.

## Einrichten

### `nxc init`

Aktiviert chat im aktuellen Verzeichnis: stellt einen `.nxs/`-Workspace sicher, registriert das
Modul `chat` und übergibt dann die gemeinsame Agentendatei und die
SessionStart-Hooks dem `nxs`-Assembler, der sie für *jedes* aktive Modul neu
zusammensetzt.

```bash
nxc init                 # für Menschen, mit Banner
nxc init --json          # die maschinenlesbare Quittung
```

`--quiet` ist stiller Erfolg — es gibt gar nichts aus, und genau das macht es steuerbar:

```console
$ nxc init --quiet

```

### `nxc agent-manifest`

Was chat als seinen Beitrag zur gemeinsamen Agentendatei deklariert — den prime-Befehl und den
Hook — als Daten statt als Prosa:

```console
$ nxc agent-manifest
nexus-chat agent manifest
  prime command:  nxc prime
  hook:           SessionStart → nxc prime

Run with --json for the machine contract the `nxs` umbrella assembles from.

```

`--json` ist dieser Kontrakt; das Dach liest ihn von allen drei Bausteinen und setzt eine `AGENTS.md`
und einen Hook zusammen, sodass das Aktivieren eines zweiten Moduls den Block des ersten nie
überschreibt.

## Wer hier ist

### `nxc list`

Das Verzeichnis: wer in diesem Workspace ansprechbar ist und wofür. Es ist eine Lesung über
`.nxs-personas/` und nichts sonst, zeigt also genau das, was deklariert ist.

```console
$ NXC_ACTOR=alice nxc list
## Who you can address

Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the message from STDIN, and a one-line message may be an argument instead.

**Coder** (handle: `coder`) — Implements a work order on a branch and merges it.

**Build-And-Ship** (handle: `build-and-ship`, members: coder, review) — the declared order a work order runs through

**Review** (handle: `review`, members: general, integrity) — the review quorum — one round asks both reviewers and hands back one verdict

```

Ohne `--persona` sieht ein Mensch das ganze deklarierte Team, während eine Persona, die unter ihrer
eigenen Sitzung läuft, ihr eigenes Adressbuch sieht — erkannt an eben dieser Sitzung. `--persona
<handle>` projiziert die Sicht dieser Persona, gleich wer fragt — nützlich, um vor dem Start zu
prüfen, was ein Agent tatsächlich sehen wird.

`--json` trägt `personas` (`handle`, `job_title`, `job_description`, `direct`), `channels` (`name`,
`members`, `description`), ein Feld `public_channels` mit den sichtbaren Vordertüren (weggelassen,
wenn es keine gibt, und aus der menschlichen Darstellung bewusst herausgehalten — eine Tür, die hier
keine Deklaration benennt, ist auffindbar, nicht ansprechbar) sowie einen Block `declarations`, der
den gelesenen Ordner und die Zahl der Dateien darin nennt. Dieser letzte Block ist der, den man
prüft, wenn die Liste leerer ist als erwartet.

## Etwas sagen

### Woher `<BODY>` kommt

Beide Verben nehmen die Nachricht auf drei Wegen entgegen, in derselben Schreibweise wie `nxf`
(`--description-file`, `--set feld=-`):

```text
nxc reply --thread <id> "lgtm"                # ein Argument — für EINE Zeile
nxc reply --thread <id> - <<'EOF'             # STDIN — für alles andere
<Ihr Text, genau so, wie er ankommen soll>
EOF
nxc reply --thread <id> --body-file urteil.md    # eine Datei; ein Pfad `-` ist wieder STDIN
```

**Alles, was länger als eine Zeile ist, gehört auf STDIN, und das ist keine Stilfrage.** In der
Argumentform bekommt die *Shell* den Text zuerst: Backticks und `$(…)` darin werden ausgeführt und
ersetzt, bevor `nxc` ein Byte sieht. Die längsten Texte dieses Systems sind genau die, die von
beidem voll sind — ein Prüfurteil, das Befehle und ihre Ausgabe zitiert, ein Sitzungsbericht, eine
Beweisführung. Im besten Fall kommt so ein Rumpf still verstümmelt an; eine Rolle, die *fremden*
Text zitiert, führt ihn aus. Auf dem STDIN- und dem Datei-Weg passiert der Text gar kein Quoting,
und die Anführungszeichen um `EOF` sind das, was auch das Heredoc selbst am Expandieren hindert
(`<<EOF` ohne sie tut es weiterhin).

Ein leerer Lesevorgang wird abgelehnt: `nxc reply --thread <id> -` ohne etwas auf STDIN würde eine
leere Antwort posten und die Runde ohne Ergebnis beenden — also schlägt es fehl, statt zu posten.

### `nxc send --to <persona|kanal> <BODY>`

Ein Gespräch eröffnen. `--to` ist Pflicht, und sein Ziel muss **deklariert** sein: ein
Persona-Handle oder ein Kanalname aus `.nxs-personas/`. Die Deklaration entscheidet, was als Nächstes
passiert — eine Persona wird auf einer frischen Sitzung gestartet, ein Kanal fächert nach seiner
eigenen Politik auf — und der Aufruf gibt in beiden Fällen die Faden-ID zurück.

```console
$ NXC_ACTOR=alice nxc send --to coder --ref nxf_ids=ab12.0007 "Add a --since flag to the export command."
-> coder · thread m-00000000000000000000000001

  The answer lands in this thread.
    nxc threads show    m-00000000000000000000000001    — the conversation
    nxc status --thread m-00000000000000000000000001    — where it stands

  To watch instead of coming back: add --stream next time.

```

Flags:

- **`--ref k=v`** (wiederholbar) — worum es in diesem Gespräch geht: `nxf_ids` (ein flow-Item;
  wiederholbar, weil eine erste Nachricht mehrere benennen kann), `branch`, `pr`, `session_id`.
- **`--no-ref`** — die ausdrückliche Antwort, dass es wirklich keinen Gegenstand gibt. „Ich habe
  nachgesehen" zu sagen, wo Schweigen nichts sagt.
- **`--stream`** — dem Faden *und* den Verläufen der Personas zusehen, bis jemand antwortet, statt
  sofort zurückzukehren. Für einen Menschen an der Tastatur; innerhalb einer laufenden Persona
  abgelehnt.
- **`--body-file <pfad>`** — den Rumpf aus einer Datei lesen statt aus dem Argument (UTF-8,
  wortwörtlich); ein Pfad `-` ist STDIN. Siehe *Woher `<BODY>` kommt* weiter oben.
- **`--machine <kennung|name>`** — diesen Chat auf jener Maschine laufen lassen (nur ein
  Persona-Chat). Ohne die Angabe entscheidet `machine:` der Persona, sonst diese Maschine. Nach einer
  Maschine, die nicht online ist, wird **nachgefragt**: Nichts wird gesendet, und die Ablehnung nennt
  die Maschinen, die online sind. Eine hier genannte Maschine gilt auch, wenn sie nicht online ist —
  der Chat wartet dann im Log, bis sie zurück ist. Läuft der Chat woanders, sagt es der Beleg
  (`handed_to`), und auf dieser Maschine startet nichts. Woher die Maschine kommt, steht unter
  [Personas](nxc-personas).

Weder `--ref` noch `--no-ref` zu nennen sendet trotzdem und warnt — als Zeile auf stderr für einen
Leser **und** als Feld `refs_warning` für eine App, damit der Hinweis nicht zwischen beiden verloren
geht:

```console
$ NXC_ACTOR=alice nxc send --to coder "One more thing." --json
{"thread_id":"m-00000000000000000000000002","message_id":"m-00000000000000000000000003","to":"coder","target":"persona","channel":"dm:9445dbc93dfdad401585c42b","session":"m-00000000000000000000000002","expects":["ab12/coder"],"spawned":true,"warnings":[],"refs_warning":"nothing was named as the subject of this conversation. Say what it is about — `--ref nxf_ids=<id>`, repeatable — or `--no-ref` if there really is nothing.","await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000002","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}
warning: nothing was named as the subject of this conversation. Say what it is about — `--ref nxf_ids=<id>`, repeatable — or `--no-ref` if there really is nothing.

```

An einen Kanal zu senden liefert dieselbe Quittung mit `target: "channel"`, der ID des deklarierten
Kanals (`decl:<name>`) und dem Betreuer des Kanals als erwartetem Antwortenden:

```console
$ NXC_ACTOR=alice nxc send --to build-and-ship --ref nxf_ids=ab12.0008 "Ship the export change." --json
{"thread_id":"m-00000000000000000000000003","message_id":"m-00000000000000000000000004","to":"build-and-ship","target":"channel","channel":"decl:build-and-ship","expects":["ab12/__channel__"],"spawned":true,"warnings":[],"refs_warning":null,"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000003","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

Die Felder der Quittung, in der Reihenfolge, in der sie ausgegeben werden: `thread_id`,
`message_id`, `to`, `target` (`persona` oder `channel`), `channel`, `session` (die interne
Sitzungs-ID, wenn eine Persona gestartet wurde), `queued_behind` / `queue_position` (nur vorhanden,
wenn der Anstoß hinter einer Arbeitskopie-Reservierung geparkt ist), `expects`, `deadline` (wenn der
Kanal ein `timeout:` deklariert), `spawned`, `warnings`, `refs_warning`. `warnings` und
`refs_warning` sind immer vorhanden — auch leer und auch `null` —, damit ein Leser nie „kein
Schlüssel" von „nichts zu melden" unterscheiden muss.

Ein Ziel, das keine Deklaration benennt, wird abgelehnt, und die Ablehnung sagt, wo gesucht wurde:

```console
$ NXC_ACTOR=alice nxc send --to nobody --ref nxf_ids=ab12.0008 "Anyone there?" --json
? 1
{"error":{"kind":"not_found","msg":"no such target: nobody (not a declared channel, not a declared persona, not a channel in this workspace) — `nxc list` shows what can be addressed"}}

```

Ebenso eine Persona, die erklärt hat, nur über einen Kanal erreichbar zu sein — und zwar für alle
gleichermaßen, Mensch oder Agent, benannt oder anonym:

```console
$ NXC_ACTOR=alice nxc send --to general --ref nxf_ids=ab12.0008 "Have a look?" --json
? 1
{"error":{"kind":"validation","msg":"general is addressable only through review — send --to review instead"}}

```

### `nxc reply [--thread <FADEN>] <BODY>`

In einem Faden antworten. Eine Faden-ID benennt ein Gespräch für sich, ohne Kanalkontext, und die
Antwort wird an den zurückgeleitet, der auf der anderen Seite steht.

**`--thread` darf entfallen, solange genau ein Gespräch für Sie offen ist.** Das ist der Normalfall
für eine beauftragte Rolle, und genau darum geht es: eine Rolle weiß, was sie zu tun hat und wem sie
antworten darf — eine Id, die sie vor Minuten bekommen hat, mitzuführen ist Buchhaltung, die sie
nicht leisten sollte. `nxc reply -` beantwortet das eine Gespräch, das auf Sie
wartet.

Sind mehrere offen, ist die Flagge wieder Pflicht — Sie schulden Ihrem Auftraggeber eine Antwort
*und* die konsultierte Rolle hat sich gemeldet, und die Engine rät nicht, welches Sie meinten. Sie
müssen es auch nicht herleiten: die Nachricht, die Sie geweckt hat, nennt jedes offene Gespräch mit
dem Befehl, der es beantwortet, und die Ablehnung tut dasselbe, falls Sie die Kurzform trotzdem
versuchen. Die Form mit Id ist immer erlaubt.

```console
$ NXC_ACTOR=coder nxc reply --thread m-00000000000000000000000001 "Added the flag and a test; branch feat/export-since." --json
{"posted":true,"message_id":"m-00000000000000000000000002","thread_id":"m-00000000000000000000000001","resumed":false,"warnings":[],"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000001","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

`resumed` ist nur dann `true`, wenn die Antwort die *eigene* Rückadress-Sitzung des Ziels geweckt
hat — der direkte Eins-zu-eins-Weg. `false` deckt zwei verschiedene Dinge ab (es gab keine Sitzung
zum Wiederaufnehmen, oder es gab eine und die Wiederaufnahme kam nicht an), und `wake_skipped`
daneben trennt sie. Das Wecken beim Abschluss einer Quorum-Tafel ist in diesem Sinn keine
Wiederaufnahme und setzt es nie. `posted` ist nur im No-Op von `--if-unanswered` unten `false`.

- **`--escalate`** — „allein komme ich nicht ans Ergebnis: ich brauche Hilfe oder eine
  Entscheidung." Eines von drei Dingen, die ein Agent mit `reply` sagen darf; die anderen sind „ich
  bin fertig" (eine blanke Antwort) und `--needs-rework` weiter unten. Es ist ein *deklariertes*
  Signal, und genau das macht es brauchbar: der Betreuer eines Kanals verzweigt an einem Bit,
  dessen Bedeutung aufgeschrieben ist, nicht an einer Formulierung, die er deuten müsste. Der Zug
  ist so oder so eingelöst — verschieden ist das Ergebnis.

  **Die Hälfte mit der Entscheidung ist die, die übersehen wird.** Die Marke ist nicht nur für eine
  Aufgabe da, an der Sie blockiert sind. Eine Frage, deren mögliche Antworten Sie alle ausführen
  könnten, die aber nicht Ihnen gehört, ist genau das, was sie trägt — und die Richtung ist, was
  die Marke ausmacht:
  eine Eskalation geht NACH OBEN, zu dem, der Sie beauftragt hat. Stellen Sie dieselbe Frage in
  einer blanken Antwort, wandert sie NACH UNTEN zum nächsten Schritt, der sie so wenig entscheiden
  darf wie Sie.

  **An einem geordneten Kanal ist das Ergebnis, dass die Kette endet.** Der Schritt nach Ihrem wird
  nicht gestartet, die Runde geht so nach oben, wie sie ist, und wer sie beauftragt hat, entscheidet.
  Genau dafür gibt es die Marke: eine Aufgabe, die nicht ausgeführt werden konnte, darf nicht die
  Arbeit beauftragen, die auf sie folgen sollte.

  **Es ist kein Weg, „noch nicht" zu sagen — und nicht die Art, zu warten.** Jede Antwort von
  `reply` beendet den Zug, und ein Aufrufer liest diese als Arbeit, die nicht kommen wird. Haben Sie
  eine eigene Runde beauftragt und warten auf sie, dann beenden Sie Ihren Zug und sagen gar nichts:
  Die Engine sieht die Runde, die Sie geöffnet haben, liest das als Warten statt als fehlende
  Antwort und weckt Sie, wenn sie zurückkommt. Ein blankes `nxc reply` bei offener eigener Runde
  wird abgelehnt — siehe `waiting_on_sub_round` unter `nxc status` weiter unten und
  [channels](nxc-channels).
- **`--needs-rework`** — „was du mir übergeben hast, genügt nicht." Die dritte Antwort, und die
  einzige über **fremde** Arbeit statt über die eigene. Nennen Sie im Rumpf, was in Ordnung zu
  bringen ist: diese Worte werden an die Stelle gereicht, die die Arbeit erzeugt hat, und sie sind
  deren nächste Aufgabe.

  **Angeboten wird es nur, wo der Kanal einen Weg dafür erklärt** (`steps:` → `on_needs_rework:`) —
  genau das macht es zu einem Signal und nicht zu einem erfundenen Token. Ein Schritt ohne diese
  Kante erfährt von zwei Ausgängen, nicht von dreien; dort gesetzt fällt das Bit weg, mit einer
  Warnung `verdict_dropped` auf der Quittung der Antwort. Mit `--escalate` zusammen ist es nicht
  erlaubt: die beiden sind Aussagen über verschiedene Arbeit mit entgegengesetzten Folgen. Siehe
  `nxc guide channels`.
- **`--accept`** — „Ich weiß, was die Prüfung gesagt hat. Es geht trotzdem weiter." Die einzige
  Antwort auf diesem Verb, die **keine** Aussage eines Agenten über vorliegende Arbeit ist: sie
  gehört der Stelle, die eine Runde **beauftragt** hat, gesagt auf dem Faden dieser Runde.

  Sie hebt das Urteil auf, an dem die Runde stehengeblieben ist. Der Schritt, dessen
  `--needs-rework` die Arbeit zurückgeschickt hat, gilt als erfüllt, und der Lauf geht über dessen
  `next:` weiter — derselbe Lauf, kein neuer. Der Folgeschritt bekommt Ihre Worte **zusammen mit dem
  aufgehobenen Urteil** gereicht, damit er weiß, dass das Urteil übersteuert und nicht erfüllt wurde,
  und nicht weiter unten meldet, die Arbeit sei sauber geprüft worden.

  **Ohne dieses Verb hat ein Zyklus, dessen Prüfer nie zufrieden wird, keinen Ausgang.**
  `max_passes` begrenzt EINEN Lauf; jede andere Antwort auf die Runde beauftragt einen neuen Lauf
  und spannt denselben Deckel wieder auf. Das ist kein Randfall — es ist, was beim ersten echten Lauf
  dieser Maschinerie geschah: sechs Durchläufe Entwurf→Prüfung, sechs Urteile, nichts weitergereicht.

  **Ihren eigenen Prüfer dürfen Sie nicht für zufrieden erklären.** Das Verb wird nur auf einem Faden
  angenommen, den Sie selbst eröffnet haben; auf einem Schritt, den Sie *bedienen*, wird es
  namentlich abgelehnt — die Trennung zwischen Erzeugen und Prüfen ist der ganze Grund, warum eine
  Runde zwei Parteien hat. Der Weg eines Agenten dorthin ist `--escalate`; dieses Verb ist die
  Antwort der Stelle darüber.

  **Abgelehnt statt gepostet, wenn es nichts bewegen würde**: keine solche Runde, nicht Ihre, kein
  Urteil zum Annehmen, oder ein Schritt der Runde arbeitet noch. Jede Ablehnung nennt, was fehlt und
  was stattdessen zu tun ist. Aufgezeichnet wird es als eigene Art (`accepted`), damit eine
  Übersteuerung später als Übersteuerung wiederzufinden ist und nicht als bestandene Prüfung.
- **`--if-unanswered`** — nur posten, wenn der Aufrufer auf diesem Faden noch eine Antwort schuldet;
  sonst ein bewusster No-Op (Exit 0, `posted: false`, nichts geschrieben), nie ein Fehler. Es gibt
  ihn für einen einzigen Aufrufer: den Abbau des Agenten-Sidecars, der etwas sagt, wenn eine
  SDK-Sitzung endet, ohne den geschuldeten Faden je beantwortet zu haben, damit dieser nicht für
  immer verstummt. Sie brauchen ihn nicht.

  **Er ist die Rückfallebene, nicht der erste Zug.** Bevor der Abbau im Namen des Agenten spricht,
  gibt er der Sitzung ihren Zug einmal zurück und sagt es ihr — mit dem konkreten Faden, beiden Arten
  einen Zug zu beenden, und jeder von ihr beauftragten Runde, die noch offen ist. Die meisten
  Sitzungen antworten dann. Eine, die erinnert wurde und erneut stumm endet, wird als **Eskalation**
  zurückgegeben: dann gibt es kein Ergebnis, und jemand muss über die Runde entscheiden — eine andere
  Tatsache als ein Zug, der schlicht nichts hervorbrachte, und `escalated: true` in `nxc status` ist,
  wo der Aufrufer es liest.

  **Was durch diese Tür geschrieben wird, ist als Werk der Laufzeit gekennzeichnet.** Jede so
  gesetzte Antwort trägt `substituted: true` in `nxc status`, sodass eine App nie einen
  Nachrichtentext lesen muss, um eine vom Agenten geschriebene Antwort von einer in seinem Namen
  gesetzten zu unterscheiden. Zum Zusammenspiel mit `escalated` siehe
  [Grenzen und Sicherheit](nxc-limits-and-safety).
- **`--body-file <pfad>`** — wie bei `send`: den Rumpf aus einer Datei lesen (`-` ist STDIN). Siehe
  *Woher `<BODY>` kommt* weiter oben.
- **`--stream`** — wie bei `send`: der Antwort beim Eintreffen zusehen.
- **`--machine <kennung|name>`** — diesen Chat an eine andere Maschine übergeben. Die Persona wird
  dort mit dem gestartet, was seit ihrer letzten Antwort offen ist, und erfährt, wo das bisherige
  Gespräch steht; diese Maschine startet nichts. Eine Entscheidung des Menschen: aus einer laufenden
  Sitzung heraus abgelehnt. Nach einer Antwort in einen Chat, dessen Maschine nicht online ist, wird
  genauso nachgefragt wie bei `send`.

Es gibt kein `--kind`, kein `--priority`, kein `--disposition` und kein `--ref` an `reply`. Eine
Antwort erbt ihren Gegenstand: der Faden sagt bereits, worum es geht — deshalb sitzt die
`--ref`-Pflicht an `send` und sonst nirgends.

### `nxc withdraw --thread <FADEN>`

Eine Beauftragung zurücknehmen — eine, die noch **auf die Arbeitskopie wartet**, oder eine Runde,
die **läuft**.

**Es ist das Verb eines Menschen.** Nur wer die erste Beauftragung des Vorgangs abgeschickt hat,
darf sie zurücknehmen, und nur, indem er diesen Faden nennt — den, den sein erstes `send` ihm
zurückgegeben hat. Alles andere wird abgelehnt, bevor sich etwas ändert, und zwar benannt:

- **Eine Sitzung, die dieser Workspace gestartet hat** — das eigene `nxc` eines Agenten — wird
  abgelehnt, gleich was sie nennt, und die Ablehnung nennt die Sitzung, die Rolle, für die sie
  gestartet wurde, und den Weg, den sie stattdessen hat: `nxc reply --thread <ihr eigener Faden>
  --escalate -`, das nach oben geht, zu dem, der sie beauftragt hat. Das ist, was die Engine an
  ihren eigenen Sitzungsaufzeichnungen erkennen kann, und mehr behauptet sie nicht: Ein Prozess,
  der seine Sitzung verbirgt, wird hier nicht erkannt, und die Regel danach prüft die Identität,
  die ein Aufrufer angibt — sie ist also keine zweite Sperre. Was dann bleibt, ist, dass die Arbeit
  geparkt statt verloren wird und die Handlung aufgezeichnet ist.
- **Wer den Vorgang nicht eröffnet hat**, erfährt, wer es war (`… did not open operation <id> —
  local/carsten did, with the nxc send that began it`).
- **Ein Faden unterhalb des ersten** — ein Schritt einer Kanalrunde, eine Unterrunde, die ein Agent
  eröffnet hat — wird abgelehnt, und der erste Faden des Vorgangs wird stattdessen genannt
  (`nxc withdraw --thread <wurzel>`).

Bei Erfolg druckt es eine Zeile je zurückgenommener Beauftragung (`withdrew <Rolle> on thread <id>
(never started)`), eine je beendeter Sitzung (`stopped <Rolle> on thread <id> (session <id> was
asked to end)`), eine, die sagt, was aus der Arbeit einer beendeten Runde wird, und eine, die sagt,
dass die Kette unterbrochen ist; `--json` trägt dasselbe als `withdrawn` / `started_meanwhile` /
`stopped` / `will_park`, dazu `warnings`, wenn etwas nicht angekommen ist.

**Eine Beauftragung, die nie startete.** Eine Persona oder ein Kanal mit `working_tree: exclusive`
lässt eine Kette zur Zeit laufen; eine Beauftragung, die eintrifft, während eine andere Kette hält,
wird **geparkt** und startet, sobald die Halterin loslässt. Bis dahin ist nichts geschehen: keine
Sitzung, kein Verlauf, kein Modellaufruf. Sie zurückzunehmen verliert nichts.

**Eine Runde, die läuft.** Ihr Faden wird entlastet und ihre Sitzung gebeten anzuhalten — eine
Anforderung, die dem Prozess zugestellt wird, der dann auf seine Weise endet (der mitgelieferte
Sidecar bricht den Zug ab, sichert sein Transkript und meldet sein Ende). Hält die Runde die
Arbeitskopie dieses Workspace und kann ein Parken hier auch einen Zweig erzeugen, sagt die Quittung
`will_park: true`: Was die Sitzung uncommittet
hinterlässt, wird vom Hintergrunddienst **auf einen Zweig geparkt**, sobald in diesem Vorgang nichts
mehr läuft — versionierte wie nicht versionierte Dateien, ignorierte bleiben, wo sie sind —, und die
Arbeitskopie geht an den weiter, der wartet. Nichts wird je zurückgerollt. `nxc status` führt den
Zweig unter dem Vorgang (`parked on <zweig> at <commit> since <zeitpunkt>`), bis jemand ihn wieder
abholt, und der Weg zurück ist **die nächste Beauftragung in denselben Faden**: auf einem direkten
Persona-Faden setzt `nxc reply --thread <id>` die angehaltene Sitzung mit ihrem eigenen Transkript
fort; bei einer Kanalrunde startet eine Folgenachricht in den Kanalfaden den nächsten Durchgang. So
oder so wird der Baum auf den Parkzweig zurückgesetzt, und die Sitzung erfährt in einem festen Text,
auf welchem Zweig und welchem Commit ihre Arbeit liegt. Ein frisches `send` eröffnet einen neuen
Anspruch und findet sie nicht. Eine Runde, die keine Arbeitskopie hält — eine geteilte Persona —,
wird angehalten, und `will_park` ist `false`.

**Und wenn die Arbeit zwar in der Arbeitskopie liegt, aber überhaupt nicht geparkt werden kann**,
sagt die Quittung `will_park: false` mit `cannot_park` und dem Grund: Der Chat-Worker dieses Hosts
führt Sitzungen in keinem benennbaren Verzeichnis aus, das Verzeichnis ist kein Git-Repository, oder
der Vorgang hat nie eine Basis vermerkt. Die Arbeitskopie wird trotzdem weitergegeben, sobald in
diesem Vorgang nichts mehr läuft — ungeparkt, mit der Warnung `work_handed_on_unparked`, und die
Arbeit bleibt im Baum. Auch dort wird nichts zurückgerollt. `nxc withdraw` gibt diesen Fall in
eigenen Worten aus, und beide Park-Zeilen nennen `nxc tick --thread <id>`, das das Parken
beziehungsweise die ungeparkte Weitergabe von Hand erledigt, wo kein Hintergrunddienst läuft — und
sagt mit der Warnung `service_not_running`, wenn keiner läuft.

Ein Host, dessen Chat-Worker **keine Sitzung anhalten kann**, lehnt den ganzen Aufruf ab, bevor sich
etwas ändert, und zwar benannt (`this host cannot stop a running session; nothing was withdrawn`).
Ein Host, dessen Worker nicht einmal *feststellen* kann, ob eine Sitzung läuft, wird ebenfalls
benannt abgelehnt — unter Nennung der Sitzungen, für die er nicht antworten kann —, statt zu melden,
es gebe nichts zurückzunehmen: Das wäre eine Aussage über Prozesse, die niemand angesehen hat. Ein
Stopp, den der Worker beherrscht und dann nicht zustellen konnte — eine verschwundene PID-Datei, ein
Anspruch, der nicht belegen kann, welchen Prozess er meint, ein verweigertes Signal —, lässt den
Faden trotzdem entlastet und wird als Warnung `session_not_stopped` gemeldet, die die Sitzung nennt;
der Befehl endet dann mit einem Fehlercode, denn der Prozess läuft möglicherweise noch, und das ist
die eine Tatsache, die ein Skript erfahren muss.

Eine Sitzung, die beim Absenden des Stopps **bereits beendet** war, ist nicht dieser Fall. Eine
Runde, die ohnehin gerade fertig wurde, oder eine, deren Prozess längst weg ist und deren PID das
System inzwischen an etwas anderes vergeben hat, ist genau der Zustand, den Sie wollten: Die
Quittung sagt das in Worten, es gibt keine Warnung, und der Befehl endet mit 0. Es wird niemals an
einen Prozess signalisiert, von dem dieser Arbeitsbereich nicht belegen kann, dass er der Sitzung
gehört.

Eine Sitzung, die **fünf Minuten nach dem Stopp noch da ist**, bekommt vom Hintergrund-Tick ein
weiteres `SIGTERM` — einmal, mit derselben Identitätsprüfung, nie öfter und nie `SIGKILL` —, und
die Warnungen des Ticks nennen sie mit PID und PID-Datei (`withdrawn_holder_wedged`); `nxc status`
zeigt den Vorgang als `WITHDRAWN` mit den Sitzungen, die ihn noch halten. Danach ist es an Ihnen,
sie von Hand zu beenden. Wissen Sie, was dieses weitere Signal bewirken kann: Der mitgelieferte
Agenten-Sidecar hält beim Eintreffen schon an, vermerkt es und ändert nichts — das Signal hilft dem
eigenen Worker eines Hosts, dessen erster Stopp verloren ging oder ignoriert wurde, und beim Sidecar
ist die Warnung das, worauf es zu handeln gilt. Eine Sitzung, die Sie selbst wieder in Gang gesetzt haben — mit einer
Folgenachricht in den zurückgenommenen Faden vor dem Parken —, ist wieder Ihre Arbeit und wird
weder signalisiert noch genannt.

Alles unter dem genannten Faden wird zurückgezogen, und jeder Faden wird mit einer Nachricht
entlastet, die sagt, wer ihn zurückgenommen hat und was aus seiner Arbeit wurde — es ist also ein
Eintrag im Gespräch, keine Leerstelle.

**Ein Zurücknehmen unterbricht die Kette darunter.** Jeder Faden zwischen dem Zurückgenommenen und
dem genannten Faden, der noch wartet — ein Kanalfaden, der das Ergebnis seiner Runde schuldet, ein
Persona-Faden, der auf die von ihm beauftragte Runde wartet —, wird auf demselben Weg entlastet, mit
einer Nachricht, die sagt, dass die Kette unterbrochen wurde, und zählt in `nxc status` nicht mehr
als offen. Darunter geht also nichts weiter: Ein geordneter Kanal beauftragt weder den nächsten
Schritt noch den zurückgenommenen, ob er `flow: sequential` oder `steps:` deklariert, auch nicht
nach dem Parken; und nichts wird konsolidiert, denn es gibt keine Antwort auszuliefern. Die
Antworten, die Schritte schon gegeben hatten, bleiben in ihren Fäden. Die Arbeit kommt zurück, wenn
Sie denselben Faden erneut beauftragen, wie oben beschrieben.

Startet eine Beauftragung zwischen dem Lesen der Warteschlange und dem Schreiben, wird sie als
`started_meanwhile` gemeldet und strikt in Ruhe gelassen — der nächste Aufruf findet sie laufend.
Die Kette darüber ist trotzdem unterbrochen, sie läuft also für sich, und die Schlusszeile sagt
das: Sie nennt die Ausnahme und den Aufruf, der auch sie stoppt (`nxc withdraw --thread <id>` noch
einmal).

Ein Faden, unter dem nichts wartet und nichts läuft, ist ein `not_found`, das das sagt.

### `nxc resume --thread <THREAD>`

Nimm einen Vorgang wieder auf, der stehengeblieben ist, **weil das Modell nicht verfügbar war** —
das Kontingent erschöpft, der Anbieter nicht erreichbar, das Netz unterbrochen. Drei Ursachen, ein
Zustand, und es ist der Zustand, in dem nichts kaputt ist und niemand etwas falsch gemacht hat.

So eine Runde wird **nicht zurückgegeben**. Der Faden schuldet weiterhin seine Antwort und die
Sitzung wartet darauf, sie zu geben — `nxc status` markiert den Vorgang deshalb `INTERRUPTED` und
nicht `NEEDS DECISION`, die Zeile nennt die getroffene Grenze und wann sie fällt, und `nxc session
state` sagt dasselbe über die Sitzung. Dass die Tatsache überhaupt nachlesbar ist, ist die
größere Hälfte davon: vorher meldete eine Sitzung, die ins Wochenkontingent lief, ein sauberes
`ended` und ihr Faden `answered` — das Einzige, was niemand feststellen konnte, war, dass etwas
passiert war.

**Der Hintergrunddienst macht das von allein**, sobald die Grenze fällt; er ist die einzige Uhr im
System, und die Laufzeit nennt die Rücksetzzeit. Von Hand läuft der Befehl, um früher dranzugehen:
`--force` startet, obwohl das Fenster noch nicht offen ist — das ist der Fall mit einem zweiten
Konto. Hat die Laufzeit gar keine Zeit genannt, nimmt nichts den Vorgang von allein auf, und die
Zeile sagt das.

**Die Sitzung macht mit ihrer eigenen Mitschrift weiter**, statt von vorn anzufangen. Das ist keine
Bequemlichkeit, sondern die Sicherheitsbegründung: die Mitschrift trägt jeden Befehl *und dessen
Ergebnis*, also sieht eine Sitzung, die ein Ticket schon angelegt hat, die Kennung, die sie
zurückbekam — und legt es nicht noch einmal an. Ein Neustart aus dem Nichts tut genau das.
Werkzeugaufrufe werden nie wiederabgespielt.

**Vorher wird die Welt geprüft**, und das meiste, was der Befehl ausgibt, ist eine Entscheidung,
gerade nicht zu starten:

- die Runde war **schon beantwortet**, bevor die Unterbrechung kam — nichts fortzusetzen, der
  Haltezustand wird geschlossen. Das ist der Normalfall und keine Ecke: ein Kontingent trifft die
  teuren Züge, und ein teurer Zug ist meistens einer, der gerade fertig geworden ist;
- für die Sitzung **läuft schon ein Prozess** — eine Sitzung, ein Prozess, also bleibt sie in Ruhe.
  Das hält ein Fortsetzen von Hand und den Dienst zur Rücksetzzeit auseinander;
- die **Arbeitskopie ist gewandert**, während der Vorgang wartete — dann geht eine Frage zurück an
  den Faden, der den Vorgang gestartet hat, und es wird nichts gestartet. Das entscheidet die
  Maschine nicht;
- die **Laufzeit dieses Arbeitsbereichs kann ein begonnenes Gespräch nicht fortsetzen** — mit Namen
  abgelehnt, denn Fortsetzen hieße dort, still eine frische Sitzung ohne die Vorgeschichte zu
  öffnen.

Solange das Fenster zu ist, wird die unfertige Arbeit des Vorgangs **auf einen Park-Zweig
committet** und die Arbeitskopie weitergegeben: ein Wochenfenster dauert bis zu einer Woche, und die
einzige Arbeitskopie dieses Arbeitsbereichs so lange zu halten, brachte alles andere zum Stehen. Das
Fortsetzen holt sie zurück und sagt, ob die Basis sich darunter bewegt hat. Ein abgelehntes Parken
folgt der Regel aus [Grenzen und Sicherheit](nxc-limits-and-safety): ein Baum mitten in einem Merge
behält die Arbeitskopie, zeigt `PARK REFUSED` in `nxc status` und wird, solange jemand wartet, vom
nächsten Tick nach dem abgeschlossenen Merge geparkt; ein Arbeitsbereich, in dem gar nicht geparkt
werden kann, gibt die Arbeitskopie ungeparkt weiter.

Was in beide Richtungen außerhalb liegt: **ignorierte Dateien** — `.env`, lokale Datenbanken,
`node_modules` — sieht weder der Anker noch der Park. Und zurück spult das Repository, nicht die
Aufzeichnung: angelegte Tickets bleiben angelegt, gesendete Nachrichten bleiben gesendet.

`--session <ID>` benennt statt eines Fadens die unterbrochene Sitzung direkt. Ein Mensch benennt
einen Faden, weil `nxc status` ihm den hinlegt; der geplante Auftrag benennt die Sitzung, weil der
Haltezustand daran hängt.

## Lesen

Keine dieser Lesungen schreibt etwas, und keine verbraucht eine Nachricht. `--consumer <handle>`
liest als ein anderes qualifiziertes Handle; ohne das liest ein Befehl als der Aufrufer selbst.

> **Hier standen `nxc inbox` und `nxc read`, und beide sind weg** (nxf 6j6v.1gm9). Es gibt kein Verb
> mehr, mit dem man nach den eigenen Nachrichten fragt, und keines, mit dem man sie bestätigt, denn
> **Menschen ziehen, Agenten bekommen geschoben**: die Nachricht eines Agenten steht in dem Prompt,
> der seine Sitzung startet, oder in dem Zug, der sie wieder aufnimmt — ein Abrufverb war also die
> Doppelung dessen, was ohnehin schon zugestellt war, gemessen mit je 0 Verwendungen in 66
> Rollensitzungen. Ein Mensch liest stattdessen das GESPRÄCH, und dafür sind die beiden Verben
> darunter da. Der Ungelesen-DATENSATZ hat die beiden Verben um zwei Wochen überlebt — an der
> App-Naht als `in_turn`/`next_session`/`count` in `prime --json` — und ist dann ebenfalls entfallen
> (nxf 6j6v.4d2z): niemand hat ihn gelesen und niemand hat ihn bestätigt, der Lesezeiger darunter
> war also ein Schreibpfad ohne Zweck.

### `nxc threads list` / `nxc threads show <FADEN>`

Die Quorum-Tafeln. `list` sind alle Tafeln, in denen der Aufrufer Mitglied ist, je mit ihrem
Sammelzustand; `--channel` engt ein.

```console
$ NXC_ACTOR=alice nxc threads list
m-00000000000000000000000003  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000004  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1/1 in  [complete]
m-00000000000000000000000002  dm:9445dbc93dfdad401585c42b  0/1 in  [waiting]  · holding working tree

```

`show` ist eine Tafel in voller Länge — der Quorum-Zustand plus die Antworten in ihrer Reihenfolge:

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

Die `--json`-Datensätze tragen `thread_id`, `name`, `channel_id`, `opener`, `expects`, `replied`,
`outstanding`, `complete`, `stale`, `working_tree` und `working_tree_queue_position`. `name` ist,
wie das Gespräch HEISST (siehe `nxc threads name` unten), und fehlt bei einem Faden, den niemand
benannt hat. Viele Tafeln zu
lesen kostet dieselbe Zahl an Abfragen wie zwei zu lesen — diese Eigenschaft wird von einem Test
gemessen, nicht bloß beabsichtigt, weil eine Koordinationsoberfläche alle auf einmal liest.

### `nxc threads name <FADEN> [<NAME>]`

Wie ein Gespräch **heißt**. Die Id einer Direktunterhaltung ist ein Hash über die beiden Handles —
so abgeleitet, damit beide Seiten dieselbe Id erreichen, ohne sich vorher abzustimmen —, und ein
Hash ist nichts, was ein Mensch lesen kann. Deshalb gibt `nxc send` dem Faden, den es öffnet, einen
kurzen Anzeigenamen von höchstens sieben Wörtern, abgeleitet aus der gesendeten Nachricht. Der Name
reist auf jeder Faden-Lesenaht oben mit, damit eine App zeigen kann, *worum es geht*, statt
`dm:d9ab4f601a08…`.

Diese Ableitung läuft außerhalb, in einem Lauf, den der Koordinator beauftragt — sie hält ein `send`
nie auf und lässt es nie scheitern, und ein Faden, den niemand benennen konnte, bleibt ein
gewöhnlicher Faden, den jede Oberfläche genau wie bisher darstellt. Dieses Verb ist derselbe Vorgang
von Hand: mit `<NAME>` setzt es einen, ohne leitet es einen aus der Eröffnungsnachricht ab.

**Ein Faden wird einmal benannt.** Ein Faden, der schon einen Namen hat, behält ihn, und die Quittung
sagt `named: false` — das ist ein No-op, kein Fehler, denn ein Name, der unter dem Leser wegwandert,
ist schlimmer als gar keiner. Er ist ein Anzeigename, und nichts keyt darauf; die Id bleibt die Id.

Das Ableiten liest die Eröffnungsnachricht des Fadens, deshalb gilt für dieses Verb dieselbe
Mitgliedschaftsregel wie für `nxc threads show`: ein Gespräch, das Sie nicht lesen dürfen, dürfen Sie
auch nicht benennen lassen.

### `nxc machine --to <PERSONA> | --thread <FADEN> [--machine <M>]`

Auf welcher Maschine ein Chat läuft — ein neuer mit einer Persona oder einer, den es gibt — und ob
Senden jetzt stattdessen nachfragen würde: die Maschine, woher sie kommt (`choice`, `chat`, `persona`
oder `started_here`), ob sie online ist, und welche Maschinen es sind. `--machine` zeigt, was eine
Wahl bewirken würde, die Sie gerade treffen wollen. Es schreibt nichts; `send` und `reply` treffen
genau diese Entscheidung, bevor sie schreiben, und deshalb kann eine App die Wahl zuerst anzeigen.

### `nxc status`

Wo ein **Vorgang** steht: der ganze Fadenbaum von seiner Wurzel abwärts, über Kanalgrenzen hinweg.
Eine Tafel erzählt Ihnen von einem Gespräch; `status` erzählt vom ganzen Stück Arbeit, das es
angestoßen hat.

**Es beantwortet eine Frage: läuft hier noch etwas?** Ein abgeschlossener Vorgang steht deshalb
nicht darin. Die Runde oben ist beantwortet, und die schlichte Auflistung sagt das, indem sie
schweigt:

```console
$ NXC_ACTOR=alice nxc status
nothing open (`nxc status --all` also shows finished operations)

```

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

```console
$ NXC_ACTOR=alice nxc status --all --json
{"operations":[{"root":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","live":false,"open":0,"needs_decision":false,"holds_working_tree":false,"interrupted":false,"threads":[{"thread_id":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","depth":0,"state":"answered","awaiting_human":true,"escalated":false,"substituted":false,"opener":"ab12/alice","expects":["ab12/coder"],"stale":false}]}],"worker_answers_liveness":false,"worker_names_a_working_copy":false}

```

- **`--thread <id>`** — der eine Vorgang, zu dem dieser Faden gehört, von *dessen* Wurzel aus
  gezeigt, ob er noch läuft oder schon fertig ist.
- **`--channel <name>`** — die lebenden Vorgänge, deren Wurzel in diesem Kanal sitzt; ein
  deklarierter Kanalname oder eine rohe Kanal-ID. Ein Einstieg, kein Anker: ein Vorgang
  überschreitet Kanäle, und der Baum folgt ihm.
- **`--all`** — auch die abgeschlossenen, mit `· finished` markiert. Es nimmt `--channel` mit, also
  ist `--all --channel review` die Geschichte dieses Kanals.
- **keines davon** — alles, was hier noch läuft.

**Was „noch am Laufen" genau heißt**: Der Vorgang hat einen offenen Faden, oder eine unbeantwortete
Rückgabe, oder die Arbeitskopie dieses Geräts, oder eine *gescheiterte* Sackgasse — einen Faden,
dessen Antwort ankam und dessen Folge nie kam (`ORPHANED`, an der Vorgangszeile mit `· dead end`
markiert). Zwei weitere halten ihn in der Liste, wenn sich alles andere an ihm erledigt hat:
**Arbeit, die auf einem Zweig geparkt ist und die niemand zurückgeholt hat** (eine Zeile
`parked on <Zweig> at <Commit> since <Zeitpunkt>` unter dem Vorgang, `parked` in `--json`), und
**ein abgelehntes Parken, das wiederholt wird** (die Markierung `· PARK REFUSED`, eine Zeile
`park refused (<Ablehnung>): <was zu beheben ist> — retrying since <Zeitpunkt>`, `park_refused` in
`--json`). Das sind die Dinge, an denen ein Leser etwas tun kann, und zusammen sind sie das
Kennzeichen `live` an jedem ausgegebenen Vorgang. Beachten Sie, was *nicht*
auf dieser Liste steht: eine Wurzel, die beantwortet und noch nicht gelesen ist. Das ist
`awaiting_human`, und es hielt einen Vorgang früher in der Liste — womit nie etwas die Ansicht
verließ, denn das Kennzeichen leitet sich aus Tatsachen ab, die nie aufhören zu gelten (die Wurzel
hat gefragt; die Wurzel wurde beantwortet). Nach einem Tag echter Nutzung zeigte die Ansicht dreizehn
Vorgänge und 424 Zeilen, neun davon ohne einen einzigen offenen Faden. `awaiting_human` wird
weiterhin berichtet — in `--thread` und `--all` —, wo es sagt, was es immer sagte: dieser hier ist
fertig, und er gehört Ihnen.

**Die Zeilen werden genauso beschnitten.** In den beiden Listenformen gibt das Terminal nur die Fäden
aus, die sagen, *warum* ihr Vorgang noch dasteht — die Wurzel, alles, was nicht `answered` ist, eine
Rückgabe, ein Faden, der die Arbeitskopie hält oder auf sie wartet —, und zählt die abgeschlossene
Mitte der Kette in einer Zeile. Das ist es, was die Ansicht auf einen Bildschirm passen lässt: In dem
Arbeitsbereich, an dem das gemessen wurde, waren sechs lebende Vorgänge 328 Zeilen, wenn jede Zeile
gedruckt wird, und 32, wenn der Rest gezählt wird — denn vier davon lebten von je einem
zurückgegebenen Faden und trugen hundert beantwortete hinter sich her. `--thread` und `--all` geben
den ganzen Baum aus, und `--json` wird in keiner Form beschnitten.

Jeder Faden trägt einen `state` aus `open` / `answered` / `stale`, dazu `awaiting_human`, das nur an
der Wurzel eines abgeschlossenen Vorgangs `true` ist. Genau darum gibt es das Feld: eine
stehengebliebene Kette und eine fertige, die auf einen Menschen wartet, sehen für einen Zähler gleich
aus und bedeuten Gegenteiliges.

**Und jeder Faden sagt, was aus der Sitzung wurde, die daran arbeitet.** `session` nennt die interne
Sitzung dessen, der die Antwort schuldet; `session_state` sagt, ob sie `running`, `ended` oder
`unknown` ist — das ist der Unterschied zwischen einem Faden, der nachdenkt, und einem, der hängt,
ohne einen zweiten Aufruf:

```text
m-01M0…  #coding  waiting on ab12/coder — ITS SESSION HAS ENDED, nothing is coming
```

Im Terminal steht das nur an einem **offenen** Faden, denn dort ist es ein Befund; `--json` trägt es
überall dort, wo es eine Sitzung gibt. `unknown` heißt, dass niemand antworten konnte, und der
Bericht sagt im selben Atemzug, ob das daran liegt, dass die Sitzung wortlos starb, oder daran, dass
diese Laufzeitumgebung gar nicht gefragt werden kann: `worker_answers_liveness`. Im Beispiel oben
steht `false` — die durchgespielte Sitzung dieser Anleitung protokolliert ihre Auslöser, statt echte
Sitzungen zu starten, es gibt also nirgends einen Prozess zu befragen — und unter dem ausgelieferten
Sidecar steht `true`.

**Und ein Faden sagt, wenn die Partei, auf die er wartet, selbst wartet.** `waiting_on_sub_round`
listet die Runden, die diese Partei aus diesem Faden heraus beauftragt und noch nicht zurück hat; im
Terminal steht

```text
m-01M0…  #positioning  waiting on 47jy/head-of-marketing — waiting on its own sub-round (4 open)
```

Das ist der Unterschied zwischen einer Kette, die steht, und vier Sitzungen, die arbeiten. Erklärt
wird dabei nichts — die Engine hat diese Fäden im Auftrag jener Partei geöffnet, es ist also eine
Tatsache über den Datensatz — und die Abwesenheit sagt so viel wie die Anwesenheit: ein offener
Faden ohne diese Liste ist einer, aus dem nichts weiter herausgegeben wurde. Der Schlüssel fehlt
ganz, wenn es nichts zu sagen gibt.

Zwei Kennzeichen sitzen am **Vorgang** statt an einem Faden, und beide beantworten eine Frage, die
die Faden-Zeilen sonst nur einzeln nacheinander beantworten:

- **`needs_decision`** — irgendwo unter dieser Wurzel wurde eine Aufgabe zurückgegeben, und niemand
  hat sie aufgenommen. Lesen Sie es neben `awaiting_human`: beide sagen „der Mensch ist dran", über
  Lagen von völlig verschiedener Dringlichkeit. Ein fertiger Vorgang, der gelesen werden will, ist
  das normale Ende; einer, dessen Wurzel genauso aussieht, während darunter eine unbeantwortete
  Eskalation liegt, ist eine Kette, die *steht* — und eine Eskalation hält die Arbeitskopie, hält
  also auch die Maschine an. Es räumt sich selbst ab: Runde neu beauftragen, Kennzeichen weg.
- **`holds_working_tree`** — irgendwo unter dieser Wurzel wird die Arbeitskopie gehalten. Es sagt
  Ihnen, ob Sie überhaupt nachsehen müssen; das Feld `working_tree` am Faden sagt weiterhin, *welcher*
  Faden.

### `nxc search <SUCHTEXT>`

Groß-/kleinschreibungsunabhängige Teilzeichenkette über die Nachrichtentexte in den Kanälen des
Aufrufers, deterministisch sortiert.

```console
$ NXC_ACTOR=alice nxc search "since flag" --json
[{"message_id":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","sender":"ab12/alice","body":"Add a --since flag to the export command."}]

```

Gesucht wird nur in Texten, und nur in Kanälen, in denen der Konsument Mitglied ist. Bei einem
**deklarierten** Kanal ist das die `members:`-Liste selbst, in beiden Richtungen und ohne ein `send`
dazwischen: in die Datei geschrieben, finden Sie seine Runden sofort; aus ihr gestrichen, finden Sie
sie nicht mehr. Es ist dieselbe Antwort, die `nxc threads show` gibt — eine Regel für die
Mitgliedschaft, welche Lesung Sie auch fragen.

Die Mitgliedschaft ist nicht das ganze Tor: Ein Kanal mit `visibility: requester_only` behält die
Antwort jedes Mitglieds dem vor, der die Runde angefragt hat, und die Suche hält sich an diese
Deklaration genauso wie `nxc threads show` — Sie finden die Anfrage und Ihre eigenen Worte, nie die
Antwort eines anderen Mitglieds.

**Und die Mitgliedschaft ist der ganze Suchraum — die eine Stelle, an der diese Lesung absichtlich
enger ist als `nxc threads show`.** Ein `public`-Kanal, dem Sie nie beigetreten sind, wird nicht
durchsucht: ihn zu finden ist Sache von `nxc list`, und zu lesen, was hinter der Tür liegt, ist eine
eigene Lesung. Ebenso folgt diese Suche einem *Vorgang* nicht über Kanalgrenzen, wie `nxc status` und
`nxc threads show` es tun: Wer einen Vorgang eröffnet hat, liest jeden Faden darin, die Textsuche
bleibt aber in den Kanälen, in denen dieser Eröffner Mitglied ist. Um zu finden, *was eine Sitzung
getan hat* statt was sie gesagt hat, lesen Sie ihren Verlauf.

## Sitzungen und Verläufe

Das sind die Innereien der Rollen-Laufzeit. Geschrieben werden sie vom Agenten-Sidecar; eine davon
ist eine Lesung, die Sie beim Namen kennen wollen.

### `nxc session bind <INTERN> <ECHT>`

Bindet eine interne (von nxc geprägte) Sitzungs-ID an die echte Sitzungs-ID, die das Claude Agent SDK
zurückgegeben hat. Das Sidecar ruft das auf, sobald eine Sitzung startet. Eine unbekannte interne ID
wird als `not_found` gemeldet.

### `nxc session state <SITZUNG>` / `--thread <ID>`

**Läuft diese Sitzung noch, oder ist sie tot?** Die Lese-Seite zu den beiden Schreibvorgängen, die
das Sidecar auf dieser Naht macht — `session bind` oben und das `session ended`, das es als letztes
aufruft.

```bash
nxc session state m-01M0…            # eine Sitzung
nxc session state --thread m-01M0…   # alle Sitzungen eines Fadens, beendete eingeschlossen
```

Drei Antworten, und die dritte ist kein Ausweichen:

- **`running`** — hinter ihr steht ein lebender Prozess.
- **`ended <Zeitpunkt>`** — die Sitzung hat ihr Ende selbst gemeldet, und zwar dann. Das ist die
  Tatsache, auf die ein Kanal mit `working_tree: exclusive` seinen nächsten Schritt öffnet.
- **`unknown`** — niemand hat ein Ende gemeldet, und kein lebender Prozess antwortet für sie: eine
  hart getötete Sitzung, oder eine, die diese Maschine nie ausgeführt hat. `ended` zu melden hieße,
  eine Tatsache zu behaupten, die niemand festgestellt hat.

`unknown` hat eine dritte Lesart, und die Antwort sagt Ihnen, wann Sie sie vor sich haben. Ob eine
Sitzung läuft, beantwortet der *Worker* — und nicht jeder kann das: wer keinen Prozess startet, hat
auch keinen anzusehen. Deshalb trägt die Antwort `worker_answers_liveness`: `false` heißt, hier
wurde nie gefragt, und kein `unknown` darunter sagt etwas über eine Sitzung aus. Der ausgelieferte
Sidecar beantwortet die Frage, auf der normalen Kommandozeile steht das Feld also auf `true` und ein
`unknown` ist wirklich eine Sitzung, die wortlos gestorben ist.

Die Sitzungsform an einem still gewordenen Faden, bevor Sie ihn für gescheitert halten. Die Form
`--thread`, bevor Sie eine Arbeitskopie anfassen, an der ein vorheriger Schritt noch die Hände haben
könnte: sie listet **jede** Sitzung, die dort lief — beendete eingeschlossen — und was aus jeder
wurde. `nxc status` beantwortet die engere Frage, nämlich den Zustand der EINEN Sitzung, die es je
Faden nennt; greifen Sie also hierher, wenn Sie die ganze Geschichte eines Fadens brauchen und nicht
nur, wer gerade daran sitzt. Eine unbekannte Sitzung ist `not_found`; ein Faden, auf dem nichts lief,
ist eine leere Antwort und kein Fehler.

### `nxc transcript show <SITZUNG>`

Der normalisierte Strom einer Sitzung als Zeitleiste: Assistenztext, Denken, Werkzeugaufrufe und
deren Ergebnisse, wobei die Einträge eines per `Task` gestarteten Unteragenten unter dem `tool_use`
verschachtelt sind, der ihn gestartet hat. Die Sitzungs-ID ist die interne — das, was `send --to
<persona>` als `session` zurückgibt.

```console
$ NXC_ACTOR=alice nxc transcript show m-00000000000000000000000001
transcript m-00000000000000000000000001  role=coder  (0 entries)

```

`--from-seq <n>` und `--limit <n>` blättern eine lange Sitzung: übergeben Sie das größte `seq`, das
der letzte Abschnitt gezeigt hat; ein Abschnitt kürzer als `--limit` ist das Ende. Eine unbekannte
Sitzung liefert einen **leeren** Verlauf statt eines Fehlers — eine Sitzung, deren Sidecar nie
geschrieben hat, ist von einer, die nichts zu sagen hatte, nicht zu unterscheiden.

Zwei Dinge sollten Sie wissen, bevor Sie so etwas irgendwohin einfügen. Ein Verlauf enthält **rohe
Werkzeug-Eingaben und -Ergebnisse** — alles, was der Agent gelesen, geschrieben oder ausgeführt hat.
Behandeln Sie einen Auszug also wie die Workspace-Datenbank, nicht wie ein Nachrichtenprotokoll. Und
er ist bewusst nicht mitgliedschaftsgeprüft: ein Verlauf hat keinen Kanal, an dem man prüfen könnte,
und die Tabelle ist gerätelokal und wird nie synchronisiert — eine Prüfung erkaufte also nichts, was
wer die Datei hat nicht ohnehin mit `sqlite3` täte.

### `nxc transcript append --session <ID>`

Hängt normalisierte Einträge an, als JSON-Zeilen von STDIN gelesen. Das ist der Rückruf-Kontrakt des
Sidecars — der Flag-Name, die stdin-Rahmung und der `--json`-Datensatz sind alle tragend, weil ein
bereits ausgelieferter Erzeuger davon abhängt. Eine Zeile, die sich nicht parsen lässt, ist ein
lauter `validation`-Fehler, der die Zeile benennt, nie ein stilles Überspringen.

### `nxc transcript prune`

Räumt die Verläufe von Sitzungen ab, in die seit einer Weile niemand geschrieben hat, und sagt, was
weg ist. Ganze Sitzungen, gealtert an ihrem *letzten* Eintrag: eine Sitzung, in die noch geschrieben
wird, ist nie ein Kandidat, wie lange sie auch schon läuft, und eine lange wird nie halbiert.

```bash
nxc transcript prune --dry-run          # melden, nichts entfernen, keine Schreibsperre nehmen
nxc transcript prune --keep-days 7      # enger als das konfigurierte Fenster
```

Sie müssen das nicht ausführen, um die Tabelle zu begrenzen — dieselbe Aufbewahrung läuft beim ersten
Schreiben jeder neuen Rollensitzung mit. Greifen Sie danach, um einen still gewordenen Workspace
aufzuräumen oder einmalig ein engeres Fenster anzuwenden. Es stoppt das Wachsen der Datenbank; es
macht die Datei nicht kleiner (dafür bräuchte es ein `VACUUM`, das hier nicht läuft, weil es den
ganzen gemeinsamen Workspace unter exklusiver Sperre neu schreibt). Sitzungen ohne lesbaren
Zeitstempel sind von unbekanntem Alter, werden also **behalten** und gesondert gemeldet — eine
sichtbare Lücke statt einer stillen.

## Dokumentation

### `nxc guide [THEMA]`

Die Anleitungen, die Sie gerade lesen, ins Binary kompiliert. Kein Workspace nötig, kein Netz:

```bash
nxc guide                     # die Themen von chat auflisten
nxc guide core-concepts       # eines ausgeben
nxc guide --json              # [{topic, summary}, …] — der Agenten-Kontrakt
```

`nxs guide` fächert über die aktiven Module auf und listet die Themen aller drei Bausteine auf
einmal. Wo ein Themenname in mehr als einem Baustein vorkommt — `getting-started` und `commands` tun
das —, wählt es nicht für Sie aus, sondern nennt die Befehle je Werkzeug.

## Keine Verben, die man tippt

Drei Unterbefehle existieren und sind aus `--help` ausgeblendet, weil niemand sie tippen soll:

- **`nxc prime`** — der Sitzungsstart. Der eigene `SessionStart`-Hook von chat führt es direkt aus,
  und `nxs prime` fächert dorthin auf, wenn Sie die Klammer von Hand fragen.
- **`nxc tick --thread <id>`** — ein Uhrzeiger. Der Einmal-Job, den das deklarierte `timeout:` eines
  Kanals plant, führt ihn aus, um einen Faden erneut zu prüfen und ihn, wenn fällig, durch die
  `on_complete`-Politik des Kanals zu leiten. Er ist idempotent: ein bereits behandelter Faden ist
  ein sauberer No-Op, nie ein zweites Wecken.
- **`nxc pick-up`** — die Hand der ausführenden Maschine. Der Hintergrunddienst führt es nach einem
  Durchgang aus, der einen Chat für DIESE Maschine hinterlassen hat: Es startet oder weckt die
  Persona jedes hier bestimmten Chats, der einen Zug schuldet, einmal je Nachricht, und nur für
  Nachrichten, deren Herkunft diese Maschine vertraut. Es ist idempotent.

Sie stehen hier, damit es kein Rätsel ist, wenn man sie in einer Prozessliste oder einem Protokoll
findet. Bauen Sie nichts darauf.

## Was bewusst fehlt

Die Hälfte einer vertrauenswürdigen Referenz ist das, wovon sie sagt, dass es *nicht* da ist. Diese
Verben gab es, und sie wurden entfernt; jeder Eintrag sagt, was man stattdessen tut.

- **`nxc ask`** — in `send --to` aufgegangen. Ein Kanal wird genau wie eine Persona angesprochen, und
  ein Verb, das einen Faden prägt, schlägt zwei, die sich uneins sind, ob sie das tun.
- **`nxc channels create` / `dm` / `join` / `leave`** — mit den rohen Kanälen entfallen. Ein Kanal
  ist eine **Deklaration** in `.nxs-personas/channels.yaml`; die Mitgliedschaft ist seine
  `members:`-Liste. Ein direktes Gespräch wird für Sie geprägt, sobald Sie `send --to` an eine
  Persona richten, unter einer aus den beiden Handles abgeleiteten ID — es gibt also nichts
  anzulegen. Die projektübergreifende Entdeckung öffentlicher Kanäle hat die Agentenoberfläche ganz
  verlassen und ist Sache einer App; `nxc list --json` trägt die Vordertüren weiterhin, die es sieht.
- **`nxc agents register` / `list` / `search` / `show`** — ein Team wird deklariert, nicht
  registriert. `nxc list` ist die Lesung, und sie durchsucht dieselben Felder `job_title` /
  `job_description`, die `agents search` durchsuchte. Eine zur Laufzeit registrierte Profilzeile lag
  in der Datenbank einer Maschine, niemand prüfte sie, und sie überlebte den Lauf nicht.
- **`nxc workflow start` / `step` / `status` / `tick` / `done` / `bind` / `append` / `show` /
  `expect`** — die deklarative Lauf-Engine ist weg, samt Lauf-Datensatz. Ein Kanal deklariert seinen
  eigenen Ablauf (`flow: sequential`), `send --to <kanal>` startet ihn, und `nxc status` ist der Ort,
  an dem die Position eines Vorgangs gelesen wird. Siehe [Kanäle](nxc-channels).
- **`send --role` / `--session`** — `--to` ist der eine Weg, ein Ziel zu benennen. `--role` ist darin
  aufgegangen; `--session` ist eine festgehaltene Lücke und kein Zusammenfallen, und im Quelltext
  auch als solche benannt.
- **`send --kind` / `--priority` / `--disposition` / `--model` / `--deadline` sowie `reply --kind`** —
  gemeinsam entfernt. Die ersten drei ließen einen Aufrufer eine Frage beantworten, die ihm niemand
  gestellt hatte; die letzten beiden sind an der Persona und am Kanal deklariert, und eine Übersteuerung
  je Aufruf neben einem deklarierten Wert sind zwei Antworten auf eine Frage. Wie viel Denken eine
  Aufgabe wert ist, sagt das `stage:` oder `model:` der Persona; wie lange eine Tafel warten darf,
  das `timeout:` des Kanals.
- **`nxc release --thread <FADEN>`** — auf der Agentenoberfläche angeboten, konnte er die Arbeitskopie
  einer laufenden Coding-Operation wegnehmen, und bei einem Worker, der seine Lebendigkeitsprüfung nie
  implementierte, war er ein bedingungsloses Freigeben ohne jede Absicherung. Jede Übergabe parkt
  jetzt zuerst die Arbeit des Halters: Eine Arbeitskopie, die eine tote Kette hält, parkt und gibt der
  Hintergrunddienst von selbst weiter — über ihrer Zwei-Stunden-Grenze, oder schon davor, sobald die
  Kette nachweislich tot ist —, und eine Arbeitskopie, die eine noch laufende Kette hält, nimmt der
  Mensch, der sie gestartet hat, mit `withdraw` zurück, das ihre Sitzung stoppt und parkt, was sie
  unfertig hinterlässt; ein Agent eskaliert stattdessen. Siehe
  [Grenzen und Sicherungen](nxc-limits-and-safety).

## Weiter

- [Kernkonzepte](nxc-core-concepts) — was ein Faden, ein Kanal und ein Handle wirklich sind.
- [Kanäle](nxc-channels) — die Deklaration, die diese Verben ansprechen.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — die Kappen und Tore rund um all das.
