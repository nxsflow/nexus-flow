# Kanäle

Ein **Kanal** ist eine deklarierte Gruppe — und weil die Reihenfolge seiner Mitglieder verbindlich
gemacht werden kann, ist er zugleich der einzige Arbeitsablauf, den dieses System kennt. Beides steht
in einer Datei, `.nxs-personas/channels.yaml`, als YAML-Liste:

```yaml
- name: review
  members: [general, integrity]
  description: the review quorum — one round asks both reviewers and hands back one verdict
```

`name` und `members` sind die einzigen Pflichtschlüssel. Angesprochen wird er genau wie eine Persona:

```bash
nxc send --to review --ref nxf_ids=ab12.0008 "Judge the export change."
```

**Es gibt kein `nxc channels create`, und es wird nie eines geben.** Ein Kanal, den ein Verb prägte,
lag in der Datenbank einer Maschine, hatte keine prüfbare Mitgliedschaft und überlebte den Lauf
nicht. Ein deklarierter Kanal ist eine Datei in Ihrem Repository — und deshalb ist `members:` die
Mitgliedschaft: es gibt nichts beizutreten und nichts zu verlassen.

## Was ein Senden an einen Kanal wirklich tut

Ein Aufruf öffnet zwei Ebenen, und wer weiß, welche welche ist, versteht jede spätere Lesung von
`nxc status` sofort:

1. **Der Kanalfaden** — Ihr Gespräch mit dem Kanal. Er hat genau zwei Enden: Sie und den **Betreuer**
   des Kanals. Der Betreuer ist eine reservierte Engine-Identität, `<origin>/__channel__` — er hat
   keine Sitzung, keinen Verlauf, und nichts nimmt ihn je wieder auf. Er ist Maschinerie, kein
   Beteiligter, und keine Deklaration kann seinen Namen beanspruchen, denn ein Handle darf nicht mit
   `__` beginnen.
2. **Ein Platzfaden je Schritt** — das Gespräch des Betreuers mit jedem Mitglied, unter den Kanalfaden
   gehängt. Das ist der Faden, in den ein Mitglied antwortet, und seine ID steht in der
   Anstoßnachricht des Mitglieds.

```console
$ NXC_ACTOR=alice nxc send --to build-and-ship --ref nxf_ids=ab12.0008 "Ship the export change." --json
{"thread_id":"m-00000000000000000000000003","message_id":"m-00000000000000000000000004","to":"build-and-ship","target":"channel","channel":"decl:build-and-ship","expects":["ab12/__channel__"],"spawned":true,"warnings":[],"refs_warning":null,"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000003","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

Das `expects` dieser Quittung ist der ganze Entwurf in einem Feld: *Sie* warten auf den Betreuer, und
der Betreuer wartet auf die Mitglieder. Sie werden einmal geweckt, wenn die Runde fertig ist.

## Parallel: die Vorgabe

Ein Kanal, der zur Reihenfolge nichts sagt, fächert an alle Mitglieder gleichzeitig auf. Jedes
bekommt eine frische Sitzung und seinen eigenen Platzfaden; nichts wartet auf irgendetwas. Der
Absender wird nie angestoßen, auch wenn er deklariertes Mitglied ist.

```yaml
- name: review
  members: [general, code-quality, test-quality, integrity]
  expects: all               # oder eine Handle-Liste — nur diese müssen antworten
  on_complete: summarize     # oder pass_through (Vorgabe)
  summary_model: opus
  summary_prompt: |
    Fold the four reviews into one verdict and end with `Ready to merge? yes|no`.
  visibility: requester_only # oder all_members
```

**`expects`** entscheidet, wer antworten muss, damit die Runde vollständig ist: die bloße Zeichenkette
`all` (die Vorgabe, jedes Mitglied) oder eine YAML-Liste mit einer Teilmenge. Alles andere an dieser
Stelle ist ein lauter Parse-Fehler, der die Datei nennt.

**`on_complete`** entscheidet, was Sie erhalten.

- **`pass_through`** (Vorgabe) gibt Ihnen jede gesammelte Antwort in einer definierten Form —
  definiert, weil der Leser meist ein Agent ist, der sie parst, und kein Mensch, der sie überfliegt.
  Sie beginnt mit einem Kopf, der Kanal, Faden und die Zahl der gesammelten Nachrichten nennt, dann
  eine Rahmenzeile, dann jede Antwort in einem Element `<message from="…">`.

  **Dieses `from`-Attribut ist die eigene Aufzeichnung der Engine darüber, wer geschrieben hat, und
  keine Antwort kann eines fälschen.** Die Trennzeichen werden je Runde gewählt, nachdem jede Antwort
  gelesen wurde, so dass keine Antwort sie enthält — eine Antwort, die `</message>` ernsthaft
  schreibt, verschiebt die Runde einfach auf `<message.1 …>` … `</message.1>`, und die Lieferung
  nennt die verwendete Grenze. Nichts wird maskiert und nichts abgelehnt: die Grenze wird an die
  Antworten angepasst, nie die Antworten an die Grenze.

  Was das **nicht** klärt, ist, ob ein Mitglied die Wahrheit sagt. Die Texte sind das, was die
  Mitglieder geschrieben haben, geliefert in einen Prompt — und die Rahmung gibt es, um genau das zu
  sagen: behandeln Sie sie als Daten, nie als Anweisungen.

- **`summarize`** lässt ein definiertes Modell mit einem definierten Prompt über diese Antworten
  laufen und gibt Ihnen stattdessen dessen Ergebnis. `summary_prompt` ist dann Pflicht — ein Kanal mit
  `summarize` und ohne Prompt wird am Einsatzpunkt abgelehnt, nicht still mit nichts gefaltet.
  `summary_model` ist optional; ein Kanal, der keines nennt, faltet auf der Stufe `junior`, denn eine
  Faltung ist abgeleitete Arbeit: sie gibt wieder, was andere Sitzungen bereits erzeugt haben.
  Benennen Sie ein Modell, wenn Ihre Faltung etwas **entscheidet** — ein Merge-Urteil, eine
  Weichenstellung —, denn falsch entschieden schickt sie Arbeit in die falsche Richtung.

**Eine Eskalation wird nie weggefaltet.** Hat irgendein Mitglied mit `--escalate` geantwortet, wird
die Runde durchgereicht, wie sie ist — was der Kanal auch deklariert —, und kein Synthesizer wird
gestartet. Zwei Gründe: eine Bitte um Hilfe oder eine Entscheidung ist kein Ergebnis, und ein Modell
darüber laufen zu lassen erzeugte Prosa *über* ein Scheitern an der Stelle, an der ein Anfragender
eine Antwort liest — während
die Antworten der erfolgreichen Mitglieder ungewaschen danebenstehen, was strikt mehr Information ist.

**`visibility`** ist `requester_only` (Vorgabe) oder `all_members`. Unter `requester_only` sieht der
Anfragende alles, jedes andere Mitglied die Eröffnungsnachricht und die eigenen Antworten. Gefiltert
wird nur der *Inhalt*; wer erwartet wird und wer geantwortet hat, wird nie verborgen.

Es ist eine **Zugriffs**regel und kein Anzeigekomfort, sie hängt also nicht davon ab, welche Lesung
Sie gerade nehmen: `nxc threads show` und `nxc search` geben dieselbe Antwort auf „darf ich das
sehen?", und die beiden Nachrichten-Lesungen an der Einbettungsnaht ebenso.

**Wer sonst noch lesen darf: wer die OPERATION eröffnet hat.** Eine Operation quert Kanäle — ein
Mensch fragt einen Planer, der Planer beauftragt einen Bauer in `#coding`, der Bauer eine Runde in
`#review` — und keiner dieser Kanäle nennt den Menschen in seinen `members:`. `nxc status` zeigt
diesen ganzen Baum von der Wurzel abwärts schon immer, über jede Kanalgrenze; seither folgen die
Nachrichten. Wer die Operation eröffnet hat, liest jeden Faden darin, in welchem Kanal er auch
liegt.

Für alle anderen weitet das `requester_only` nicht: diese Regel gilt zwischen den Teilnehmern
*einer Runde*, und ein Mitglied sieht die Antwort seines Nachbarn weiterhin nicht. Der Eröffner der
Operation steht nicht in der Runde — er ist die Partei, der die ganze Kette antwortet, und jede
Zusammenführung wird ohnehin zu ihm hin geliefert.

