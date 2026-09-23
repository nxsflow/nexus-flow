# Grenzen und Sicherheit

Jeder Agent, den chat startet, ist ein echter Prozess und ein bezahlter Modellaufruf, und jeder von
ihnen kann weitere starten. Diese Seite ist die Sammlung der Bremsen darum herum — und, mindestens
ebenso wichtig, die Liste dessen, was ein Agent **nicht** selbst entscheiden darf.

Lesen Sie sie als Entwurfsaussage, nicht als Konfigurationsreferenz. Fast nichts hier ist einstellbar,
und das mit Absicht.

## Was ein Agent nicht entscheidet

Die Oberfläche wurde bewusst so geschnitten, dass ein laufender Agent keine Fragen beantworten kann,
die ihm niemand gestellt hat. Diese entscheidet eine **Deklaration**, die Sie geschrieben haben, und
es gibt keine Übersteuerung je Aufruf:

| Frage | Wo sie beantwortet wird |
| --- | --- |
| Welches Modell, und wie viel Denken ist das wert? | `stage:` oder `model:` der Persona |
| Wie lange darf eine Runde warten? | `timeout:` des Kanals |
| Wer ist in dieser Runde, und in welcher Reihenfolge? | `steps:` des Kanals (oder `members:` und `flow:`) |
| Wer darf wen ansprechen? | `addressable:` der Persona |
| Braucht das die Arbeitskopie allein? | `working_tree:` an Persona oder Kanal |
| Was geschieht mit den Antworten? | `on_complete:` und `visibility:` des Kanals |

`send --model`, `send --deadline`, `send --kind`, `send --priority`, `send --disposition` und `reply
--kind` gab es alle, und alle wurden entfernt. Die angegebene Begründung ist es wert, wiederholt zu
werden, denn sie ist die Regel hinter der ganzen Tabelle: *sonst wird ein Agent vielleicht übereifrig
Optionen ändern.* Eine Übersteuerung je Aufruf neben einem deklarierten Wert sind zwei Antworten auf
eine Frage — und ausgerechnet der Aufrufer ist die Partei mit der geringsten Legitimation zu
antworten.

Was ein Agent sehr wohl entscheidet, ist, was er sagt, wem von den Deklarierten er es sagt, ob er die
Aufgabe überhaupt ausführen kann — und, wo ein Schritt des Kanals einen Weg dafür erklärt, ob die
Arbeit, die er beurteilen sollte, lieferbar ist (`--needs-rework`).

## Die Tiefenbegrenzung

Eine Startkette ist bei **32 Sprüngen** gedeckelt. Darüber hinaus wird ein Anstoß abgelehnt, bevor
irgendetwas getan wird — und genau das hindert zwei Agenten, die sich Arbeit nur gegenseitig
zuschieben, daran, ewig zu kreisen, wobei jede Runde einen Prozess und einen Modellaufruf kostet.

Drei Einzelheiten machen daraus eine echte Bremse statt einer Formalie:

**Gezählt wird die OFFENE Kette**, nicht jeder Sprung, an dem eine Sitzung je beteiligt war. Arbeit,
die beauftragt und *beantwortet* wurde, wickelt sich ab: das Ziel tritt wieder in der Tiefe ein, die
es ohnehin hatte, statt eine unter der Sitzung, die geantwortet hat. Das ist keine Nebensache — ein
roher Zähler tötete genau die Form, die er schützen sollte: ein Orchestrator und ein Kanalbesitzer,
die über viele Runden reden, gewannen vier Sprünge pro Runde und waren nach sieben dauerhaft nicht
mehr aufrufbar. Beauftragen geht abwärts, Antworten kommt zurück.

**Sie ist im Speicher verankert, nicht in der Umgebung.** Die Tiefe einer Sitzung wird von dem
geschrieben, der sie gestartet hat, bevor sie je lief. `NXC_HOP` in der eigenen Umgebung eines
Prozesses ist ein *Anspruch*, der die Tiefe anheben darf und nie senken. Diesen Unterschied gibt es,
weil jede Persona in den ausgelieferten Beispielen `tools: [Bash]` deklariert — `unset NXC_HOP` vor
ihrem eigenen `nxc send` hätte die Kette sonst bei jedem Sprung auf null zurückgesetzt und die
Bremse damit ausgehebelt.

**Was sie nicht schließt, klar gesagt.** Ein Aufrufer, der gar keine Sitzung vorweist, löst auf seinen
beanspruchten Wert auf, denn ein echter Mensch am Terminal hat genau diese Form. Ein Agent, der
entschlossen ist zu entkommen, kann also seine ganze Umgebungsidentität ablegen und wie eine frische
Kette aussehen — um den Preis von allem, was diese Identität ihm einbringt: keine Rückadresse auf
seinen Posts, keine Wiederaufnahme, keine Sitzung, der sich irgendetwas zurechnen ließe. Diese letzte
Lücke zu schließen braucht ein fälschungssicheres Merkmal, das dem gestarteten Prozess übergeben
wird, statt einer Umgebungsvariablen — das ist ein Zugriffsentwurf, nicht diese Bremse.

## Das menschliche Tor

