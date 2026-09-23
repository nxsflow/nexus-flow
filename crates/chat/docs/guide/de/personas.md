# Personas

Eine **Persona** ist ein Agent, deklariert in einer Datei: `.nxs-personas/<handle>.yaml`. `nxc` liest
diesen Ordner; niemand außer Ihnen schreibt hinein. Es gibt kein `register`-Verb, keine mitgelieferten
Personas und keinen Weg, zur Laufzeit eine herbeizuzaubern — und das ist der Punkt. Ein deklarierter
Agent ist einer, den Sie lesen, diffen, prüfen und einchecken können, und einmal eingecheckt ist er
morgen noch da. **Checken Sie ihn ein** — siehe
[der Ordner gehört in die Versionsverwaltung](#der-ordner-gehort-in-die-versionsverwaltung); das ist
kein Ordnungshinweis.

Das Minimum sind zwei Zeilen:

```yaml
handle: coder
system_prompt: |
  You are the coder. Do what the trigger message asks; an answer from you means it is done.
```

Alles andere hat einen Vorgabewert, der bedeutet, was er bedeutete, bevor es das Feld gab — eine
Deklaration geht also nie dadurch kaputt, dass sie stehen bleibt.

## Wer sie ist

```yaml
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
expected_output: A short report of what changed, and the branch it is on.
```

`handle` ist die Adresse — das, was Sie hinter `send --to` tippen. Konventionsgemäß ist es auch der
Dateiname ohne Endung, verbindlich ist aber das YAML-Feld.

`job_title` und `job_description` sind keine Zierde. Sie sind das, was `nxc list` einem Menschen
zeigt, der entscheidet, wen er anspricht, und das, was die Persona selbst zum Sitzungsstart erfährt.
Ein Handle ohne Beschreibung ist ein Name in einem Verzeichnis, aus dem niemand auswählen kann — der
Einzeiler leistet, was die Frontmatter eines Skills leistet, und ist einen Satz Nachdenken wert.

`expected_output` ist die Form der Antwort, die Sie zurück haben wollen. Sie erreicht den
Identitätsblock der Persona und ist damit der billigste Weg, die Antworten mehrerer Agenten
vergleichbar zu machen.

Ein Handle darf nicht mit `__` beginnen. Dieses Präfix ist den engineeigenen Identitäten vorbehalten
(der Betreuer eines Kanals ist `__channel__`), und genau das macht sie durch eine Deklaration
unfälschbar.

## Wofür sie da ist

`system_prompt` ist die Aufgabe der Persona, in Ihren eigenen Worten, und das Letzte, was das Modell
vor dem eigentlichen Gespräch liest. Der zusammengesetzte Prompt ist geschichtet, in dieser
Reihenfolge:

1. **Der prime-Block** — das, was `nxs prime --persona <handle>` zusammensetzt; Sie können ihn
   ausgeben und selbst lesen. Es ist der Sitzungsstart der Suite, in Modulreihenfolge
   zusammengesetzt: das Brett (`nxf prime`), die Erinnerungen des Projekts (`nxm prime`), dann der
   eigene Block von chat — die Identität dieser Persona (Titel, Aufgabe, erwartete Ausgabe,
   Erfahrungsstufe, wie sie erreichbar ist), wen sie ansprechen darf und wofür, die
   `nxc`-Befehlsliste und ihre Antwortregeln. Er kommt zuerst, damit „wer bin ich und wie antworte
   ich" alles Folgende verankert.
2. **Die `CLAUDE.md` des Projekts**, wenn die `claude_md:`-Politik der Persona danach verlangt.
3. **Der eigene `system_prompt` der Persona** — die Rolle, in Ihren Worten.
4. **Die Aufgabe, die der Schritt erklärt hat**, wenn diese Persona von einem Schritt eines
   Kanalablaufs gerufen wurde, der ein `task:` trägt (`nxc guide channels`). Sie steht unter der
   Rolle, weil sie die engere von beiden ist und weil sie das benennt, was die Rolle nicht wissen
   kann: welche der mehreren Aufgaben, denen sie dient, diese Station ist. Sie kommt unter einer
   eigenen Überschrift, damit das Modell sie von der Rolle unterscheiden kann, und sie **ergänzt** —
   sie kann die Persona weder ersetzen noch Teile davon abschalten, und der Prompt sagt das in einem
   eigenen Satz der Engine. Ein Schritt ohne `task:` setzt genau die drei Ebenen darüber zusammen,
   Byte für Byte.

Die Antwortregeln aus Schicht 1 sollte man wörtlich nehmen, denn es gibt genau **zwei** davon und
keine dritte: antworte in dem Faden, den man dir gegeben hat — oder sag, im selben Faden, dass du
es nicht kannst. Beides ist `nxc reply --thread <id>`, und **beides beendet den Zug.** Es gibt
keinen Weg, zu sagen, was fehlt, und danach weiterzuarbeiten: man antwortet, der Zug endet, und
solange die Runde offen ist, setzt die Antwort einen fort — mit allem, was man schon weiß. (Ein
Ende hat gar keine *Antwort* in sich: das Warten auf eine selbst beauftragte Runde, also den Zug
beenden und nichts sagen. Es ist keine dritte Antwortregel, und das erzwungene Ende weiter unten
schreibt es aus.) Ist eine
Runde beantwortet und weitergereicht, ist sie geschlossen: eine Antwort in einen ihrer Fäden wird
abgelehnt und sagt das auch. Das Nächste beginnt dann mit `send --to`, womit man auch etwas Neues
mit jemand anderem anfängt; `nxc list` zeigt, wer da ist.

Drei Schalter formen diese Schichten:

```yaml
prime: true            # Vorgabe. false steigt aus Schicht 1 aus, wenn der Prompt sie selbst abdeckt
claude_md: inherit     # Vorgabe | ignore (Projektkonventionen nicht zeigen) | override (reserviert)
base_prompt: claude_code   # Vorgabe. `none` läuft ohne das Claude-Code-Preset darunter
```

`prime:` nimmt auch eine Abbildung, wenn eine Persona einen Teil der Suite bekommen soll und den
Rest nicht:

```yaml
prime:
  flow: false          # kein Brett für DIESE Persona — ein reiner Prüfer braucht keines
  memory: true         # die Erinnerungen des Projekts (die Vorgabe)
  chat: true           # Identität, Adressbuch und Antwortregeln (die Vorgabe)
```

Was Sie weglassen, bleibt an — die Abbildung oben sagt also eine Sache und ändert eine Sache. Der
Filter gilt je Persona: das Brett hier auszuschließen entfernt den Abschnitt aus dem Block **dieser**
Persona und aus keinem anderen.

> **`memory: false` kann mehr kosten als Erinnerungen.** Wenn dieses Projekt `nxm migrate` gelaufen
> ist, sind seine Konventionen AUS der `CLAUDE.md` heraus in den Speicher gewandert — genau das tut
> die Migration —, die Projektregeln kommen also aus `nxm prime`, und `claude_md: inherit` trägt
> nichts mehr bei. Eine Persona, die in einem solchen Projekt die Erinnerungen ausschließt, hat die
> Regeln von nirgendwoher. Die Engine kann ein migriertes Projekt nicht von einem nicht migrierten
> unterscheiden und weist die Deklaration deshalb nicht ab; sie sagt es stattdessen der Sitzung, in
> einer Zeile ihres eigenen Prompts: sie hat beides nicht und sollte `nxm prime` lesen, bevor sie
> etwas ändert, bei dem sie sich nicht sicher ist.

Weil Schicht 1 die Antwortschleife bereits beibringt, kann eine Anstoßnachricht nur die Aufgabe
tragen. Genau deshalb ist das Feld standardmäßig an.

**Einen Satz können Sie nicht abschalten.** Sobald der Anstoß, der eine Sitzung startet, erklärt
hat, dass ein Faden auf ihre Antwort wartet, trägt der zusammengesetzte Prompt das *erzwungene
Ende* — und zwar auch unter `prime: false`:

```text
Obligation: thread <id> is waiting on your reply, and your turn may not end without one.
Give the message on STDIN, so that nothing in it is evaluated by the shell — the quotes
around EOF are what stop that:

nxc reply --thread <id> - <<'EOF'
<your message>
EOF

There are exactly two ways to end the turn, and they are that same command with one flag
added or left off:
- no flag — you are finished.
- `--escalate` — you cannot reach the result and need help or a decision.
A ONE-LINE answer may be an argument instead: `nxc reply --thread <id> --escalate "no
test workspace here"`. Never for longer text — backticks, `$(…)` and quotes in it are
rewritten by the shell before nxc sees them.
While this is the only conversation open for you, `--thread` may be left out: the same
command without it means this one.
```

Diese Formen sind die ganze Menge der ANTWORTEN, und der Absatz nennt anschließend das eine Ende,
in dem keine Antwort steckt: Eine Sitzung, die auf eine selbst beauftragte Runde wartet, beendet
ihren Zug und sagt nichts. Sie wird geweckt, wenn die Antwort da ist, und ein blankes `nxc reply`,
solange die eigene Runde offen ist, wird abgelehnt — es hieße „ich bin fertig". Die Regel steht mit
Absicht in der Engine und nicht in Ihrer Deklaration — ein Schritt eines deklarierten Ablaufs endet,
wenn seine Sitzung antwortet, und eine Deklaration, die das zu sagen vergisst, ließe den Ablauf ohne
Signal zurück.

**Einem Schritt, der Arbeit zurückschicken kann, wird eine dritte Form genannt**, und nur einem
solchen: wo der Kanal für ihn `on_needs_rework:` erklärt (`nxc guide channels`), bietet derselbe
Absatz zusätzlich `--needs-rework` als dritte Flagge an derselben einen Zeile an. Das
Angebot hängt am erzwungenen Ende und nicht am Nutzungsblock — aus dem Grund darüber: eine Rolle mit
`prime: false` ist weiterhin zur Antwort verpflichtet, und eine Verpflichtung, deren Mittel niemand
sicherstellt, ist genau der Fehler, den diese Regel verhindert.

## Womit sie läuft

```yaml
stage: senior          # junior | senior | principal
model: opus            # fable | opus | sonnet — schlägt `stage`, wenn beides dasteht
tools: [Bash, Read, Write]
permissions: acceptEdits
```

**`stage` ist ein Vokabular, das kein Modellname ist.** Sie entscheiden, wie viel Denken eine Aufgabe
wert ist, ohne wissen zu müssen, welche Modelle es diesen Monat gibt: `junior` → Sonnet, `senior` →
Opus, `principal` → Fable. Eine Tabelle verbindet beides, damit die Schreibweise der Deklaration und
die Wahl der Engine nicht auseinanderlaufen können. Greifen Sie zu `model:` nur, wenn Sie etwas
meinen, das die Stufe nicht ausdrücken kann — und beachten Sie die bewusste Vorrangregel: die Stufe
ist die grobe, besprechbare Wahl, ein benanntes Modell ist Ihre Übersteuerung.

**Ein Schritt eines Kanalablaufs darf die Stufe heben oder senken** (`stage:` am Schritt, `nxc guide
channels`) — dieselbe prüfende Persona beurteilt an der einen Station einen Diff und an der anderen
einen Entwurf, und das Zweite ist die teurere Denkarbeit. Er bewegt die Stufe und sonst nichts: Ein
Schritt darf kein Modell benennen, und eine Persona mit eigenem `model:` läuft darauf, was ein
Schritt auch verlangt.

`tools` hat drei Zustände, und der Unterschied zählt. Lassen Sie das Feld ganz weg, gilt der volle
Standardwerkzeugsatz der Agenten-Laufzeit. Schreiben Sie `tools: []`, hat die Persona ausdrücklich
keine — eine enge, nicht-agentische Rolle. Schreiben Sie eine Liste, bekommt sie genau diese. Ein
weggelassener Schlüssel und eine leere Liste sind *nicht* dasselbe.

Eine Persona, die überhaupt `nxc` ausführen soll, braucht `Bash`, denn so antwortet sie — und wo die
Engine eine Antwort *verlangt*, gibt sie das selbst dazu. Jede Beauftragung sagt der Persona in ihrem
eigenen Systemprompt, sie solle ihren Zug mit `nxc reply --thread <id>` beenden; wer so verpflichtet,
muss also sicherstellen, dass es ausführbar ist. Der Trigger nimmt `Bash` zusätzlich zu dem auf, was
Sie deklariert haben — ohne den Werkzeugsatz zu verengen, den ein weggelassenes `tools:` gewährt. Für
eine Persona, die die Shell für ihre *Arbeit* braucht, deklarieren Sie `Bash` weiterhin selbst; was
Sie nicht mehr mitdenken müssen, ist, dass Antworten auch Arbeit ist.

## Wer sie ansprechen darf

```yaml
addressable: general        # die Vorgabe: jeder darf direkt ein Gespräch eröffnen
addressable: none           # niemand darf — komm über einen Kanal, der mich besetzt
addressable:                # genau diese Aufrufer, und sonst niemand
  personas: [pm]
  humans: true
```

**Die Abbildung ist eine Erlaubnisliste, und eine weggelassene Hälfte nennt niemanden dieser
Klasse.** Genau das macht die beiden Fälle, die in der Praxis auftreten, zu *einer* Form:

- `{humans: true}` — der Mensch am Terminal darf direkt schreiben, keine Persona. Die Form für
  einen PM, den Sie selbst ansprechen können wollen, während jeder Agent durch die Runde geht, die
  er führt; der Satz in seinem Prompt („dieser Kanal ist der einzige Weg, auf dem dich jemand
  erreicht") wird damit eine Zusicherung statt einer Konvention.
- `{personas: [head-of-marketing]}` — genau ein benannter Kollege darf diese Persona einzeln
  ansprechen, ein Mensch spricht den Kanal an. Der Fachpersona-Fall, andersherum.

`{}` sagt dasselbe wie `none`. Und `addressable: [review]` — eine Liste von Kanalnamen — wird
weiterhin gelesen und heißt weiterhin „niemand direkt", ist aber **veraltet**: in welchen Kanälen
eine Persona mitwirkt, ist Sache des Kanals (`nxc guide channels`), und es hier noch einmal
hinzuschreiben ist eine Tatsache an zwei Stellen. Schreiben Sie `none`; `nxs prime` sagt es Ihnen
ebenfalls.

### Warum eine aufruferabhängige Schranke hier zulässig ist

Die Regel, auf der diese Oberfläche steht, lautet: das Ablegen der Identität darf nie *mehr*
erlauben — deshalb ist das Adressbuch weiter unten Orientierung und nie Durchsetzung. Die
Unterscheidung, die eine frühere Fassung dieser Seite zu weit gezogen hat:

- `general` und `none` lesen sich für jeden Aufrufer gleich, genau wie bisher.
- Eine Erlaubnisliste kann immer nur jemanden **abweisen**, den `general` zugelassen hätte.
  Anonymität löst sich zu „ein Mensch" auf; bei `{personas: [...]}` gewährt das Ablegen der
  Identität also strikt *weniger* — die Richtung, die die Regel verlangt.
- Bei `{humans: true}` gewährt es *mehr*, und das ist der ehrliche Preis der zweiten Richtung.
  Was ihn begrenzt: `nxc send` hat kein `--persona`. Die Aufruferklasse kommt aus dem
  Sitzungsstempel, den dieser Arbeitsbereich selbst vergeben hat, und eine Persona, die ihn ablegt,
  um durchzukommen, hört für diesen Aufruf auf, sie selbst zu sein — keine Rückadresse, kein
  Fortsetzen, nichts, dem die Antwort zugerechnet würde. Sie bekommt nicht dasselbe plus, sondern
  etwas Schlechteres.

So oder so ist das **Korrektheit, keine Verteidigung**, wie jede Prüfung in `nxc`: auf einer
Maschine ist keine Schranke hier stärker als der Zugriff auf die Arbeitsbereichsdatei (`nxc guide
limits-and-safety`). Was sie einbringt, ist, dass eine Deklaration bedeutet, was sie sagt.

### Was das für `nxc list` heißt

Eine Persona, die der Leser nicht direkt ansprechen darf, ist kein eigener Eintrag — der Kanal, der
sie besetzt, ist es. Das ist meist gewollt: wenn der Weg zu vier Reviewern darin besteht, die Runde
anzusprechen, in der sie sitzen, lädt eine Einzelauflistung genau zu dem Aufruf ein, den Sie nicht
wollen. Die Darstellung folgt dem *Leser*, dieselbe Deklaration kann eine Persona Ihnen also zeigen
und einem Agenten verbergen.

## Wen sie ansprechen darf

```yaml
address_book:
  - to: review
    why: to get a change judged before merging
  - to: pm
    why: to report that a work order is done
```

Das **`why` ist die tragende Hälfte** — es wirkt wie die einzeilige Beschreibung eines Skills, ein
Satz, aus dem ein Modell auswählt, und deshalb wird es überall gerendert, wo auch das Ziel steht.

**Es hat drei Zustände, genau wie `tools:`, und der Unterschied zählt.**

```yaml
# Schlüssel fehlt        -> „nicht aufgeschrieben": die Persona sieht das ganze deklarierte Team
address_book: []         # -> „beauftragt nichts": die Persona sieht niemanden
address_book: [{to: pm}] # -> genau dieses Buch, in der Reihenfolge des Autors
```

`address_book: []` ist die Deklaration für eine Rolle am **Blatt** des Baums: ein reiner Prüfer, ein
Zusammenfasser, alles, dessen einziger ausgehender Ruf die Antwort auf dem eigenen Faden ist. Dass es
in der Datei steht, ist der Punkt — vorher blieb dafür nur ein Satz im `system_prompt` („du
beauftragst nichts; ignoriere die Liste"), also ein Prompt, der gegen eine Deklaration anredet.

Ein **fehlender** Schlüssel bleibt „nicht aufgeschrieben" und zeigt das ganze Team; jede Persona, die
vor dieser Änderung deklariert wurde, bedeutet also unverändert das, was sie immer bedeutet hat.

Eines lässt die abgeleitete Liste weg: **die Kanäle, in denen die Persona selbst Mitglied ist.** Einem
Prüfer die `review`-Runde anzubieten, in der er sitzt, hilft in keiner Lesart — und den eigenen Kanal
zu beauftragen ist keine Deklarations-Zyklizität, die wird beim Laden abgewiesen, also fiele es sonst
durch jede Prüfung. Ein Buch, das Sie selbst schreiben, wird Eintrag für Eintrag befolgt, diesen
Kanal eingeschlossen: die Datei ist die Autorität.

Beachten Sie, was das Adressbuch in dieser Stufe bewusst *nicht* tut: Es schränkt nicht ein. Es ist
Orientierung, die eine Persona liest — aus demselben Grund wie oben: eine Schranke aus einer
ablegbaren Identität abzuleiten, wäre schlimmer als nutzlos.

## Wie sie die Antworten erfährt, die sie beauftragt hat

Eine Persona wird nie zum Pollen aufgefordert. Wenn sie Arbeit hinausgibt und ihr eigener Zug endet,
hält der Koordinator fest, was zurückkommt, und startet die Sitzung mit **allem auf einmal** wieder:
eine Kopfzeile mit der Anzahl, ein abgegrenzter Block je Nachricht mit dem Absender und dem Faden,
den sie beantwortet, und eine Zeile, was noch aussteht. Das ist dieselbe Form, in der die Antworten
einer Kanalrunde ankommen — wer die eine lesen kann, kann auch die andere lesen.

Zwei Dinge über diese Zustellung sollte man wissen, bevor man den Prompt einer Persona schreibt:

- **Eine Rückgabe steht abgesetzt.** „Ich brauche Hilfe oder eine Entscheidung" stoppt eine Kette
  und wird deshalb nie in den Stapel gemischt: sie kommt zuerst, unter ihrem eigenen Hinweis, vor
  den gewöhnlichen Antworten.
- **„Alles, was eintraf" ist nicht „alle Antworten".** Zwei von drei antworten vielleicht in
  Sekunden und die dritte in zwanzig Minuten. Die Zustellung benennt, welche Aufträge noch
  ausstehen, mit Faden und Handle — die Persona muss nicht selbst mitzählen.

Konnte sie niemand wecken — die Sitzung wurde abgeschossen, oder die Maschine war aus —, geht nichts
verloren: ihr nächster **Sitzungsstart** benennt die Aufträge, die fertig wurden, während sie weg war.
Diese Meldung ist ein Fenster, keine Schuld: sie umfasst, was seit dem Ende der vorigen Sitzung
dieser Persona fertig wurde, wird also einmal gesehen und muss nicht quittiert werden. `nxc status`
beantwortet dieselbe Frage jederzeit.

## Was sie zum Arbeiten braucht

```yaml
working_tree: exclusive        # shared (Vorgabe) | exclusive
```

`exclusive` sagt, dass die Sitzungen dieser Persona die Arbeitskopie und das Build-Verzeichnis des
Repositories allein brauchen. Eine zweite Kette, die kollidieren würde, wartet stattdessen. Das wird
deklariert und nicht erschlossen, denn eine Persona darf heißen, wie sie will, und die Laufzeit kann
einer Aufgabe nicht ansehen, dass gleich ein Checkout verzweigt wird. Die vollen Mechanik — was ein
Anspruch umfasst, was ihn freigibt und wovor er nicht schützt — steht unter
[Grenzen und Sicherheit](nxc-limits-and-safety).

## Wo sie läuft

```yaml
machine: studio                # eine Maschinen-Kennung oder ein Name aus `nxs sync machines`; Standard: wo der Chat beginnt
```

Ein Chat mit dieser Persona läuft auf genau **einer** Maschine. Jede andere Maschine, die den
Arbeitsbereich synchronisiert, sieht den Chat und startet nichts. Die Maschine wird festgelegt, wenn
der Chat beginnt, in dieser Reihenfolge: `nxc send --to <persona> --machine <m>` für diesen einen
Chat, dann dieses `machine:`, dann die Maschine, die den Chat beginnt. Die Antwort steht im Chat
selbst, sodass jede Maschine dieselbe liest, und ein späteres
`nxc reply --thread <id> --machine <m>` übergibt den Chat an eine andere Maschine.

Ist die Maschine, auf der ein Chat laufen würde, **nicht online**, wird nichts gesendet, sondern
nachgefragt: Die Ablehnung nennt die Maschinen, die online sind, und eine davon mit `--machine` zu
nennen ist die Antwort. Der Dienst der bestimmten Maschine nimmt den Chat etwa eine halbe Minute nach
dem Schreiben auf, aber nur von einer Maschine, deren Schlüssel er vertraut
(`nxs sync trust add`): Ein Auftrag von anderswo bleibt im Faden sichtbar und startet nichts.

Das alles gilt für einen Arbeitsbereich, der mit anderen Maschinen synchronisiert
(`nxs sync bind`). Einer, der nirgends synchronisiert, hat eine einzige Maschine; dort wird nichts
festgelegt, und ein Chat beginnt, wo er geschrieben wird.

`machine:` wird nur gelesen, wenn ein Mensch einen Chat beginnt. Ein Chat, den eine laufende Persona
beginnt, läuft auf deren Maschine, weil die Antwort zu der Sitzung zurückkommen muss, die gefragt
hat. Eine Persona, die als Mitglied eines Kanals beauftragt wird, läuft dort, wo der Kanal begonnen
wurde.

## Ein ausgearbeitetes Beispiel

```yaml
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
working_tree: exclusive
system_prompt: |
  You are the coder. Implement the work order in the message you were handed: on a branch of
  its own, with the project's own gates green before you merge it.

  An answer from you means the branch is merged and those gates were green on the tree that was
  merged. If you cannot get there — something is missing, or a decision came up that is not
  yours to make — say what you are missing in your thread rather than answering.
tools: [Bash, Read, Write]
permissions: acceptEdits
```

**Drei Dinge in diesem Prompt lohnt es sich zu übernehmen, und eine Leerstelle trägt alle drei.** Er
benennt eine ROLLE und nie eine Position — nichts darin behauptet, der erste Schritt von irgendetwas
zu sein, also stimmt dieselbe Datei auch an dem Tag noch, an dem der Kanal einen Schritt dazubekommt,
und eine Persona, die ihre Position behauptet, hat sie falsch (gemessen: eine sprach von drei
Schritten, es waren vier). Er sagt, was *hier* als FERTIG zählt — der eine Teil der Antwortschleife,
der Ihnen gehört. Und er benennt, was der Coder zurückbringen muss, nicht, wie er es verschickt.

Die Leerstelle ist der Punkt. Kein Abschnitt „How you finish", kein `nxc`-Aufruf, kein Heredoc: die
Engine schreibt das erzwungene Ende in jeden Zug, der eine Antwort schuldet, mitsamt der echten
Faden-ID und jedem Weg, den Zug zu beenden — siehe „Einen Satz können Sie nicht abschalten" weiter
oben. Eine Kopie hier sagte dasselbe mit einem Platzhalter dort, wo die Wahrheit steht, und sagte es
weiter, wenn die Engine längst weiter ist. `nxs prime` meldet eine solche Kopie als
**Deklarationswarnung**, und `nxc guide writing-declarations` ist das Kapitel, das diese Fehlerklasse
und sechs weitere durcharbeitet.

## Deklariert, aber noch nicht gelesen

Vertrauenswürdige Dokumentation sagt, welche Felder wirkungslos sind. Diese werden geparst,
zurückgeschrieben und validiert, und nichts in der Engine handelt heute danach:

- **`session: fresh | continue`** — wirkungslos, weil **nicht die Persona es entscheidet**. `nxc
  send --to <persona>` prägt eine frische Sitzung; eine `nxc reply --thread` in den eigenen Faden
  dieser Persona setzt die Sitzung fort, die sie bereits hat — mit allem, was sie bereits weiß; und
  in einem Kanal mit `steps:` entscheidet der betretene Schritt es mit seinem eigenen `resume:`
  (`nxc guide channels`). Eine Politik auf der Persona hat nichts mehr zu entscheiden, was das Verb
  oder der Schritt nicht schon entschieden hat.
- **`sub_agents: true | false`**.
- **`reports_to: <handle>`**.

Sie zu deklarieren kostet nichts und hält die Absicht fest; bauen Sie keinen Prozess darauf, dass sie
etwas tun.

**Bei `claude_md: override` ist Vorsicht geboten**, denn es ist nur ZUR HÄLFTE ungelesen. Das damit
gemeinte Ersatzdokument je Persona ist noch nicht spezifiziert, also setzt nichts eines zusammen —
untätig ist die Angabe deswegen aber nicht: sie ist schlicht nicht `inherit`, also wird eine
Persona, die sie deklariert, OHNE die `CLAUDE.md` des Projekts zusammengesetzt, genau wie bei
`ignore`. Schreiben Sie `ignore`, wenn Sie das meinen. `inherit` (die Vorgabe) und `ignore` sind
beide wirksam.

## Der Ordner gehört in die Versionsverwaltung

`.nxs-personas/` ist keine Dokumentation darüber, wie Ihre Agenten arbeiten — er *ist*, wie sie
arbeiten, bei jedem Start frisch gelesen. Und er liegt in derselben Arbeitskopie, in der die Agenten
selbst arbeiten sollen. Das ist eine Rückkopplung, die keine andere Konfiguration in diesem System
hat: ein `git switch`, ein `git stash` oder ein `git checkout -- .` einer Persona ändert, wie die
**nächste** Persona denkt.

Eine nicht eingecheckte Änderung an einer Deklaration ist deshalb gar keine richtige Änderung. Sie
ist etwas, das ein Zweig hat und jeder andere nicht — in einem Ordner, dessen Inhalt die Regeln
festlegt. Und was sie zurückdreht, ist gewöhnliche, korrekte Zweighygiene von jemandem, der nicht
wissen kann, dass Ihre Datei wichtig war. Das ist keine Vermutung, sondern das, was hier passiert
ist: drei Deklarationen wurden genau auf diesem Weg zurückgedreht, darunter eine Regel, die einer
Persona sagte, sie solle jeden Zug mit einer Antwort beenden. Die Persona lief danach ohne sie, und
auf vier Fäden antwortete die Laufzeit an ihrer Stelle, bevor es jemandem auffiel. Gefunden wurde es
von Hand, durch einen Vergleich der gespeicherten Sitzungs-Spec mit der Datei auf der Platte.

Checken Sie den Ordner ein und prüfen Sie Änderungen daran wie Code. Zwei Dinge helfen Ihnen zu
bemerken, wenn es doch passiert ist, und keines davon ist eine Sperre — die Dateien bleiben jederzeit
editierbar, denn stabil sein muss ein laufender **Vorgang**, nicht das Verzeichnis:

- Jede gestartete Sitzung hält fest, aus welcher Fassung der Deklaration ihr Prompt gebaut wurde: als
  `declarationHash` in `.nxs/agent-logs/<session>.spec.json`. „Lief diese Sitzung unter der Regel,
  die ich geschrieben habe?" ist damit ein Vergleich und keine Textsuche in einem Prompt.
- Wird eine Persona unter einer anderen Deklaration gestartet als beim letzten Mal, sagt das der
  Empfangsschein des Aufrufs, der sie gestartet hat — ein `declaration_changed`-Eintrag in
  `warnings`, der beide Fassungen nennt. Es ist eine **Warnung und keine Verweigerung**: eine Persona
  zu ändern und sie dann anzusprechen ist die gewöhnliche Arbeitsweise, und der Exit-Code bleibt `0`.
  Eine Änderung ist meistens gewollt. Eine unbemerkte nie.

Beides beobachtet **die Datei der Persona selbst** und sonst nichts in diesem Ordner. Eine
`channels.yaml`, die auf demselben Weg zurückgedreht wurde — andere Mitglieder, ein anderes
`working_tree:`, ein anderes `timeout:` — steuert Ihre Agenten genauso und wird von beidem **nicht**
gemeldet. Der Rat oben heißt deshalb nicht „das Werkzeug passt schon auf": es ist die
Versionsverwaltung, die aufpasst, und dies ist ein zweites Paar Augen auf der Hälfte des Ordners, die
es sehen kann.

## Wenn eine Deklaration falsch ist

Eine fehlerhafte Datei ist ein lauter `validation`-Fehler, der den Pfad nennt — nie eine still
übersprungene Persona. Ein fehlender Ordner ist *kein* Fehler: ein Workspace ohne Deklarationen löst
sauber auf und hat schlicht niemanden anzusprechen, und `nxc list` und `nxs prime` sagen genau das.

Referenzielle Probleme — ein Kanal, der eine nicht existierende Persona nennt; eine Persona, die sich
über einen Kanal erreichbar erklärt, in dem sie kein Mitglied ist — erscheinen in `nxs prime`, und
zwar nur im interaktiven Kontext. Einer gestarteten Persona werden die Fehler ihres Autors nicht
vorgehalten; einem Menschen an der Tastatur schon.

**Qualitätswarnungen kommen an derselben Stelle an, nach derselben Regel.** Eine Deklaration, die
Text kopiert, den die Engine ohnehin einspielt, oder deren `job_description` einem Rufer nicht sagen
kann, wann er ruft, ist nicht kaputt — deshalb ist sie eine *Warnung*: Sie wird dem Menschen
aufgelistet, sie schließt nichts aus, und die Deklaration lädt und läuft genau wie sonst. Was
geprüft wird, was bewusst nicht, und die vier Fehlerklassen, die keine Prüfung entscheiden kann,
stehen in [Deklarationen schreiben](nxc-writing-declarations).

## Weiter

- [Deklarationen schreiben](nxc-writing-declarations) — wie Sie diese Felder füllen, damit die
  Antworten brauchbar sind: sieben gemessene Fehlerklassen und eine Checkliste.
- [Kanäle](nxc-channels) — mehrere Personas zusammen arbeiten lassen.
- [Befehle](nxc-commands) — ansprechen, was Sie gerade deklariert haben.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — die Kappen rund um eine laufende Persona.