Und `members:` ist wirklich die Mitgliedschaft, in beide Richtungen: eine Änderung an der Datei
erreicht das Tor beim nächsten *Lesen*, nicht beim nächsten `send`. Ein hinzugefügter Handle liest
sofort; ein entfernter liest nicht mehr, ohne dass dazwischen irgendetwas neu gesendet werden muss.
Das gilt für **jede** Lesung der Nachrichten des Kanals, `nxc search` eingeschlossen — eine Liste,
eine Antwort, welches Verb auch fragt.

**Am KANAL deklariert nichts, wann ein Mitglied fortgesetzt wird**, und das ist auch nicht nötig:
ein Auffächern beginnt den Zug eines Mitglieds immer auf einer frischen Sitzung, und eine `nxc reply
--thread` in den eigenen Faden dieses Mitglieds setzt die Sitzung fort, die es bereits hat. Bis zu
seiner Entfernung stand hier ein Schlüssel `member_session: fresh | resume` — er steuerte nichts und
lehnte seinen eigenen zweiten Wert ab —, und eine `channels.yaml`, die ihn noch setzt, lädt
unverändert; der Schlüssel wird ignoriert.

Die eine Stelle, an der eine Deklaration es *doch* entscheidet, ist ein **Schritt**: er hat ein
eigenes `resume:` — siehe [Eine Sitzung fortsetzen oder frisch
beginnen](#eine-sitzung-fortsetzen-oder-frisch-beginnen). Je Schritt und nicht je Kanal, aus dem
Grund, an dem der entfernte Schlüssel scheiterte: was eine Runde mitnehmen soll, hängt davon ab,
über welche Kante sie ankam, und eine Antwort für den ganzen Kanal kann das nicht sagen.

## Sequenziell: die Reihenfolge ist der Ablauf

```yaml
- name: build-and-ship
  members: [coder, review]
  flow: sequential
  description: the declared order a work order runs through
  timeout: 2h
```

Ein Feld macht aus einer Gruppe einen Arbeitsablauf. `flow: sequential` sagt, dass die Reihenfolge von
`members:` **bindet**: ein Ziel nach dem anderen, jedes erst gestartet, wenn das davor erledigt ist.

**`flow: sequential` hat bewusst keine eigene Schrittliste.** Ein zweites Feld mit einer eigenen
Zielfolge wäre eine zweite Mitgliederliste neben `members`, mit eigenen referenziellen Regeln und
einer eigenen Art, der ersten zu widersprechen. `members` ist bereits eine geordnete Liste; was
fehlte, war keine Reihenfolge, sondern die Erklärung, dass die Reihenfolge bindet. Genau das sagt
dieses Feld, und alles Übrige über einen solchen Ablauf wird an `members` abgelesen.

Damit steht auch seine Grenze fest: eine flache Liste ist eine gerade Linie. Wer einen **Zyklus**
braucht — arbeiten, prüfen, zurück an die Arbeit — deklariert stattdessen `steps:`, der nächste
Abschnitt.

Das Ziel eines Schritts wird genau so angesprochen, wie `send --to` eines anspricht: ein deklariertes
Persona-Handle oder ein deklarierter **Kanalname**. So wird aus einer Review-Runde ein *Schritt*
statt etwas, das jemand von Hand öffnet — und was ein Kanalschritt beisteuert, ist seine eigene
konsolidierte Antwort.

**Wie ein Schritt weiterrückt.** Das Mitglied beantwortet seinen Platzfaden mit `nxc reply --thread
<id>`. Diese Antwort ist der ganze Mechanismus: der Betreuer läuft im selben Schreibpfad mit, sieht,
dass die Menge erledigt ist, und öffnet den nächsten Schritt. Es gibt kein „Schritt fertig"-Verb,
kein Verzweigungswort und nichts abzufragen. Endet ein Mitglied seine Sitzung ohne zu antworten,
steht der Vorgang still, bis das deklarierte `timeout` zuschlägt.

**„Erledigt" heißt `complete || stale`** — ein Schritt, der geantwortet hat, und einer, dessen Fenster
mit nichts darin ablief, geben beide den nächsten frei. Ein Schritt, der stumm blieb, wird
*gemeldet*, nie geschluckt: der Aufruf, der den Ablauf an ihm vorbeibewegt hat, trägt eine Warnung
`step_unanswered`, die den stummen Faden benennt, und die Konsolidierung, die keine Antwort von ihm
trägt, tut es ebenso.

**Ein Schritt, der eskaliert hat, gibt gar nichts frei — er beendet die Kette.** `nxc reply
--escalate` sagt „allein komme ich nicht ans Ergebnis — ich brauche Hilfe oder eine Entscheidung",
und eine Aufgabe, die ihr Ergebnis nicht erreicht hat, darf nicht die Arbeit beauftragen, die auf
sie folgen sollte. Der Nachfolger wird also nicht gestartet,
die Runde geht so nach oben, wie sie ist, mit der Eskalation daran, und wer den Kanal beauftragt hat,
entscheidet, was nun geschieht. An einem `parallel`-Kanal ändert sich nichts: dort wurden alle auf
einmal gefragt, es gibt also keinen Nachfolger zurückzuhalten.

**Eine zurückgeholte Runde beendet die Kette ebenso, gleich welcher Form.** `nxc withdraw` gehört
dem Menschen, der die erste Beauftragung des Vorgangs abgeschickt hat, nennt diesen Faden und holt
alles darunter zurück: Ein wartender Schritt wird entfernt, ein laufender gestoppt und seine Arbeit
auf einem Zweig geparkt. Danach wird nichts mehr beauftragt — nicht der nächste Schritt und auch
nicht der zurückgeholte, an einem `flow: sequential`-Kanal genauso wie an einem mit `steps:` — und
nichts wird zusammengeführt: Der Kanal-Faden wird mit einer Nachricht entlastet, die sagt, dass die
Kette unterbrochen wurde. Die Arbeit kommt zurück, wenn dieser Mensch den Kanal-Faden erneut
beauftragt. Siehe [commands](nxc-commands).

**Ein Schritt, der eine eigene Runde beauftragt hat, sagt „noch nicht", indem er nichts sagt.**
`reply` hat zwei Antworten, und beide beenden den Zug — ein Schritt, der jemanden konsultiert hat und
auf die Antwort wartet, beendet seinen Zug also einfach ohne zu antworten. Die Engine sieht, was er
beauftragt hat (sie hat diese Fäden für ihn geöffnet), und liest das als Warten statt als fehlende
Antwort: keine Erinnerung, nichts in seinem Namen gepostet, und der Ablauf steht, bis die Runde
zurückkommt und ihn weckt. `nxc status` sagt es in der Zeile — `waiting on its own sub-round` — und
der Vorgang wird nicht mit `NEEDS DECISION` markiert.

Zwei Folgen, die man besser vorher kennt. Ein blankes `nxc reply`, solange die eigene Runde offen
ist, wird **abgelehnt**: es heißt „ich bin fertig", und das stimmt noch nicht. Und arbeiten Sie
nicht weiter, während Sie warten — das Wecken startet einen zweiten Prozess unter derselben Sitzung
in derselben Arbeitskopie, und der überschreibt, was der erste noch bearbeitet hat.

`--escalate` bleibt möglich und heißt weiter, was es sagt: „ohne Hilfe oder eine Entscheidung komme
ich nicht ans Ergebnis". Danach zu greifen, um den Ablauf zu halten, war früher der Weg und ist
jetzt der falsche — es kostet den Rest der Kette und sagt dem Aufrufer, dass die Arbeit nicht kommt.

Ein **stepped** Kanal hat eine dritte Antwort, und dafür gibt es den nächsten Abschnitt.

**…und bei `working_tree: exclusive` muss auch die SITZUNG des Schritts vorbei sein.** Eine Antwort
ist eine Nachricht, und der Prozess, der sie geschrieben hat, kann weiterarbeiten: in einem echten
Lauf gemessen, antwortete ein Coder um 00:16:33 und schrieb bis 00:34:27 weiter, während der nächste
Schritt schon um 00:16:36 gestartet war — in genau die Arbeitskopie, die der Kanal für sich allein
deklariert hat. Ein Kanal, der die Arbeitskopie beansprucht, wartet deshalb auf beide Tatsachen. Die
zweite liefert die Sitzung selbst, indem sie beim Abbau ihr Ende meldet (`nxc session ended`, was
der Sidecar für Sie erledigt); bleibt das aus, antwortet die Prozessprüfung der Laufzeit beim
nächsten Mal, dass überhaupt jemand fragt — und es fragt jemand, auf einer Uhr, siehe unten. Ein `shared`-Kanal ist davon unberührt — zwei lebende
Sitzungen sind dort kein Fehler.

Eine Folge, die man besser vorher kennt: an einem solchen Kanal startet der nächste Schritt einen
Moment nach der Antwort statt im selben Augenblick, denn er startet, wenn die vorige Sitzung endet.

**Dieses Warten bewacht eine eigene Uhr.** `timeout:` weiter unten kann es nicht: es ist eine Frist
für eine ANTWORT, und ein Mitglied, das geantwortet hat, hat geantwortet. Eine Ablehnung, deren
einziger verbliebener Hinderungsgrund eine lebende Sitzung ist, plant deshalb **eine Minute später**
eine Nachprüfung desselben Bretts ein — und tut das weiter, solange der Prozess noch da ist. In der
Praxis kommt die Meldung eine Sekunde später und Sie sehen von alldem nichts. Wenn nicht — ein vor
dem Abbau getöteter Sidecar, ein `nxc` älter als diese Funktion, eine Wirt-Laufzeit, die
`Engine::session_ended` nie aufruft —, findet die Nachprüfung den Prozess weg, und der Ablauf geht
von selbst weiter, rund eine Minute später.

Von Hand fragen können Sie immer, und die Antwort ist eine Diagnose statt eines Achselzuckens:

```text
$ nxc tick --thread <der Kanalfaden>
tick declined on thread …: the set is settled, but a session behind it is still running.
```

Welche Sitzung das ist und was aus ihr geworden ist, sagt
`nxc session state --thread <der Faden des Schritts>`.

Ein Prozess, der hängt statt zu enden, ist der eine Fall, den die Uhr nicht auflösen kann: sie
schaut weiter nach und findet ihn weiter lebend — dasselbe Warten wie vorher, aber ein sichtbares,
auf das etwas schaut. Wer eine Wirt-Laufzeit baut, hält sich mit `Engine::session_ended` aus
alldem heraus.

Zwei Dinge werden an einem sequenziellen Kanal abgelehnt, beide am Einsatzpunkt und beide, ohne dass
etwas gestartet wäre:

- **`expects: <teilmenge>`** — die Auffächerung gibt eine Teilmenge unverändert zurück, sie wäre also
  eine zweite Liste, die die Reihenfolge still bestimmt. Die Ablehnung nennt beide Listen.
- **dasselbe Ziel zweimal** in `members`, sowie jede Menge von Kanälen, die sich über ihre Mitglieder
  selbst erreichen kann. Ein zyklischer Katalog wird beim Laden der Deklarationen rundheraus
  abgelehnt.

Die zweite Ablehnung ist keine Ordnungsliebe: an einem solchen Kanal ordnet der Motor einen Faden
seinem Schritt zu, indem er dessen Ziel gegen `members` abgleicht — eine Wiederholung machte zwei
verschiedene Schritte ununterscheidbar. `steps:` unten hebt sie auf, indem es jedem Schritt einen
Namen gibt.

## Schritte: wenn der Ablauf zurückkommen muss

```yaml
- name: coding
  members: [coder, review, finisher]
  working_tree: exclusive
  steps:
    - id: build
      target: coder
      next: check
    - id: check
      target: review            # eine Persona oder ein ganzer Kanal
      on_needs_rework: build    # der Weg ZURÜCK
      max_passes: 3             # Durchgänge INSGESAMT — der ursprüngliche plus zwei Nacharbeiten
      next: ship
    - id: ship
      target: finisher
```

`steps:` ist eine kleine Zustandsmaschine, und **wer in einem Schritt genannt ist, gehört damit zum
Kanal** — Sie wiederholen ihn nicht unter `members:`, und Sie nennen den Kanal auch nicht bei der
Persona. Die Besetzung eines Kanals sind seine Schrittziele plus das, was `members:` hinzufügt,
einmal deklariert, hier. **Ein Kanal mit `steps:` darf nicht zusätzlich `flow: sequential`
deklarieren**: das wären zwei Listen, die dasselbe entscheiden.

`members:` ist damit für einen Sitz da, den kein Schritt nennt — ein Mensch, der die Runde
mitliest, eine Persona, die liest und nichts ausführt. Ein Schrittziel dort trotzdem aufzuführen ist
nur redundant (`nxs prime` sagt es) und ändert nichts; jeder Kanal, der vor dieser Regel geschrieben
wurde, bedeutet genau das, was er bedeutet hat.

Vorher mussten die beiden Listen übereinstimmen, und nichts sagte das: ein Schrittziel, das in
`members:` fehlte, wurde beauftragt, bekam seinen Faden und durfte antworten — und wurde
abgewiesen, als es das Gespräch *lesen* wollte, auf das man es gesetzt hatte.

- **`id`** benennt den Schritt, und erst das macht einen Zyklus möglich. Zwei Schritte dürfen
  dieselbe Persona ansprechen, und derselbe Schritt darf in einer Runde zweimal laufen, weil der
  Motor den Faden eines Schritts an dessen `id` festmacht statt am Namen seines Ziels.
- **`next`** ist der Normalweg. Ein Schritt ohne `next` ist das Ende der Runde: dort konsolidiert der
  Kanal und antwortet dem, der gefragt hat.
- **`on_needs_rework`** ist der Weg zurück, und die einzige Abzweigung, die es gibt.
- **`max_passes`** zählt **Durchgänge insgesamt, nicht Nacharbeiten**: der ursprüngliche Lauf ist
  Durchgang 1, `max_passes: 3` erlaubt also zwei Wege zurück. Alles unter `2` wird abgelehnt — es
  erklärte eine Rückkante, die nie genommen werden kann, und ein Schritt, der nie zurückschicken
  soll, deklariert schlicht kein `on_needs_rework`. Es sitzt an der Rückkante, nie am Kanal: ein
  Kanal kann mehrere Zyklen haben, und ein Kanalzähler würde sie zusammenzählen. Es ist Pflicht, wo
  die Kante steht — eine Rückkante ohne Decke ist eine Schleife, die nichts beendet.
- **`resume`** entscheidet, ob der Schritt die Sitzung FORTSETZT, die sein Ziel in dieser Runde schon
  geführt hat, oder eine frische beginnt. Lässt man es weg, entscheidet die *Kante* — und das ist
  fast immer das, was man will; siehe unten.
- **`task`** sagt, was das Ziel *hier* tun soll, und wird zu einer vierten Ebene seines Prompts,
  unter dem der Persona (`nxc guide personas`). Siehe unten.
- **`stage`** lässt das Ziel auf einer anderen Stufe laufen als der, die es selbst deklariert hat —
  `junior | senior | principal`, das Vokabular der Personas (`nxc guide personas`). Siehe unten.
- **`input`** sagt, was dieser Schritt aus dem Durchgang mitbekommt: den Auftrag, die Antwort eines
  benannten Schritts, beides oder nichts. Ohne Angabe gelten die Vorgaben, und die sind fast immer
  das, was man will — siehe unten.

### Dieselbe Rolle, eine andere Aufgabe

Eine Persona sagt, wer sie ist: welche Dimension sie vertritt, wie sie benotet, ab wann sie
zurückschickt. Ein Schritt sagt, was ihr vorliegt. Das Zweite aus der Persona herauszuhalten ist,
was einen deklarierten Prüfer an zwei Stationen brauchbar macht:

```yaml
    - id: check
      target: review            # dieselben drei Prüfer wie in der Codekette
      stage: senior             # … aber ein Entwurfsurteil ist teurere Denkarbeit als ein Diff
      task: |
        Du beurteilst eine SPEZIFIKATION, keinen Diff. Lies das im Auftrag genannte Dokument und
        frage, was passieren WÜRDE, wenn es so gebaut wird.
```

Beide Schlüssel sind freiwillig, und ein Schritt, der keinen davon deklariert, verhält sich genau
wie vor ihrer Einführung.

- **`task` ergänzt, es ersetzt nie.** Es kommt unter dem eigenen Prompt der Persona an, als das des
  Schritts gekennzeichnet, und die Engine setzt einen eigenen Satz darüber: wo beide sich zu
  widersprechen scheinen, gewinnt die Rolle — wie sie urteilt, wie sie antwortet und wo ihre
  Schwelle liegt, gehört ihr und keinem Aufrufer. Ein Kanal kann einem Prüfer also seine Rubrik
  nicht wegnehmen, und Sie müssen nicht in jede Aufgabe „und halte Dich weiter an alles aus Deiner
  Rolle" hineinschreiben.
- **`stage` bewegt die Stufe und sonst nichts.** Ein Schritt darf **kein** Modell benennen — der
  Schlüssel wird beim Laden des Teams namentlich abgelehnt und verweist auf `stage:`. Eine Persona
  mit eigenem `model:` läuft darauf, was der Schritt auch verlangt; eine Persona mit nur einer Stufe
  oder ohne läuft auf der des Schritts.
- **Ein Schritt, dessen Ziel ein ganzer Kanal ist, reicht beides an jedes Mitglied weiter.** Nur
  diese Platzierung kann überhaupt etwas bedeuten: ein Kanalfaden führt keine Sitzung, seine
  Mitglieder tun es. Weiter reicht es nicht: Wen die Sitzung des Schritts danach selbst mit
  `nxc send` beauftragt, erledigt eine Besorgung dieser Sitzung und nicht den Schritt — und erfährt
  davon nichts.

### Eine Sitzung fortsetzen oder frisch beginnen

Ein Schritt, der über `on_needs_rework:` erreicht wird, ist dieselbe Stelle, die ihre eigene Arbeit
zurückbekommt. Sie wird mit einem Hinweis geweckt, der sagt *„was Du übergeben hast"* und *„Dein
Anspruch auf die Arbeitskopie ist weiterhin Deiner"* — Sätze, die nur für die Sitzung wahr sind, die
tatsächlich etwas übergeben hat. Deshalb **setzt eine Rückkante diese Sitzung fort**: die Befunde
kommen als ihr nächster Zug an, alles Gebaute steht noch in ihrem Kontext, und sie muss ihre eigene
Arbeit nicht erst wieder aus der Arbeitskopie herleiten.

Ein Schritt, der über `next:` erreicht wird, ist das nächste Stück Arbeit. Dass er zufällig dieselbe
Persona nennt, ist für sich kein Grund, eine Sitzung mitzunehmen — deshalb **beginnt eine Vorwärts­kante
frisch**.

Das ist die Vorgabe, und `resume:` am Schritt sticht sie in beide Richtungen — dieselben zwei
Schritte wie oben, beide Vorgaben umgedreht:

```yaml
    - id: build
      target: coder
      next: check
      resume: false             # eine Nacharbeit geht jedes Mal in eine SAUBERE Sitzung
    - id: check
      target: review
      on_needs_rework: build
      max_passes: 3
      next: ship
      resume: true              # … während der Prüfer seine Sitzung über die Durchgänge behält
```

Ein Prüfer, der seine Sitzung behält, ist keine Spielerei: beim zweiten Blick weiß er noch, was er
verlangt hat, und kann beurteilen, ob es geschehen ist.

In beiden Fällen bekommt der Schritt einen **frischen Faden** — das ist es, was der
Durchgangszähler zählt. Fortsetzen betrifft die Sitzung, nicht den Platz. Eine Runde, die dieses Ziel
noch nie geführt hat, hat nichts fortzusetzen und beginnt schlicht — das ist der erste Durchgang und
kein Fehler. Und ein Schritt, dessen Ziel ein ganzer **Kanal** ist, lässt sich gar nicht fortsetzen:
ein Kanal hat keine einzelne Sitzung hinter sich, seine Mitglieder haben je eine eigene. `resume:
true` wird dort beim Laden des Teams als fehlerhafte Deklaration gemeldet.

### Die dritte Antwort

Ein Schritt, dessen Deklaration eine `on_needs_rework:`-Kante nennt, gibt der Stelle, die ihn
bedient, einen dritten Weg, ihren Zug zu beenden:

```text
nxc reply --thread <id> -                   # fertig
nxc reply --thread <id> --needs-rework -    # fremde Arbeit geht zurück
nxc reply --thread <id> --escalate -        # ich brauche Hilfe oder eine Entscheidung; hoch damit
```

(Es gibt eine vierte Flagge, `--accept`, und sie steht bewusst nicht in dieser Liste: sie ist keiner
der Ausgänge, die ein Schritt hat. Sie gehört dem, der die Runde **beauftragt** hat, auf dem Faden
der Runde selbst — siehe [Wenn die Decke erreicht ist](#wenn-die-decke-erreicht-ist).)

Das `-` liest die Nachricht von STDIN — `nxc reply --thread <id> - <<'EOF'`, Ihr Text, dann `EOF` in
einer eigenen Zeile — sodass die Shell nichts darin auswertet. Genau hier zählt das am meisten: ein
Urteil nennt Dateien, zitiert Befehle und klebt deren Ausgabe hinein, und in der Argumentform
`nxc reply --thread <id> "…"` werden Backticks und `$(…)` darin umgeschrieben, bevor `nxc` sie sieht.
Eine einzeilige Antwort darf weiterhin ein Argument sein.

Die drei sind eine Trichotomie über *erledigt*, *nochmal*, *nicht allein*, und einen vierten Fall
gibt es nicht. „Gut, aber unvollständig" ist Nachbesserung. „Ich brauche erst X" ist Eskalation —
und „diese Entscheidung gehört nicht mir" ebenso, selbst wenn Sie jede der Antworten ausführen
könnten. „Passt" ist der Normalweg. Man beachte, um wessen Arbeit es jeweils geht: `--escalate` ist eine Aussage über
die **eigene** Arbeit, `--needs-rework` über **fremde**. Beides zugleich wird abgelehnt.

**Einem Schritt ohne Rückkante wird das Bit gar nicht erst angeboten** — seine Sitzung erfährt von
zwei Ausgängen, nicht von dreien. Das ist Absicht: zeigt der Ablauf auf eine Wertung keine Reaktion,
dann war um eine Einschätzung gebeten, nicht um eine Entscheidung. Wer es dort dennoch setzt (über
`--help` gefunden), dessen Wertung fällt weg — mit einer Warnung `verdict_dropped` auf der Quittung
genau dieser Antwort, die sagt, dass dieser Kanal dafür keinen Übergang erklärt.

**Ist das Ziel des Schritts ein ganzer Kanal, wird die dritte Antwort jedem Mitglied angeboten, und
ein einziges genügt.** Der Schritt nennt den Kanal, aber auf dem Schritt selbst steht niemand — seine
Mitglieder stehen dort, mit je einer Sitzung. Jedes von ihnen liest dieselben drei Ausgänge, die eine
direkt angesprochene Rolle läse, und die Wertung eines einzigen schickt die ganze Runde zurück. Ein
Prüfer, der etwas gefunden hat, wird nicht von dreien überstimmt, die nichts gefunden haben — und das
eigene `on_complete: summarize` des Kanals faltet die Antworten weiterhin zu dem einen Urteil, für das
er deklariert wurde.

Es gilt die nächstgelegene Deklaration, und daraus folgt eine Ausnahme, die man kennen sollte: Erklärt
der von einem Schritt beauftragte Kanal eigene `steps:`, dann liest, wer einen davon bedient, DESSEN
Ausgänge — sein eigenes `on_needs_rework:`, oder zwei Ausgänge, wo er keines erklärt — und erbt nicht
die des äußeren Schritts.

### Was mit der Arbeit zurückkommt

Die zurückgeschickte Stelle wird mit der Wertung selbst geweckt — die Befunde *sind* die Aufgabe —
unter einem Hinweis, der sagt, was geschehen ist, der wievielte Anlauf das ist und dass ihr Anspruch
auf die Arbeitskopie **weiterhin ihrer** ist:

```text
NEEDS REWORK — this is NOT an approval. What you handed over was reviewed and did not meet the
standard of the party that reviewed it; their reasons are in the body below, and they are the work.
This is pass 2 of 3. Your claim on the working copy is STILL YOURS and is held across this round — do
not acquire it again. …
```

Mit `rework_notice:` am Kanal sagt man es in eigenen Worten; `{pass}` und `{max}` werden eingesetzt.

### Was ein Schritt mitbekommt: `input`

**Die Vorgaben tragen den gewöhnlichen Kanal, und dieses Feld schreibt man selten.** Der erste
Schritt eines Durchgangs bekommt den Auftrag, mit dem er eröffnet wurde. Jeder weitere bekommt die
**Antwort des Schritts, von dem aus er erreicht wurde**. Für den Kanal am Anfang dieses Abschnitts
ist das genau richtig: mit einer Rückkante von `check` nach `build` folgt auf jedes `build` wieder
ein `check`, aber nicht auf jedes `check` ein `build` — der Schritt vor `ship` ist damit *immer* die
Prüfung, und der Finisher liest das Urteil, ohne dass irgendetwas deklariert wäre.

Das Feld schreibt man dort, wo man eine andere Form gewählt hat. Dieselben drei Stationen, flach
ausgeschrieben statt geschleift:

```yaml
  steps:
    - id: build
      target: coder
      next: check
    - id: check
      target: review
      next: fix
    - id: fix
      target: coder
      next: ship
    - id: ship
      target: finisher
      input: [check]            # das Urteil, nicht der Bericht des Coders über die eigene Arbeit
```

Hier ist der Schritt vor `ship` der Coder, die Vorgabe reichte dem Finisher also *„beide Befunde
behoben"* statt der Befunde. Ein Schlüssel rückt das gerade. (In einer so geschnittenen Kette tut
der Coder die Abschlussarbeiten ohnehin, und `ship` könnte ganz entfallen — das ist Kanaldesign, und
es gehört dem Designer.)

Die Formen:

```yaml
input: [request]              # nur die Worte, mit denen der Durchgang eröffnet wurde
input: [request, check]       # beides
input: [build]                # die Antwort eines bestimmten Schritts, auch über eine Schleife hinweg
input: []                     # nichts außer der eigenen Aufgabenbeschreibung dieses Schritts
```

- **`request`** ist der einzige Name, der kein Schritt ist. Ein Schritt unter dieser id wird beim
  Laden zurückgewiesen, und ebenso ein `input:`, das einen Schritt nennt, den dieser Kanal nicht
  deklariert — ein Tippfehler darf keinen Schritt erzeugen, der auf nichts läuft und nichts darüber
  sagt.
- **`input: []` ist der Abschalter** und braucht kein zweites Feld. Der Fall dafür: ein Verifier, der
  den **Baum** prüfen soll und nicht den Bericht des Coders darüber. Ihm wird gesagt, dass er nichts
  mitbekommen hat — ein leerer Auftrag ließe sich sonst nicht von einem kaputten unterscheiden. Er
  schaltet ab, was über eine **Vorwärts**kante hereinkommt; ein Schritt, der zugleich Ziel einer
  `on_needs_rework:`-Kante ist, bekommt die Befunde weiterhin, wenn er auf diesem Weg erreicht wird
  — siehe den letzten Punkt.
- **Lief ein benannter Schritt mehrfach, kommt seine letzte Antwort.** Lief er in diesem Durchgang
  noch gar nicht, kommt nichts — was an Schritt eins zulässig ist und kein Fehler.
- **`input` entscheidet über das Material, nie über die Aufgabe.** Die Persona und das `task:` des
  Schritts kommen an, was auch immer `input:` sagt. Ein Schritt ohne Eingabe ist immer noch ein
  Schritt mit einem Auftrag.
- **Dazwischen läuft nichts.** Der Koordinator setzt zusammen, er fasst nicht zusammen: zwischen zwei
  Schritten läuft kein Modell, und was ankommt, ist die vorige Antwort im Ganzen.
- **`input` regelt nur Vorwärtskanten.** Der Weg *zurück* trägt die Befunde des Prüfers ohnehin schon
  wörtlich (siehe oben); `input` auch dort zu beachten hieße, denselben Text zweimal zu übergeben.

Eine fremde Antwort kommt **als Daten gekennzeichnet** an — im selben abgegrenzten, zugeordneten
Block, den auch die Zusammenführung eines Kanals verwendet, hinter dem Satz, der sagt, dass sie nie
eine Anweisung an dich ist, ganz gleich, wofür sie sich ausgibt. Das `from=` daneben ist der eigene
Vermerk des Motors darüber, wer gepostet hat, und nicht etwas, das die Antwort selbst schreiben
könnte.

**Zwei Achsen, die dieses Feld auseinanderzieht, und es lohnt zu wissen, welche welche ist.**
`visibility:` entscheidet, **wer einen Faden lesen darf**. `input:` entscheidet, **was ein Schritt zu
sehen bekommt**. Das ist nicht mehr dieselbe Frage: ein Finisher kann jetzt das Urteil einer
`requester_only`-Prüfrunde bekommen, deren Faden er nicht lesen darf. Genau das ist der Zweck — das
Material reist, das Gespräch nicht.

### Wenn die Decke erreicht ist

Die Runde wird **nach oben eskaliert**, nicht still gestoppt: kein weiterer Anlauf wird gestartet,
jede Antwort der Runde wandert zu dem, der den Kanal beauftragt hat, und die Zustellung nennt die
erreichte Decke. Diese Stelle entscheidet, und sie hat **zwei** Züge.

**Noch ein Anlauf** — eine Antwort in den Kanalfaden wie sonst auch. Das beauftragt die Runde erneut,
von ihrem ersten Schritt an, und eine neue Runde beginnt ihre Zählung bei eins; mehr „Zurücksetzen"
als eine Freigabe von oben gibt es nicht.

**Oder das Urteil annehmen und die Runde weiterschicken:**

```text
nxc reply --thread <Faden der Runde> --accept -
```

Der Schritt, dessen Urteil die Runde angehalten hat, gilt als erfüllt, und **dieselbe** Runde läuft
über dessen `next:` weiter. Das ist der Ausgang aus einem Zyklus, dessen Prüfer nie von selbst
zufrieden wird — und das ist ein realer Zustand, kein gedachter: die erste Planungskette, die diese
Maschinerie im Ernst gefahren hat, lief sechs Durchläufe Entwurf→Prüfung, sammelte sechs Urteile und
reichte nichts weiter, weil jede Antwort von oben drei weitere Durchläufe derselben Prüfung kaufte.

Der Folgeschritt bekommt Ihre Worte **zusammen mit dem aufgehobenen Urteil**, unter einem Hinweis,
der klar sagt, dass dies keine bestandene Prüfung ist. Genau diese Paarung ist der Punkt: ein
Folgeschritt, der nur die Befunde bekäme, arbeitete gegen ein gerade aufgehobenes Urteil weiter, und
einer, der keines von beidem bekäme, meldete weiter unten, die Arbeit sei sauber geprüft worden.

Zwei Regeln machen daraus eine Entscheidung statt einer Abkürzung:

- **Nur wer die Runde beauftragt hat, darf sie geben.** Auf einem Schritt, den Sie *bedienen*, wird
  das Verb namentlich abgelehnt und auf `--escalate` verwiesen: wer die Arbeit erzeugt hat, darf
  seinen eigenen Prüfer nicht für zufrieden erklären — das ist die Trennung, um derer willen die
  Runde zwei Parteien hat. `--escalate` ist, wie ein Agent um diese Entscheidung bittet; `--accept`
  ist, wie die Stelle darüber sie gibt.
- **Nichts daran liest den Zähler.** Es ist kein Recht, das erst die Decke freischaltet — eine Runde,
  die vor ihrer Decke stehengeblieben ist (weil etwa ein Mitglied eskaliert hat), lässt sich genauso
  annehmen. Was es braucht, ist eine Runde, die *steht*: solange ein Schritt noch arbeitet, wird das
  Annehmen abgelehnt, denn den Folgeschritt daneben zu öffnen hieße, zwei Sitzungen in eine
  Arbeitskopie zu setzen.

Die Annahme wird als eigene Art aufgezeichnet, damit eine Übersteuerung später als Übersteuerung
wiederzufinden ist — `nxc threads show` zeigt sie im Faden der Runde, und sie ist nicht „die Prüfung
wurde bestanden".

### Was die Runde am Ende berichtet

Eine Runde, die über eine Rückkante gegangen ist, übergibt zwei Antworten desselben Schritts, die
einander absichtlich widersprechen — *„the lock order is wrong"* und *„good now"*. Deshalb kommen die
Antworten nicht als flache Liste an: jede trägt den Schritt, dem sie diente, und den wievielten
Durchgang dieses Schritts sie war, und die Antwort, die die Wertung trug, ist als solche markiert.

```text
<message from="local/coder" step="build" pass="1">
first cut is in: added the cache
</message>
<message from="local/review" step="check" pass="1" needs_rework="true">
the lock order is wrong in two places
</message>
<message from="local/coder" step="build" pass="2">
lock order fixed
</message>
```

Über dem Block sagt die Zustellung die eine Sache, die sich aus den Nachrichten selbst nicht
herleiten lässt: **taucht ein Schritt mehrfach auf, löst der spätere Durchgang den früheren ab** —
die früheren Durchgänge sind der Weg hierher, keine Befunde, die noch stehen.

An derselben Stelle nennt sie außerdem, wie oft jeder Schritt wirklich gelaufen ist — vom Motor aus
seinem eigenen Protokoll gezählt. Das ist die eine Sache, die ein Leser nicht durch Lesen ermitteln
kann: mit wie vielen Durchgängen zu rechnen ist, und genau das macht „der spätere Durchgang löst den
früheren ab" zu einer Regel über eine bekannte Zahl von Dingen. Die Marken selbst sind die des
Motors, und keine Antwort kann eine schreiben — die Grenze, die das garantiert, steht oben bei
`pass_through`.

Beide Ausgabeformen tragen es. `on_complete: pass_through` zeigt es dem, der gefragt hat;
`on_complete: summarize` übergibt dieselbe Struktur der Sitzung, die den Abschlussbericht schreibt —
was zurückkommt, ist also aus der Runde *gebaut* statt durchgereicht. Das `summary_prompt:` sagt,
wozu der Bericht dient; die Gestalt des Laufs wird mitgeliefert.

Ein Kanal ohne `steps:` hat keine Durchgänge, und seine Zustellung ist genau die, die sie immer war.

## Fristen

`timeout:` ist, wie lange eine Runde warten darf. Es wird hier deklariert oder nirgends — die
Übersteuerung `--deadline` je Aufruf ist entfallen, weil ein deklariertes Fenster und die
Übersteuerung eines Aufrufers zwei Antworten auf eine Frage sind.

```yaml
timeout: 20m                      # ein Fenster: 20 Minuten Stille
timeout: 2026-08-16T10:30:00Z     # ein Zeitpunkt: dieser Moment, was auch geschieht
```

Der Parser versucht RFC3339 zuerst, beide Formen sind also in einem Feld ausdrückbar — und der
Unterschied ist real. **Eine Dauer ist ein Fenster je Mitglied und rücksetzbar**: die Uhr jedes
Mitglieds wird von jedem Schreiben in dessen eigenen Sitzungsverlauf neu gestartet, ein Mitglied, das
*arbeitet*, wird also nie dafür bestraft, dass es dauert. Wovor es schützt, ist ein Mitglied, das
verstummt ist. **Ein absoluter Zeitpunkt bewaffnet keine Uhr**, und nichts verschiebt ihn.

Ein Wert, den die Grammatik nicht lesen kann, ist ein `validation`-Fehler, der Feld und Wert nennt,
abgelehnt bevor irgendetwas persistiert wird — nie ein stilles „dann gibt es eben keine Kappe".
Einheiten sind `s`, `m`, `h`, `d`, `w`.

Ist ein Fenster deklariert, stellt chat eine Frist auf den Moment, in dem es erstmals fällig wird.
Ist der Moment da, läuft ein verstecktes Verb: `nxc tick --thread <id>`. Es prüft den Faden erneut
und leitet ihn, wenn fällig, durch die Politik des Kanals. Weil sich das Fenster mit jedem
Lebenszeichen der Mitglieder verschiebt, bewaffnet sich die Kette selbst neu: ein Tick, der nichts
Fälliges findet, stellt die nächste Frist. Er ist idempotent — ein bereits behandelter Faden ist ein
sauberer No-Op, nie ein zweites Wecken.

Niemand tippt `tick`. Es steht hier, damit es kein Rätsel ist, wenn man es in einem Protokoll oder
einer Prozessliste findet.

### Die Frist hält der nexus-flow-Hintergrunddienst

Eine Frist zu stellen schreibt eine Zeile in das Buch des Arbeitsbereichs, `.nxs/timers.json`. Der
nexus-flow-Hintergrunddienst — ein Prozess, derselbe, der Ihre Arbeitsbereiche synchronisiert —
liest dieses Buch bei jedem Durchgang und führt die Prüfung aus, wenn es so weit ist.

Standardmäßig heisst er `nexus-flow`, und auf einem Mac ist das der Name, den Sie in den
**Systemeinstellungen › Allgemein › Anmeldeobjekte** finden. Einmal einrichten:

```
nxs sync daemon install          # macOS: ein launchd-Agent, der ihn bei der Anmeldung startet
nxs sync daemon                  # überall: im Vordergrund, unter der eigenen Prozessverwaltung
```

Sein Protokoll liegt in `~/.nexusflow/logs/service.log`. `nexus-flow` ist auch ein Befehl — er *ist*
`nxs sync daemon` —, `nexus-flow status` beantwortet also „läuft er, und was hat er zuletzt getan".

#### Er hält den Rechner wach, solange ein Lauf arbeitet

Ein Lauf, der stundenlang unbeaufsichtigt arbeitet, überlebt es nicht, wenn der Mac einschläft.
Solange in irgendeinem betreuten Arbeitsbereich eine Sitzung lebt, hält der Dienst eine
Ruhezustands-Sperre — und gibt sie in dem Moment zurück, in dem die letzte vorbei ist. Sie können
sie unter ihrem Namen in `pmset -g assertions` sehen; dort steht auch, welche Instanz sie hält.

Sie verhindert nur den Ruhezustand aus *Untätigkeit*: Ihr Bildschirm schläft weiterhin ein, ein
zugeklapptes Gerät schläft weiterhin ein, und „Ruhezustand" aus dem Menü funktioniert weiterhin.
Und sie wird vom Dienstprozess selbst gehalten — stirbt der Dienst, stirbt die Sperre mit ihm, ein
Absturz kann Ihren Mac also nicht dauerhaft wachhalten.

#### Mehrere Dienste auf einem Rechner

Wer an mehreren Arbeitskopien gleichzeitig entwickelt, will nicht, dass sie sich eine Uhr teilen:
wer zuletzt installiert, besäße sie — für alle anderen mit. Geben Sie einer Arbeitskopie einen
eigenen **Instanznamen**, und sie bekommt alles Eigene: einen eigenen launchd-Agenten, ein eigenes
Verzeichnis `~/.nexusflow-<name>` und damit eine eigene Registry, einen eigenen Riegel, einen eigenen
Herzschlag und ein eigenes Protokoll:

```
export NXS_SERVICE_INSTANCE=nexus-flow-dev    # in Ihrer .envrc, einmal je Arbeitskopie
cargo build                                   # … und installieren Sie sie aus dem BAU heraus:
./target/debug/nxs sync daemon install        # installiert *diese* Instanz mit *dieser* Binärdatei
./target/debug/nxs sync daemon status         # sagt, für welche Instanz er antwortet
```

**Es entscheidet die laufende Binärdatei; die Variable benennt nur.** Eine Binärdatei aus
`target/debug` oder `target/release` ist eine Entwicklungsinstanz, und `NXS_SERVICE_INSTANCE` sagt,
*welche* — ein Rechner hat mehrere Arbeitskopien. Die installierten `nxs`, `nxf`, `nxm` und `nxc` in
Ihrem `PATH` sind der Dienst des Rechners selbst, und daran ändert es nichts, in welchem Verzeichnis
Sie stehen: Die Variable hat über sie keine Stimme, Ihre alltäglichen Befehle sprechen also weiter
mit dem Dienst, der die Uhr Ihres Rechners stellt.

Deshalb fährt die Installation oben die gebaute und nicht die installierte Datei. Eine Installation
richtet die Verknüpfung der Instanz auf genau die Binärdatei, die sie ausgeführt hat — aus dem Bau
heraus zu installieren ist also das, was einen Entwicklungsdienst Ihren Entwicklungsstand auch
wirklich fahren lässt; aus der installierten Datei heraus hätte es lediglich die Produktion unter
einem Entwicklungsnamen erneut installiert. `nxs sync daemon install` sagt es, wenn die Variable eine
Instanz genannt hat, der es keine Stimme geben konnte.

Ein Name ist `nexus-flow` oder `nexus-flow-<etwas>`. Der Name der Instanz ist zugleich der Befehl:
`nexus-flow-dev status` ist das `status` genau dieses Dienstes — und er ist auch, woran der laufende
Dienst erkennt, welcher er ist: der launchd-Agent fährt eine Verknüpfung dieses Namens, und der Name
ist die ganze Anweisung — die Verknüpfung steht für `nxs sync daemon`, das Verb folgt ihr also
direkt. Über sie erreichen Sie eine bereits installierte Instanz auch von überall her:
`~/.nexusflow-dev/bin/nexus-flow-dev uninstall` entfernt *diesen* Dienst, gleich welche Binärdatei
hinter der Verknüpfung steht.

Zwei Instanzen können einander nicht in die Quere kommen: getrennte Verzeichnisse heißen getrennte
Riegel, also laufen beide nebeneinander, und die Installation der einen entfernt die andere nie. Das
Einzige, was sie sich *teilen* können, ist ein Arbeitsbereich — nichts hindert Sie daran, dieselbe
Arbeitskopie bei beiden anzumelden — und dann betreuen ihn beide, lesen beide sein Fristenbuch, und
eine fällige Frist kann zweimal gestartet werden. Melden Sie einen Arbeitsbereich bei genau einer
Instanz an; `nxs sync bind`, `nxs sync daemon status` und der laufende Dienst sagen es, wenn Sie es
nicht getan haben.

Diese Antwort sagt außerdem, **welche Binärdatei er fährt**: das Programm, aus dem der laufende
Dienst gestartet ist, dessen Version, und worauf die `nexus-flow`-Verknüpfung *jetzt* zeigt. Beides
kann auseinanderlaufen — `nxs self-update` schreibt eine neue Binärdatei und verschiebt die
Verknüpfung bewusst nicht, welcher Bau die Uhr dieser Maschine stellt, bleibt also eine Entscheidung
—, und `status` sagt es, wenn sie auseinandergelaufen sind. Zeigt die Verknüpfung auf eine Datei,
die es nicht mehr *gibt*, ist das der eine Fehler, den launchd überhaupt nicht melden kann (sein
einziges Symptom ist ein Dienst, der nie startet). Dieser Fall bleibt deshalb nicht `status`
überlassen: jeder Aufruf von `nxs`, `nxf`, `nxm` und `nxc` sagt ihn, benennt das tote Ziel und nennt
den einen Befehl, der ihn behebt.

Was das bringt, verglichen damit, für jede Frist einen Job beim Betriebssystem zu bestellen (so war
es vorher):

- **Sie gilt auf die Sekunde.** Nichts wird auf die nächste volle Minute aufgerundet.
- **Ein früh geschlossenes Brett lässt nichts zurück.** Die Zeile wird überschrieben oder entfernt;
  kein Agent bleibt in Ihrer Anmeldesitzung stehen für ein Fenster, das es nicht mehr gibt.
- **Überall dieselbe Mechanik.** macOS, Linux und alles andere, worauf der Dienst läuft, lesen
  dasselbe Buch.

**Das hat sich geändert, und es lohnt sich zu wissen, falls Sie es umgangen haben.** `nxc` plante
früher über `at` — und macOS liefert `atrun` *deaktiviert* aus, also nahm `at` den Job an, beendete
sich mit 0, druckte eine Job-Nummer, und niemand führte ihn je aus. Auf einem Mac ab Werk stapelten
sich Jobs, deren Termin längst verstrichen war, und ein deklariertes `timeout:` feuerte dort **nie**:
ein Mitglied, das verstummte, hielt seine Runde unbegrenzt an, weil nichts das Brett je `stale`
werden liess. Wer dafür `atrun` von Hand aktiviert hat, braucht das nicht mehr. `NXC_TIMER=at` wählt
den `at`-Zeitgeber weiterhin ausdrücklich, auf jeder Plattform, falls Sie ihn wollen.

### Und was es kostet, ausgesprochen

Ohne Dienst keine Uhr. `at` trug sich selbst — den Job abgeben, und das Betriebssystem besitzt ihn,
ob von uns etwas läuft oder nicht. Das hier tut das nicht, und `nxc` tut nicht so als ob: eine Frist
in einem Arbeitsbereich zu stellen, den kein Dienst betreut, oder während keiner läuft, **trägt die
Frist trotzdem ein** (ein später gestarteter Dienst hält sie) und meldet es zugleich auf der
Quittung, als Eintrag `tick_unscheduled` im `warnings`-Array, mit dem betroffenen Brett und dem, was
zu tun ist. Ein von Null verschiedener Exit-Code ist es bewusst nicht: das Brett ist offen und seine
Mitglieder laufen — was fehlt, ist die Sicherung.

Zwei Fälle unterscheidet es, weil sie verschiedene Antworten brauchen:

- *dieser Arbeitsbereich ist beim Dienst nicht angemeldet* — anmelden (`nxs sync bind`, oder aus
  Ihrer App), dann `nxs sync daemon install`;
- *es läuft kein Dienst* — starten.

Wenn Sie ihn doch sehen: `nxc tick --thread <id>` ist dasselbe Verb, das der Dienst ausgeführt hätte,
idempotent und jederzeit aufrufbar. Nichts geht verloren — das Fenster wird geprüft, wenn jemand
fragt, statt von selbst.

Wenn eine Runde nicht hängen bleiben darf und Sie den Dienst nicht laufen lassen können, verlassen
Sie sich dafür nicht auf `timeout:` — behandeln Sie die Sicherung als abwesend und sehen Sie auf dem
Brett nach.

## Alleinnutzung der Arbeitskopie

```yaml
- name: review
  members: [general, code-quality, test-quality, integrity]
  flow: sequential
  working_tree: exclusive
```

Am *Kanal* deklariert, wenn die Runde das Repository braucht und nicht ein einzelnes Mitglied für
sich — ein Review-Quorum, in dem jedes Mitglied die Gates des Projekts selbst laufen lässt, ist genau
diese Form. Es geht auch andersherum: sagt ein deklariertes Mitglied für sich `working_tree:
exclusive`, gilt der ganze Kanal als bedürftig, und diese Zeile wird nicht wiederholt. Diese Vererbung
gilt **nur einen Sprung** — ein Kanal fängt sie sich nicht über ein gemeinsames Mitglied bei einem
anderen Kanal ein.

Wovor sie schützt und wovor nicht: Sie schützt diese Runde vor *anderen* Ketten, die dann warten,
statt ihren eigenen Build gegen dasselbe Zielverzeichnis laufen zu lassen. Sie trennt die Mitglieder
*einer* Runde **nicht** voneinander — sie sind ein Anspruchsgebiet, das erste erwirbt und die übrigen
erben, also laufen alle gleichzeitig. Die Einheit der Ausschließung ist das Gebiet, nicht die Sitzung.

**Deshalb steht im Beispiel oben auch `flow: sequential`, und das ist keine Zierde.** Vier Mitglieder,
die jedes den Build des Projekts in derselben Arbeitskopie laufen lassen, sind vier Builds in einem
Zielverzeichnis. Zwei gleicher Form nehmen bloß die Sperre des Build-Werkzeugs und warten aufeinander
— richtig, und um ein Vielfaches langsamer. Zwei *verschiedener* Form überschreiben einander die
Artefakte, und was Sie dann lesen, ist ein kaputter Build, der in Wahrheit ein zerschossenes Artefakt
ist. `working_tree:` kann sie nicht trennen; die *Reihenfolge* kann es, denn der nächste Schritt eines
geordneten Ablaufs öffnet erst, wenn der vorige geantwortet hat **und** seine Sitzung vorbei ist.
Also: ein Quorum, dessen Mitglieder nur lesen, darf auffächern und gibt Ihnen vier Meinungen auf
einmal; ein Quorum, dessen Mitglieder BAUEN, will beide Zeilen. Der Preis der zweiten ist Wanduhrzeit
— und die Regel geordneter Abläufe, dass eine eskalierende Antwort die Kette beendet: ein Mitglied,
das nicht laufen kann, hält die Runde an, statt die Urteile der anderen übrig zu lassen. Das ist
meist genau das, was man will, wenn ohnehin alle dieselbe Arbeitskopie teilen.

**Und das Gebiet ist die OPERATION** — der ganze Fadenbaum, zu dem die Arbeit gehört, dieselbe
Einheit, nach der `nxc status` gruppiert, und nicht nur der Zweig, in dem der exklusive Schritt
zufällig begann. Der Anspruch *entsteht* weiterhin beim ersten exklusiven Schritt, eine Kette, die nie
einen erreicht, hält also nichts; ist er einmal genommen, wird er zurückgegeben, wenn die Operation
fertig ist. Genau das nimmt der zweiten Coding-Runde eines Planers die Lücke davor. Soll die Kopie ab
dem ersten Zug der Operation gehalten werden — also auch während der Planung —, deklarieren Sie
`working_tree: exclusive` an dem Kanal, der sie eröffnet.

So sieht es aus, wenn eine Runde hinter einem Gespräch geparkt ist, das die Arbeitskopie bereits hält:

```console
$ NXC_ACTOR=alice nxc threads list
m-00000000000000000000000003  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000004  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1/1 in  [complete]
m-00000000000000000000000002  dm:9445dbc93dfdad401585c42b  0/1 in  [waiting]  · holding working tree

```

Beachten Sie, was die Sende-Quittung *nicht* gesagt hat: ein Kanal-Send meldet `spawned: true`, auch
wenn jedes Mitglied geparkt ist, denn es ist eine Quittung für die ganze Auffächerung. Die Tafel oben
ist der Ort, an dem ein Anfragender es erfährt. Volle Mechanik unter
[Grenzen und Sicherheit](nxc-limits-and-safety).

## Vorbedingungen: was gelten muss, bevor ein Schritt startet

```yaml
- name: coding
  members: [coder, finisher]
  flow: sequential
  working_tree: exclusive
  preconditions:
    - name: remote-nicht-voraus
      run: git rev-list --count HEAD..@{u}
      expect: "0"
    - name: baum-sauber
      run: git status --porcelain
      expect: ""
```

Eine Vorbedingung ist eine Hürde, die der Betreuer **vor jeden Schritt dieses Kanals** stellt. Sie
läuft in der Arbeitskopie, in der auch die Sitzungen laufen, und zwar *bevor* die Sitzung des
Schritts gestartet wird — nach dem Start ist die Sitzung da, und was sie vorhatte, ist es auch.

**Eine Hürde ist genommen, wenn ihr Befehl mit Null endet** — und, wenn die Deklaration ein
`expect:` nennt, wenn seine getrimmte Standardausgabe genau das ist. Lass `expect:` weg für einen
Befehl, der schon selbst ein Prädikat ist (`test ! -f .nxs/build.lock`); schreib `expect: ""` für
einen, der nichts ausgeben darf. Der Rückgabewert wird immer geprüft, also wird ein Befehl, der
scheitert *und* zufällig die erwartete Zeichenkette ausgibt, trotzdem abgelehnt: `git rev-list
--count HEAD..@{u}` scheitert ohne konfiguriertes Upstream und gibt nichts aus, und das als „null
Commits zurück" zu lesen wäre genau die falsche Antwort.

**Alles Übrige lehnt ebenfalls ab.** Ein Befehl, den es nicht gibt, der abstürzt oder der nicht
innerhalb seiner Frist fertig wird, hält den Schritt an — und eine Hürde, die in ihre Frist läuft,
wird *mitsamt allem getötet, was sie gestartet hat* (sie läuft in einer eigenen Prozessgruppe), damit
nichts von ihr in der Arbeitskopie zurückbleibt, die der nächste Schritt haben will. Eine Hürde, die bei Zweifel durchlässt, ist
keine — und eine Laufzeit, die überhaupt keine Befehle ausführen kann, lehnt jede deklarierte Hürde
ab, statt sie zu überspringen. Dieselbe Regel, von der anderen Seite gesehen.

Die Hürden werden in ihrer Deklarationsreihenfolge gefragt, und die **erste Absage gewinnt** —
danach wird nichts mehr ausgeführt, und was zurückkommt, nennt genau eine Hürde und zeigt ihre
Ausgabe. Wo die Absage auftaucht, hängt davon ab, welchen Schritt sie angehalten hat: ein `nxc send
--to <kanal>`, dessen erster Schritt nicht öffnen kann, scheitert unmittelbar und nennt die Hürde;
ein späterer Schritt im Ablauf wird in den `warnings` derjenigen Antwort gemeldet, die ihn gestartet
hätte — unter der Klasse `precondition_refused`, ausdrücklich verschieden von `step_skipped`, das
heißt: etwas ist kaputt, nicht eine Regel hat Nein gesagt. Einen abgelehnten Schritt wiederholt
nichts von selbst: eine Hürde lehnt ab, weil die Welt noch nicht so weit ist, und noch einmal zu
fragen ist Dein Zug.

**Was Du nicht deklarieren musst.** Manches gilt, ob Du etwas schreibst oder nicht, weil es eine
Eigenschaft des Bausteins ist und keine Deines Projekts — vor allem, dass keine Sitzung eines
früheren Schritts desselben Kanals noch läuft, wenn der nächste starten soll, auf einem Kanal, der
die Arbeitskopie beansprucht. Das ist hier bewusst nicht ausdrückbar: eine Regel, an die man denken
muss, ist eine Regel, die die Hälfte der Arbeitsbereiche nicht hat.

**Was es kostet, ausgesprochen:** ein Prozessstart je deklarierter Hürde je Schritt. Ein Quorum aus
vier Prüfern mit zwei Hürden sind acht zusätzliche Prozesse je Runde. Halte Hürden bei schnellen,
nur lesenden Fragen; sie laufen innerhalb des Aufrufs, der den Schritt gestartet hätte, und solange
sie laufen, kann niemand sonst den Arbeitsbereich benutzen.

**Deshalb teilen sich die Hürden eines Schritts EIN Zeitbudget** — eine Minute, alle zusammen. Es
wird vor jeder Hürde geprüft, eine Hürde wird also nie mehr *gestartet*, wenn das Budget verbraucht
ist; sie kommt als abgelehnt zurück und sagt, dass die vor ihr es aufgebraucht haben. Genau deshalb
gibt es keine Obergrenze für die Zahl der Hürden: zu begrenzen war nie die Anzahl, sondern die
Zeit.

**Und sie sind mit dem Rest der Deklaration eingefroren.** Eine Hürde ist ausführbarer Code in einer
Datei, die die deklarierten Agenten selbst ändern dürfen — deshalb prüft ein Vorgang gegen die
Deklarationen, unter denen er *geöffnet* wurde: `preconditions:` mitten in einer laufenden Kette zu
ändern, schaltet sie für diese Kette nicht ab, und die Änderung wirkt im nächsten Vorgang. Das ist
ebenso eine Grenze wie eine Zusage — wer zwischen zwei Vorgänge schlüpft, wird davon nicht gefasst.

## Vordertüren

```yaml
- name: front-desk
  kind: public
  members: [triage]
```

`kind:` ist `group` (die Vorgabe) oder `public`. Ein öffentlicher Kanal ist die Vordertür des
Projekts: seine Nachrichten und Tafeln sind ohne Mitgliedschaft lesbar, und er ist über Workspaces
hinweg auffindbar, die sich einen Sync-Stream teilen. Es ist eine **Leseöffnung und sonst nichts** —
die mitgliedschaftsbezogenen Auflistungen bleiben Mitgliedern vorbehalten, und Schreiben war für
keine Art je mitgliedschaftsgeprüft.

`direct` ist bewusst keine deklarierbare Art: ein direktes Gespräch wird aus zwei Handles *abgeleitet*
und für Sie geprägt, es zu deklarieren wäre also nichts, was man erfüllen könnte — und ließe man das
Wort durch, erzeugte ein Tippfehler still einen Gruppenkanal.

## Was aus `nxc workflow` wurde

Es gab eine Verbgruppe `workflow` — `start`, `step`, `status`, `tick`, `done`, `bind`, `append`,
`show`, `expect` — mit einem Lauf-Datensatz dahinter. Das alles ist weg, und diese Seite ist ihr
Ersatz, nicht ihre Dokumentation.

Der Grund ist ein Satz aus dem Entwurf: *Kanal = Arbeitsablauf. Ein „echter" Workflow ist derselbe
Begriff mit fester Reihenfolge und definierter Schreibberechtigung. Es gibt keinen zweiten Begriff
dafür.* Eine feste Reihenfolge aus einem Rollenschritt gefolgt von einem Kanalschritt — die Form, die
jeder deklarierte Workflow tatsächlich hatte — ist `flow: sequential`. Und „definierte
Schreibberechtigung" brauchte gar kein Feld: jeder Faden hat genau zwei Enden, nur der Öffner eines
Fadens darf dessen Erwartung neu deklarieren, und die Betreuer-Identität ist als Klasse reserviert.
Wer was wo schreiben darf, klärt damit das Datenmodell statt einer Deklaration, die ihm widersprechen
könnte.

Die Zuordnung lautet also:

| War | Ist |
| --- | --- |
| `workflow start` | `nxc send --to <deklarierter Kanal>` |
| `workflow step done` | `nxc reply --thread <platz>` — der Betreuer entscheidet, was folgt |
| `workflow status` / `list` | `nxc status --thread <id>` oder blankes `nxc status` |
| `workflow expect` | das deklarierte `expects:` des Kanals |
| `workflow tickets add` | `--ref nxf_ids=<id>` am `send`, das die Runde eröffnet |

## Weiter

- [Deklarationen schreiben](nxc-writing-declarations) — wann eine Runde trägt und wann sie
  verdünnt, was eine `description` beantworten muss und was ein `summary_prompt` verlangen muss.
- [Personas](nxc-personas) — die Mitglieder, die diese Kanäle benennen.
- [Befehle](nxc-commands) — die Verben, die sie ansprechen.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — Tiefe, Ansprüche und das menschliche Tor.