**Kein Mensch steht mitten im Ablauf.** Ein Mensch eröffnet einen Vorgang und ein Mensch liest sein
Ergebnis; nichts dazwischen wartet auf einen. Deshalb ist `awaiting_human` in `nxc status` nur an der
**Wurzel** eines abgeschlossenen Vorgangs wahr, nie an einem Faden in der Mitte. Ein
abgeschlossener Vorgang steht gar nicht in der Standardauflistung — `--all` ist der Ort dafür:

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

Das Merkmal ist eine Aussage über ein Ende der Kette, neben dem Zustand des Fadens statt an seiner
Stelle, und es gibt es aus einem Grund: ein Vorgang, der **stehengeblieben** ist, und einer, der
**fertig ist und auf Sie wartet**, sehen für einen Zähler gleich aus und verlangen Gegenteiliges.
Wenn Sie eine Übersicht über `nxc status --json` bauen, ist das das Feld, an dem Sie verzweigen.

Das andere menschliche Tor schreiben Sie selbst. Eine Persona, die ohne Freigabe nicht handeln darf,
ist eine Persona, deren `system_prompt` das sagt und deren Deklaration ihr keine Werkzeuge zum
Handeln gibt — `tools: []` ist ein echter, eigener Zustand und nicht dasselbe wie den Schlüssel
wegzulassen.

## Eskalation: das deklarierte „ich brauche Hilfe oder eine Entscheidung"

Ein Agent darf mit `reply` genau zwei Dinge sagen: *ich bin fertig* und *allein komme ich nicht ans
Ergebnis*. Das zweite ist `--escalate`, und es ist ein **deklariertes** Signal, keine Formulierung:

```bash
nxc reply --thread <id> --escalate "Die Migration braucht ein Produktionsgeheimnis, das ich nicht habe."
```

Das zählt, weil der Betreuer an einem Bit verzweigt, dessen Bedeutung aufgeschrieben ist, und nicht
an einem Wort, das ein Agent erfunden hat und kein Leser aufzählen kann. Der Zug ist so oder so
eingelöst — die Sitzung endet und die Schuld mit ihr. Verschieden ist das Ergebnis, und der Kanal
entscheidet, was daraus wird: eine eskalierende Runde wird durchgereicht statt gefaltet, damit die
Rückgabe den Anfragenden erreicht, wie sie dasteht, und nie als Absatz *über* ein Scheitern.

**„Ich kann nicht" ist nur die halbe Bedeutung, und die andere Hälfte wird übersehen.** Eine
Sitzung, die ihre Aufgabe sehr wohl ausführen kann, aber auf eine Frage gestoßen ist, die nicht ihr
gehört, gehört ebenfalls hierher — das meint „eine Entscheidung brauchen". Der Grund ist die
Richtung: eine Eskalation geht NACH OBEN, zu dem, der Sie beauftragt hat. Dieselbe Frage in einer
blanken Antwort geht stattdessen NACH UNTEN zum nächsten Schritt, der sie so wenig entscheiden darf
wie Sie, und die Arbeit läuft dann um eine offene Frage herum weiter. Gemessen am 12.09.2026: ein
PM beendete seinen Zug mit einer blanken Antwort und zwei offenen Produktfragen im Text; drei
Prüfrunden beurteilten einen Entwurf, dessen zentrale Entscheidung noch offen war, und die Fragen
erreichten den Eigentümer erst, als die Durchlaufgrenze ablief.

Es ist kein Papierkorb für alles, was kein Ergebnis ist. Und es ist heute zugleich der einzige Weg,
überhaupt etwas zurückzugeben: `reply --kind question` fiel mit dem übrigen `--kind` weg, eine echte
Rückfrage wird also entweder mit `--escalate` gesagt oder gestellt, indem man ein eigenes Gespräch
eröffnet. Das ist ein festgehaltener Verlust, kein Zusammenfallen.

**Warten ist keine Eskalation, und genau das hält das Signal lesbar.** Eine Sitzung, die eine eigene
Runde beauftragt hat und auf sie wartet, beendet ihren Zug und sagt gar nichts — siehe
[channels](nxc-channels). Eine Eskalation in einem Vorgang heißt deshalb genau eines: Eine Kette
STEHT, und jemand muss entscheiden, wie es weitergeht. Gibt man das Verb zweimal für den Normalfall
aus, bedeutet der Alarm nichts mehr; ein blankes `nxc reply` bei offener eigener Runde wird aus
demselben Grund abgelehnt — es behauptete ein Ergebnis, das noch nicht vorliegt.

## Der Anspruch auf die Arbeitskopie

Ein Repository hat eine Arbeitskopie und ein Build-Verzeichnis. Zwei Ketten, die darin gleichzeitig
verzweigen, committen und `cargo test` laufen lassen, sind kein Wettlauf, den man durch Sorgfalt
gewinnt — deshalb nimmt chat einen Anspruch.

Deklarieren Sie den Bedarf (`working_tree: exclusive` an einer Persona oder einem Kanal), und ein
Anstoß, der kollidieren würde, **wartet** statt zu kollidieren. Die Mechanik, in der Reihenfolge, in
der sie greift:

- **Was ein Anspruch umfasst, ist die OPERATION**: die Wurzel des Fadenbaums, zu dem die Arbeit
  gehört, und alles, was in beliebiger Tiefe darunter hängt — dieselbe Einheit, nach der `nxc status`
  gruppiert. Der Anspruch *entsteht* weiterhin erst beim ersten exklusiven Schritt, eine Kette, die
  nie einen erreicht, hält also nichts; ist er aber einmal genommen, wird er zurückgegeben, wenn die
  **Operation** fertig ist, und nicht schon, wenn die Runde fertig ist, die ihn genommen hat. Genau
  das schliesst die Lücke zwischen zwei Runden desselben Epics: ein Planer, der Coding zweimal
  beauftragt, behält die Kopie über die Pause dazwischen, statt zuzusehen, wie eine fremde Kette
  hineinrutscht.

  Diese Seite sagte das Gegenteil — der Anspruch hörte vor Ihrem Gespräch mit dem Agenten auf, damit
  nicht „ein einziger unbeantworteter Mensch die Arbeitskopie der Maschine bis morgen hält". Diese
  Grenze kostete mehr, als sie einbrachte, und das Risiko, das sie benannte, beantwortet jetzt die
  Frist weiter unten, statt das obere Ende der Operation ungeschützt zu lassen.
- **Erwerben oder anstellen.** Ein Anstoß innerhalb eines gehaltenen Gebiets **erbt** den Anspruch,
  statt sich anzustellen — sonst stellte sich ein verschachtelter exklusiver Schritt hinter seinem
  eigenen Elter an, das nie freigeben könnte, weil es auf genau den Anstoß wartet, den es gerade
  geparkt hat. Die Warteschlange ist Wer-zuerst-kommt, und eine Freigabe befördert das ganze
  Anspruchsgebiet an der Spitze, nie einen einzelnen Eintrag: mehrere Mitglieder einer Auffächerung
  teilen sich einen Anspruch, und eines zu starten, während seine Geschwister geparkt bleiben,
  verklemmt die Tafel.
- **Was ihn freigibt.** Eine Antwort innerhalb des Gebiets gibt den Anspruch frei, wenn nichts in
  diesem Gebiet noch eine Antwort schuldet *und* das letzte Wort keine Rückgabe war. Eine
  **Eskalation hält den Anspruch** — die Frage wandert noch nach oben, die Aufgabe ist noch in der
  Luft, und die Kopie in genau diesem Moment einem Rivalen zu überlassen, wäre genau falsch.
- **Wie sich der Halt löst.** Eine Rückgabe **in einem Kanal** zu beantworten deklariert die Runde
  neu, und der Halt ist weg. Diese Seite sagte das ohne den Zusatz, und der Zusatz ist der ganze
  Punkt: auf einem **direkten** Faden stimmt es nicht. Eine Antwort zählt dort nur als Antwort auf
  eine Rückgabe, wenn sie von einem Handle kommt, das der Faden erwartet — und auf einem eskalierten
  Faden ist das erwartete Handle der Eskalierende selbst. Eine Antwort von jemand anderem, auch vom
  Eröffner, wird gepostet und bewegt nichts. Zweimal gemessen: `posted: true`, `woke: null`, und der
  Anspruch danach unverändert `holding`.
- **Eine unbeantwortete Eskalation hält die Warteschlange nicht ewig.** Wenn — und erst wenn — **ein
  anderer Vorgang wartet**, läuft eine Frist von 30 Minuten. Verstreicht sie, wird die Arbeit dieses
  Vorgangs auf einen Zweig committet (unversionierte Dateien eingeschlossen; was Ihre `.gitignore`
  ausschließt, bleibt im Baum liegen), der Baum kehrt auf den Ursprungszweig dieses Vorgangs zurück,
  und der wartende Vorgang startet. Kommt die Antwort später, wird der Vorgang auf seinen eigenen
  Zweig zurückgestellt und erfährt in einem festen Text, auf welchem Zweig und welchem Commit seine
  Arbeit liegt — und ob sich der Zweig **oder seine Basis** inzwischen weiterbewegt hat, was Sie sonst
  nie bemerken würden.

  Zweierlei tut das bewusst nicht. **Die Frist beginnt bei Kontention, nicht beim Eskalieren**: steht
  niemand an, kostet es niemanden etwas, also wird nichts gemessen und nie geparkt — das ist nicht der
  normale Ausgang einer Eskalation. Und **ein Baum mitten in Rebase, Merge, Cherry-Pick oder Bisect
  wird nie geparkt**: einen konfliktbehafteten Index zu committen hieße, einen halbfertigen Merge als
  die Arbeit festzuhalten.

  **Wird ein Parken abgelehnt, entscheidet eine Regel, was mit der Arbeitskopie geschieht.** Eine
  Ablehnung, die vorübergehen kann — ein laufender Ablauf oder ein fehlgeschlagener git-Befehl —
  **hält** sie: die `warnings` des Ticks sagen `work_not_parked` und nennen, was zu beenden ist,
  `nxc status` markiert den Vorgang mit denselben Worten als `PARK REFUSED`, und der
  Hintergrunddienst versucht es bei jedem Tick erneut, sodass die Kopie von selbst weitergeht, sobald
  der Baum in Ordnung ist. Eine Ablehnung, die nicht vorübergehen kann — die Laufzeit nennt keine
  Arbeitskopie, das Verzeichnis ist kein git-Repository, oder für den Vorgang wurde nie ein
  Basiszweig vermerkt — **gibt die Arbeitskopie ungeparkt weiter**, als `work_handed_on_unparked`:
  was dieser Vorgang nicht committet hatte, liegt weiter im Baum, jetzt in den Händen des
  nächsten. Die Warteschlange für eine Antwort festzuhalten, die kein Warten ändert, hieße, sie für
  immer festzuhalten.

  Die Markierung steht so lange, wie jemand es erneut versucht, und keinen Tick länger: Vergeht der
  Grund, aus dem die Kopie gebraucht wurde, zuerst — die Eskalation ist beantwortet und der Vorgang
  arbeitet weiter, der angehaltene Vorgang wird aufgenommen, niemand wartet mehr —, verschwindet die
  Zeile `PARK REFUSED` beim nächsten Tick, auch wenn der Baum noch mitten im Merge steckt. Niemand
  versucht dieses Parken, also sagt es auch niemand.
- **Eine Kopie, die eine Kette hält, die Sie gestartet haben und zurückwollen, während sie noch
  läuft: `nxc withdraw --thread <id>`.** Nennen Sie den ersten Faden des Vorgangs — den, den Ihr
  `send` Ihnen zurückgegeben hat; `nxc status` führt den Vorgang, der die Kopie hält, unter dieser
  id und markiert ihn mit `holds working tree` —, und der Aufruf entlastet die Runde, bittet ihre
  Sitzung, sich zu beenden, unterbricht die Kette darunter und parkt, sobald der Prozess weg ist,
  was sie unfertig hinterlassen hat, auf einem Zweig und gibt die Kopie weiter; die nächste
  Beauftragung in denselben Faden bringt die Arbeit zurück. Das darf nur der Mensch, der diese
  erste Beauftragung abgeschickt hat: Eine Sitzung, die dieser Workspace gestartet hat, wird
  abgelehnt, und ein Agent, der eine Runde anhalten will, eskaliert stattdessen. Eine Kopie, die
  eine bereits tote Kette hält, braucht niemanden, der fragt: der Sweep weiter unten nimmt sie von
  selbst — über ihrer Frist, oder, weil er eine tote Kette nachweisen kann, schon davor.
- **Wie lange ein Anspruch läuft: so lange, wie die Operation es selbst zugesagt hat.** Die Frist ist
  das späteste `timeout:`, das im Anspruchsgebiet noch aussteht — eine Runde, die sechs Stunden sagt,
  hält die Kopie sechs Stunden, und eine Operation, deren Schritte *alle* Minuten sagen, ist nach
  einem Absturz in Minuten wieder frei. Das „alle" trägt: **eine einzige** ausstehende Verpflichtung
  ohne Fenster — und ein Gespräch, das Sie mit einem schlichten `nxc send` eröffnen, ist eine — setzt
  die ganze Operation auf die pauschale **Zwei-Stunden-Grenze** zurück, die früher für jeden Anspruch
  galt. Die zwei Stunden sind also jetzt die Antwort für Deklarationen, die nichts sagen — kein
  Budget, das denen aufgezwungen wird, die etwas sagen —, und für eine kürzere Frist entscheidet man
  sich, indem man `timeout:` entlang der Kette deklariert.
- **Die Rückfallgrenze.** Ein verwaister Anspruch hört auf, *künftige* Erwerbungen abzulehnen, sobald
  seine Frist verstrichen ist, und ein `tick` nach diesem Punkt gibt die Warteschlange frei — eine
  Beauftragung, die parkt, plant einen für genau diesen Zeitpunkt ein, das Leeren passiert also auch,
  wenn niemand da ist. (Diese Seite sagte, die Warteschlange werde überhaupt nie geleert: nichts
  fragte ab, ein bereits geparkter Anstoß wartete unbegrenzt über die Grenze hinaus. Jetzt fragt
  etwas ab.)

  **Und dieses Leeren parkt zuerst die Arbeit des toten Vorgangs**, genauso wie es bei einer
  unbeantworteten Eskalation geschieht: Die nicht committeten Dateien kommen auf einen Zweig, die
  Arbeitskopie kehrt auf den Zweig zurück, auf dem sie begonnen hat, und erst dann startet die
  wartende Beauftragung. Eine abgestürzte Kette hat weder eskaliert noch eine Grenze angekündigt,
  das ist also das einzige Parken, das sie je bekommen kann — bisher bekam sie gar keines, und der
  nächste Agent lief in ihre halbfertige Arbeit hinein. Die Ablehnungsregel von oben gilt auch hier:
  Ein Baum mitten im Merge **behält** die Arbeitskopie und wird mit `PARK REFUSED` markiert, bis Sie
  den Merge beenden oder abbrechen — **oder bis Sie die Beauftragung zurückziehen, die auf die Kopie
  wartet** (`nxc withdraw --thread <id>`), was den Anlass beendet, statt ihn zu beheben, und der
  einzige Ausweg ist, wenn die Ablehnung ein fehlgeschlagener git-Befehl ist und kein Merge, den Sie
  sehen und beenden können.
- **Ein toter Vorgang darf die Kopie nicht bis zum Ablauf seiner Frist behalten.** Solange jemand
  hinter einem Anspruch wartet, sieht ein Tick einmal pro Minute nach — nicht einmal zur Frist — und
  fragt, ob der Halter *nachweislich* fort ist. Ist er es, wird die Kopie sofort übernommen, mit
  zuvor geparkter Arbeit, genau wie beim Leeren oben. In der Regel also innerhalb einer Minute nach
  dem Tod statt bis zu zwei Stunden später.

  „Nachweislich" ist bewusst streng, denn eine Arbeitskopie zu übernehmen schreibt in sie hinein. All
  das muss zutreffen: Der Chat-Worker dieses Arbeitsbereichs kann tatsächlich beantworten, ob der
  Prozess einer Sitzung lebt (kann er es nicht, wird nie vorzeitig etwas übernommen — `nxc status`
  schließt dann mit dem Hinweis *this workspace's runtime cannot say whether a session's process is
  alive*, und `nxc status --json` führt die Antwort in jedem Fall als `worker_answers_liveness`);
  mindestens ein Thread des Vorgangs schuldet noch eine Antwort; kein Prozess des Vorgangs lebt; keine
  Sitzung eines schuldenden Threads hat ein Ende gemeldet (ein gemeldetes Ende ist ein geordneter
  Abbau, kein Absturz); und nichts im Vorgang pausiert an einer Verfügbarkeitsgrenze, die ihren
  eigenen Weg zurück hat. Ein Vorgang, dessen Sitzungen alle endeten und die Aufgabe zurückgegeben
  haben, fällt **nicht** darunter — er wartet auf Sie, und für ihn gilt weiterhin die
  Dreißig-Minuten-Regel von oben.
- **Wer schon wartet, kommt zuerst dran.** Das Zurückholen eines abgelaufenen Anspruchs gibt den Zug
  der Warteschlange im selben Schritt weiter: die Kette, die schon anstand, startet, und der
  Neuankömmling stellt sich hinten an. Diese Seite schwieg dazu, weil es umgekehrt war — die Kopie
  bekam, wer als Nächstes *fragte*, ganz gleich, wie lange ein Rivale schon darauf gewartet hatte.

  **Auch diese Übergabe parkt zuerst die Arbeit der toten Kette**, nach derselben Regel wie das
  Leeren oben: Es ist derselbe Anlass, nur durch die andere Tür erreicht, und oft ist es das eigene
  `send` eines Neuankömmlings, das vor jedem Tick dort ankommt. Wird dieses Parken also aus einem
  Grund abgelehnt, der vorübergehen kann, bekommt **niemand** die Kopie: Der Neuankömmling stellt
  sich schlicht hinter den Halter, genau wie er es bei noch laufendem Anspruch getan hätte, und
  weiterbewegen wird sie der erneute Versuch des Hintergrunddienstes.

  **Geparkt wird auch, wenn niemand sonst wartet** — der schlichteste Fall überhaupt: Sie lassen
  einen Agenten laufen, er stürzt mit Arbeit im Baum ab, und Stunden später übernimmt Ihr nächstes
  `nxc send` die Kopie. Dann gibt es keinen Zug weiterzugeben, aber die Kopie verlässt trotzdem einen
  Vorgang, der verunglückt ist: Seine Arbeit kommt auf einen Zweig, und die neue Sitzung startet in
  einem sauberen Baum. Der Hintergrund-Sweep macht das nicht — bei leerer Warteschlange gibt er
  nichts weiter, also ist dort auch nichts zu sichern.
- **Eine abgelaufene Frist genügt nicht, um die Kopie zu nehmen.** Eine Kette, deren Anspruch
  abgelaufen ist, deren **Prozess aber noch lebt**, behält sie — dieselbe Ablehnung, die die
  Prüfung auf eine nachweislich tote Kette oben für einen Halter noch innerhalb seiner Frist
  ausspricht, hier nur auf der anderen Seite davon gefragt. Diese Seite warnte, ein einzelner Schritt, der
  länger als zwei Stunden arbeitet, verliere den Anspruch unter sich und die nächste Kette laufe in
  die Arbeitskopie hinein, in der er noch baut. Das gilt nicht mehr: was ein Rivale antrifft, ist ein
  lebender Prozess, keine Frist.

Nur ein beauftragender Anstoß wird je geparkt. Ein Anstoß, der ein *Ergebnis* nach oben trägt, wird
nie angestellt — sonst würde ausgerechnet die Antwort zurückgehalten, deren Ankunft den Anspruch
freigibt.

## Was eine Zugübergabe über die Arbeitskopie festhält

Jedes `nxc send` und jedes `nxc reply` hält fest, **wo die Arbeitskopie in diesem Moment stand** —
den Commit, auf den `HEAD` zeigte, den Zweig, auf dem er stand, ob etwas nicht eingecheckt war, und
einen Fingerabdruck, der den ganzen Zustand später vergleichbar macht. Das steht an der Nachricht
selbst und kommt darum überall zurück, wo die Nachricht zurückkommt.

`nxc threads show` zeigt es unter der Nachricht, über die es eine Aussage macht — `working copy:
9f2c1ab4e7d0 on feat/parser · uncommitted work NOT saved anywhere` —, und `nxc status` dieselben
Tatsachen als Zusatz an der Zeile des Fadens: `· tree 9f2c1ab4e7d0 on feat/parser, uncommitted work
NOT saved`. In `--json` sind es `refs.working_copy` an einer Nachricht und `working_copy` an einer
Statuszeile, mit den Feldern `commit`, `branch`, `dirty` und `fingerprint` — verzweigen Sie über die
Felder, nie über den Text.

Wofür es gut ist, in der Reihenfolge des Nutzens: eine Operation fortsetzen, die an einer Stelle
unterbrochen wurde, die niemand notiert hat; im Nachhinein nachvollziehen, **welchen Stand des Baums
ein Prüfer eigentlich vor sich hatte**; und den Commit benennen, auf den zurückgespult wird.

**Drei ehrliche Grenzen, und es sind Grenzen von git, nicht dieser Funktion.**

- **Ein schmutziger Baum wird notiert, nicht gesichert.** Ein Agent übergibt mitten in der Arbeit —
  das ist der Normalfall —, und dann benennt der Commit, *wovon* die Arbeit ausging, nicht, wie der
  Baum aussieht. Der Eintrag sagt das mit genau diesen Worten (`uncommitted work NOT saved
  anywhere`), denn die gegenteilige Annahme ist die, die ein Leser macht. Eine Übergabe committet
  nichts: Zweig, `HEAD` und ungestagte Änderungen sind danach genau, wie Sie sie verlassen haben.
- **Was git nicht sieht, sieht auch das nicht** — vor allem ignorierte Dateien. Eine `.env`, eine
  lokale Datenbank, `node_modules`: keines davon steckt im Fingerabdruck, eine Änderung allein darin
  liest sich also als keine Änderung.
- **Das Repo spult zurück, die Aufzeichnung nicht.** Ein Anker benennt einen Commit für die
  *Arbeitskopie*. Er spult nicht das Brett zurück (angelegte und geschlossene `nxf`-Vorgänge
  bleiben), nicht den Kanal (Nachrichten sind absichtlich anhängend), und nichts, was die Maschine
  verlassen hat — einen Pull Request, ein Release, eine Mail. `.nxs/` selbst liegt ausdrücklich
  außerhalb: die Aufzeichnung mit dem Code zurückzuspulen würde den Beleg dafür, was geschehen ist,
  zusammen mit der Arbeit vernichten.

Ein Workspace, dessen Laufzeit ihre Sitzungen nicht in einem Verzeichnis startet, das dieser Prozess
sehen kann, hält davon nichts fest. `nxc status` sagt das einmal in einer abschließenden
`note:`-Zeile, statt Ihnen eine leere Spalte zu hinterlassen; `--json` antwortet darauf mit
`worker_names_a_working_copy: false` am Bericht.

## Inhalte, die Sie nicht geschrieben haben

Zwei Stellen bringen Text, den jemand anders geschrieben hat, in den Kontext eines Modells, und beide
sollte man kennen, bevor man entscheidet, was die eigenen Agenten dürfen.

**Die Durchreichung eines Kanals** rendert die rohe Antwort jedes Mitglieds in die Nachricht, die die
Sitzung des Anfragenden erhält. Die Form sagt das: die Antworten stecken in einem ausdrücklich als
nicht vertrauenswürdig markierten Element, und die Lieferung nennt die Grenze, mit der sie umschlossen
wurden — je Runde so gewählt, dass keine Antwort sie enthält. Genau das macht die Zuschreibung
`from="…"` zur eigenen Aufzeichnung der Engine statt zu etwas, das eine Antwort fälschen könnte.
Diese Aufzeichnung sagt, unter **welchem Handle** eine Nachricht gepostet wurde — sie ist kein
Nachweis, dass der Absender zu diesem Handle berechtigt war, und erst recht keiner dafür, dass das
Geschriebene stimmt. Behandeln Sie die Worte eines
Mitglieds als Daten, nie als Anweisungen — und wenn nicht alle Mitglieder einer Runde Ihre eigenen
sind, sagen Sie das im Prompt der anfragenden Persona, denn das ist die Schicht, die zuerst gelesen
wird.

**Ein Verlauf** enthält rohe Werkzeug-Eingaben und -Ergebnisse — alles, was der Agent gelesen,
geschrieben oder ausgeführt hat. Also:

- Behandeln Sie einen Verlaufsauszug wie die Workspace-Datenbank, nicht wie ein
  Nachrichtenprotokoll. Fügen Sie keinen ungelesen in einen Fehlerbericht ein.
- `nxc transcript show` ist bewusst **nicht** mitgliedschaftsgeprüft. Ein Verlauf hat keinen Kanal,
  an dem man prüfen könnte, und die Tabelle ist gerätelokal und wird nie synchronisiert — eine
  Prüfung erkaufte also nichts, was wer die Datei hat nicht ohnehin mit `sqlite3` täte. Es ist eine
  Betreiberoberfläche; die Dateirechte sind die Grenze.

## Was Sync für Vertraulichkeit bedeutet

In dieser Stufe ist Sync **vollständig und ungefiltert**: ein an einen Stream gebundener Workspace
tauscht das ganze Log aus, ohne Autorisierung je Kanal auf der Leitung. Wer den Stream erreicht, kann
die Nachrichten darin lesen.

Also: keine Zugangsdaten und keine Geheimnisse in Nachrichtentexte, und behandeln Sie
Kanalmitgliedschaft als Konvention darüber, *wer gefragt wird*, nicht als Grenze darüber, *wer lesen
kann*. Verläufe sind das eine, das die Maschine nie verlässt.

### Was stattdessen gilt

Ein Geheimnis gehört in den Geheimnisspeicher der Plattform, und die Nachricht nennt höchstens den
Ort.

- **Zur Laufzeit holt die Persona es selbst** — aus dem Geheimnismechanismus des Wirts (ein
  SSM-Parameter, eine App-Einstellung mit `secret: true`, der Schlüsselbund des Betriebssystems, eine
  Datei, die der Prozess lesen darf). `nxc` muss davon nichts wissen, und das ist die richtige
  Teilung: das Nachrichtenlog hält fest, was gesagt wurde — und ein Zugangsdatum war nie etwas, das
  jemand gesagt hat.
- **In der Nachricht steht der ORT, nie der Wert** — der Parametername, der Einstellungsschlüssel,
  der Pfad. `der DIP-Schlüssel liegt unter .nxf/secrets/dip-api-key` darf synchronisiert werden; der
  Schlüssel nicht.
- **Ist der Ort eine Datei, halten Sie sie aus dem Log und aus git heraus** — Rechte `600`, und von
  `.gitignore` erfasst.

Es gibt bewusst keinen geheimnistragenden Kanal und keine verschlüsselte Nachrichtenart. Beides würde
Zugangsdaten in genau das Log legen, um das es in diesem Abschnitt geht, und jeder Verbraucher dieses
Logs würde sie erben.

## Hinweise, die Hinweise sind

Zwei Dinge in `nxc` lehnen bewusst nicht ab, und es lohnt sich zu wissen, welche:

- **Ein `send`, das keinen Gegenstand nennt**, postet trotzdem. Die Nachricht ist gepostet mehr wert
  als verloren. Aber die Quittung trägt `refs_warning` als Feld — nicht nur als Zeile auf stderr —,
  damit auch eine App den Hinweis sieht und ein Aufrufer am Wert verzweigen kann, statt Prosa zu
  parsen.
- **Ein Anstoß, der scheiterte, nachdem die Nachricht bereits geschrieben war**, verwirft die Quittung
  nicht. Das Feld `warnings` an `send`, `reply` und `tick` ist immer vorhanden, auch leer, und jeder
  Eintrag trägt eine maschinenlesbare Klasse:
  - `step_skipped` — ein Schritt, dessen *Anstoß* scheiterte. Die Nachricht liegt dauerhaft im Kanal
    und die Schuld ist registriert, aber niemand arbeitet daran. **Nichts wiederholt das.**
  - `step_unanswered` — ein Schritt, der gestartet wurde und nie antwortete; sein Fenster lief ab, und
    was folgte, geschah ohne ihn.
  - `requester_not_woken` — eine Tafel wurde vollständig, die Antwort liegt da, und die Sitzung, die
    gefragt hatte, wurde nie benachrichtigt.
  - `tick_unscheduled` — gerade beobachtet niemand ein deklariertes `timeout:`; dieses Fenster hat
    also keine Uhr: nichts lässt die Tafel von selbst `stale` werden, und ein Mitglied, das
    verstummt, hält die Runde unbegrenzt an. Der Aufruf selbst ist geglückt — die Tafel ist offen und
    ihre Mitglieder laufen —, weshalb diese eine Klasse den Exit-Code **nicht** von Null verschieden
    macht. `nxc tick --thread <id>` ist dieselbe Prüfung, von Hand aufrufbar. Die Frist ist ohnehin
    eingetragen, ein später gestarteter Dienst hält sie also nach. Die Meldung sagt, welche der
    beiden Hälften fehlt: dieser Arbeitsbereich ist beim nexus-flow-Hintergrunddienst nicht
    angemeldet, oder es läuft kein Dienst. Wie man ihn einrichtet, steht bei den
    [Kanälen](nxc-channels).
  - `service_not_running` — dieselbe Tatsache eine Ebene höher, und die einzige, die kein Fenster
    braucht: **es gibt niemanden, der irgendetwas davon ausführt.** Ein Hintergrunddienst hält jede
    Frist dieser Maschine *und* ist das Einzige, was synchronisiert. Solange er steht, feuern
    deklarierte Fenster nicht — eine Runde, die sich lösen sollte, hängt unbegrenzt — und das
    Ops-Log dieser Maschine schiebt und holt nichts mehr. Beide Hälften stehen in der Meldung, samt
    dem, was zu tun ist. Der Eintrag reitet auf jedem `send`, `reply` und `tick` mit, solange das so
    ist, und ändert wie `tick_unscheduled` den Exit-Code **nicht**: Ihr Aufruf hat alles getan, was
    er sollte; die Maschine nicht. Sie sehen ihn nur in einem Arbeitsbereich, der beim Dienst
    angemeldet ist — einer, der es nie war, verhält sich genau wie vereinbart und wird nie wegen
    eines Dienstes gewarnt, um den er nicht gebeten hat.

  - `declaration_changed` — diese Persona wurde zuletzt unter einer **anderen Fassung ihrer eigenen
    Deklaration** gestartet. Nichts ist gescheitert: die Sitzung läuft, und mitgeteilt wird Ihnen,
    dass die Regeln, unter denen sie läuft, nicht die Regeln des vorigen Laufs sind.
    `.nxs-personas/` liegt in derselben Arbeitskopie, in der die Agenten selbst arbeiten — ein
    `git switch` oder `git stash` der einen Persona ändert also, wie die nächste denkt, und genau so
    wurden hier einmal drei Deklarationen still zurückgedreht. Der Eintrag nennt die Rolle und beide
    Fassungen (`declaration.hash`, `declaration.previous_hash`), und die `spec.json` jeder Sitzung
    hält fest, unter welcher Fassung sie lief — Sie können also nachsehen statt zu raten. Auch das
    ändert den Exit-Code **nicht**: eine Persona zu ändern und sie dann anzusprechen ist die
    gewöhnliche Arbeitsweise. Eine Änderung ist meistens gewollt, eine unbemerkte nie. Warum der
    Ordner in die Versionsverwaltung gehört, steht bei den [Personas](nxc-personas).

Diese sechs verlangen verschiedene Reaktionen — Technik reparieren, ein Mitglied nachfassen, eine
Sitzung neu starten, den Hintergrunddienst starten, in eine Deklaration schauen —, deshalb sind es
eigene Klassen und kein Eimer voller Prosa. Wenn Sie irgendetwas Automatisiertes auf chat bauen, ist
dieses Feld das, was Sie beobachten.

Neben der Klasse trägt jeder Eintrag ggf. einen maschinenlesbaren **`reason`** dafür, warum eine
Sitzung nicht in Bewegung gesetzt wurde. Einer davon ist neu und lohnt sich zu kennen:
**`already_running`** — das Wecken wurde abgelehnt, weil die genannte Sitzung noch einen lebenden
Prozess hat. Eine interne Sitzung führt immer genau einen Prozess; ein zweiter Start setzte zwei
Agenten in dasselbe Arbeitsverzeichnis, die einander überschreiben. Die Nachricht liegt so oder so
dauerhaft im Faden, und die laufende Sitzung darf ihren Zug zu Ende bringen.

Ein weiteres Feld gehört daneben, an `nxc status` statt an eine Quittung: **`escalated`**. Ein Faden,
dessen laufender Zug zurückgegeben wurde — ein Mitglied, das mit `nxc reply --escalate` „ich brauche
Hilfe oder eine Entscheidung" geantwortet hat, oder eine Persona-Sitzung, die *gestorben* ist und
deren Abbau die Schuld beglichen hat —, steht auf `state: "answered"` wie jeder erledigte Faden,
denn der Zug ist wirklich vorbei. `escalated: true` ist das, was die beiden unterscheidet.
Verzweigen Sie darauf überall dort, wo Sie „answered" sonst als „fertig" lesen würden.

Und eines daneben: **`substituted`** — die Laufzeit hat die neueste Antwort dieses Fadens
geschrieben, stellvertretend für den Agenten, der sie schuldete. Endet eine gestartete Sitzung, ohne
ihrem Faden geantwortet zu haben, setzt der Abbau-Block des Sidecars die Antwort in ihrem Namen, damit
der Faden nie still hängen bleibt. Das ist richtig so — und es war bisher nur daran zu erkennen, dass
man den Nachrichtentext *las*, der mit `sidecar:` beginnt. Die beiden Felder sind unabhängig
voneinander, und Sie wollen beide:

| | `escalated` | `substituted` |
|---|---|---|
| der Agent bittet um Hilfe oder eine Entscheidung | `true` | `false` |
| die Sitzung starb, der Abbau-Block sagte es | `true` | `true` |
| die Sitzung endete still, ohne zu antworten | `false` | `true` |
| der Agent hat geantwortet | `false` | `false` |

Die dritte Zeile ist die, auf die es ankommt. Es ist nichts schiefgegangen — eine Eskalation zu
melden hieße einem Vorgesetzten fälschlich zu sagen, er solle anhalten — und trotzdem hat niemand
geantwortet. Die häufigste Ursache ist eine Persona, die zu einer Antwort *verpflichtet* wurde und
sie nicht geben konnte: prüfen Sie ihr deklariertes `tools:`.

Es gibt einen vierten Fall, und er steht mit Absicht in keiner dieser Zeilen: Eine Sitzung, die ihren
Zug **wartend auf eine selbst beauftragte Runde** beendet, schreibt gar keine Nachricht. Ihr Faden
bleibt offen und weiter geschuldet, denn die Sitzung, die ihn schuldet, kommt dafür zurück;
`waiting_on_sub_round` in der `nxc status`-Zeile dieses Fadens sagt es.

## Weiter

- [Kanäle](nxc-channels) — wo Fristen und Alleinnutzung deklariert werden.
- [Personas](nxc-personas) — wo Modell und Werkzeuge stehen.
- [Befehle](nxc-commands) — die Oberfläche, die all das einhegt.
