# Personas

Eine **Persona** ist ein Agent, deklariert in einem Ordner: `.nxs-personas/<name>/SKILL.md`. `nxc`
liest diesen Ordner; niemand außer Ihnen schreibt hinein. Es gibt kein `register`-Verb, keine
mitgelieferten Personas und keinen Weg, zur Laufzeit eine herbeizuzaubern — und das ist der Punkt.
Ein deklarierter Agent ist einer, den Sie lesen, diffen, prüfen und einchecken können, und einmal
eingecheckt ist er morgen noch da. **Checken Sie ihn ein** — siehe
[der Ordner gehört in die Versionsverwaltung](#der-ordner-gehort-in-die-versionsverwaltung); das ist
kein Ordnungshinweis.

Die Datei ist ein **Skill**, in der Form der [Agent-Skills-Spezifikation](https://agentskills.io):
ein YAML-Frontmatter zwischen zwei `---`-Zeilen, dann die Anweisungen. Das Minimum sind drei Zeilen
und ein Satz:

```markdown
---
name: coder
---
You are the coder. Do what the trigger message asks; an answer from you means it is done.
```

Alles andere hat einen Vorgabewert, der bedeutet, was er bedeutete, bevor es das Feld gab — eine
Deklaration geht also nie dadurch kaputt, dass sie stehen bleibt. Die eigenen Felder der
Spezifikation stehen oben im Frontmatter; alles, was nexus-flow darüber hinaus braucht, steht unter
EINEM Schlüssel, `nxs:`. Ein Skill ganz ohne `nxs:` ist eine Persona mit lauter Vorgabewerten —
siehe [ein veröffentlichter Skill ist eine Persona](#ein-veroffentlichter-skill-ist-eine-persona).

Die ältere Form, eine `<handle>.yaml` je Persona, wird weiterhin gelesen — siehe
[die ältere YAML-Form](#die-altere-yaml-form) dafür, wie ihre Felder sich abbilden, und
`nxs personas migrate` für den Befehl, der sie umschreibt.

## Wer sie ist

```markdown
---
name: coder
description: Implements a work order on a branch and merges it.
nxs:
  title: Coder
  expected_output: A short report of what changed, and the branch it is on.
---
```

`name` ist das Handle, die Adresse — das, was Sie hinter `send --to` tippen. Konventionsgemäß ist es
auch der Name des Ordners, verbindlich ist aber das Feld. Die Agent-Skills-Spezifikation hat dafür
eine Regel: 1 bis 64 Zeichen, Kleinbuchstaben, Ziffern und Bindestriche, kein Bindestrich am Anfang
oder Ende und keine zwei hintereinander, und derselbe wie der Name des Ordners. `nxs prime` warnt,
wenn ein Name sie verletzt; nichts weist einen ab, ein Skill-Werkzeug, das die Regel prüft, aber
schon.

`description` und `nxs.title` sind keine Zierde. Sie sind das, was `nxc list` einem Menschen
zeigt, der entscheidet, wen er anspricht, und das, was die Persona selbst zum Sitzungsstart erfährt.
Ein Handle ohne Beschreibung ist ein Name in einem Verzeichnis, aus dem niemand auswählen kann — der
Einzeiler leistet, was die Frontmatter eines Skills leistet, und ist einen Satz Nachdenken wert.

`nxs.expected_output` ist die Form der Antwort, die Sie zurück haben wollen. Sie erreicht den
Identitätsblock der Persona und ist damit der billigste Weg, die Antworten mehrerer Agenten
vergleichbar zu machen.

Ein Handle darf nicht mit `__` beginnen. Dieses Präfix ist den engineeigenen Identitäten vorbehalten
(der Betreuer eines Kanals ist `__channel__`), und genau das macht sie durch eine Deklaration
unfälschbar. Und keine Persona darf `channels` heißen: so heißt der Ordner, der eine Datei je Kanal
enthält (`nxc guide channels`).

## Wofür sie da ist

Der **Rumpf** der `SKILL.md` — alles nach dem Frontmatter — ist die Aufgabe der Persona, in Ihren
eigenen Worten, und das Letzte, was das Modell vor dem eigentlichen Gespräch liest. Der
zusammengesetzte Prompt ist geschichtet, in dieser Reihenfolge:

1. **Der prime-Block** — das, was `nxs prime --persona <handle>` zusammensetzt; Sie können ihn
   ausgeben und selbst lesen. Es ist der Sitzungsstart der Suite, in Modulreihenfolge
   zusammengesetzt: das Brett (`nxf prime`), die Erinnerungen des Projekts (`nxm prime`), dann der
   eigene Block von chat — die Identität dieser Persona (Titel, Aufgabe, erwartete Ausgabe,
   Erfahrungsstufe, wie sie erreichbar ist), wen sie ansprechen darf und wofür, die
   `nxc`-Befehlsliste und ihre Antwortregeln. Er kommt zuerst, damit „wer bin ich und wie antworte
   ich" alles Folgende verankert.
2. **Die `CLAUDE.md` des Projekts**, wenn die `claude_md:`-Politik der Persona danach verlangt.
3. **Der Rumpf der `SKILL.md` der Persona** — die Rolle, in Ihren Worten, unverändert.
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

**Ein relativer Pfad im Rumpf meint den eigenen Ordner der Persona.** Das ist die Regel der Agent
Skills: Ein Skill hält `references/`, `scripts/` oder `examples/` neben seiner `SKILL.md` und sagt
„lade `references/guide.md`". Das gilt überall, wo die Persona läuft — in diesem Repository oder aus
dem Ordner auf Benutzerebene in einem fremden —, denn die Sitzung erfährt an ihrem Start, wo ihre
Deklaration liegt (die Zeile „Declared in"), und dass ein relativer Pfad diesen Ordner meint, es sei
denn, die Anweisungen sagen, ein Pfad sei relativ zum Repository. Das Arbeitsverzeichnis der Sitzung
ist weiterhin das Repository, in dem sie arbeitet. (Die ältere YAML-Form behält die umgekehrte
Regel: Ein relativer Pfad meint dort das Repository.)

Ein Skill-Rumpf ist oft als Anweisungen geschrieben — „wenn X, tu Y" — statt als Rolle. Das trägt,
denn Identität und Antwortregeln spielt die Engine in Schicht 1 selbst ein.

Drei Schalter formen diese Schichten, alle unter `nxs:`:

```yaml
nxs:
  prime: true            # Vorgabe. false steigt aus Schicht 1 aus, wenn der Rumpf sie selbst abdeckt
  claude_md: inherit     # Vorgabe | ignore (Projektkonventionen nicht zeigen) | override (reserviert)
  base_prompt: claude_code   # Vorgabe. `none` läuft ohne das Claude-Code-Preset darunter
```

`prime:` nimmt auch eine Abbildung, wenn eine Persona einen Teil der Suite bekommen soll und den
Rest nicht:

```yaml
nxs:
  prime:
    flow: false          # kein Brett für DIESE Persona — ein reiner Prüfer braucht keines
    memory: true         # die Erinnerungen des Projekts (die Vorgabe)
    chat: true           # Identität, ihr Verzeichnis und Antwortregeln (die Vorgabe)
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

```markdown
---
name: coder
allowed-tools: Bash Read Write
nxs:
  stage: senior          # junior | senior | principal
  model: opus            # fable | opus | sonnet — schlägt `stage`, wenn beides dasteht
  permissions: acceptEdits
---
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

`allowed-tools` hat drei Zustände, und der Unterschied zählt. Lassen Sie das Feld ganz weg, gilt der
volle Standardwerkzeugsatz der Agenten-Laufzeit. Schreiben Sie es mit leerem Wert (`allowed-tools:`
oder `""`), hat die Persona ausdrücklich keine — eine enge, nicht-agentische Rolle. Schreiben Sie
eine Liste, bekommt sie genau diese. Ein weggelassener Schlüssel und ein leerer sind *nicht*
dasselbe.

Die Liste ist der durch Leerzeichen getrennte Text der Spezifikation (`Bash Read Write`); ein durch
Kommas getrennter Text und eine YAML-Liste werden ebenfalls angenommen. Ein Eintrag darf eine Regel
statt eines bloßen Werkzeugs sein — `Bash(git add *)` — und wird nur außerhalb seiner Klammern
getrennt. In Claude Code GENEHMIGT `allowed-tools` Werkzeuge VORAB, ohne die übrigen
einzuschränken; hier ist es beides, denn eine Persona läuft, ohne dass jemand an der Tastatur sitzt,
der eine Rückfrage genehmigen könnte: Die Regeln gelten genehmigt, wie sie dastehen, und die
Werkzeuge, die sie nennen, sind der Werkzeugsatz der Persona.

Eine Persona, die überhaupt `nxc` ausführen soll, braucht `Bash`, denn so antwortet sie — und wo die
Engine eine Antwort *verlangt*, gibt sie das selbst dazu. Jede Beauftragung sagt der Persona in ihrem
eigenen Systemprompt, sie solle ihren Zug mit `nxc reply --thread <id>` beenden; wer so verpflichtet,
muss also sicherstellen, dass es ausführbar ist. Was der Trigger dazugibt, hängt von Ihrer
Deklaration ab und verengt sie nie: Eine Persona ohne `tools:` bekommt `Bash`, wie bisher; eine
Persona mit einer Liste ohne `Bash` — auch `tools: []` — bekommt `Bash(nxc reply:*)`, das ihre
Antwort ausführt und keinen anderen Befehl; eine Persona, die `Bash` aufführt, bekommt nichts dazu.
Für eine Persona, die die Shell für ihre *Arbeit* braucht, deklarieren Sie `Bash` weiterhin selbst;
was Sie nicht mehr mitdenken müssen, ist, dass Antworten auch Arbeit ist. (Was dieser Absatz über
`tools` sagt, gilt für `allowed-tools`: Es ist dasselbe Feld.) `limits-and-safety` sagt genau, was
die enge Gewährung durchlässt.

## Wer sie ansprechen darf

```yaml
nxs:
  addressable: general        # die Vorgabe: jeder darf direkt ein Gespräch eröffnen
  addressable: none           # niemand darf — komm über einen Kanal, der mich besetzt
  addressable:                # genau diese Aufrufer, und sonst niemand
    personas: [pm]
    humans: true
```

(Drei Alternativen, nicht ein Block — ein Frontmatter nimmt eine davon.)

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
erlauben — deshalb ist das Verzeichnis weiter unten Orientierung und nie Durchsetzung. Die
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

### Aus einem anderen Arbeitsbereich

Eine Persona eines ANDEREN Arbeitsbereichs auf derselben Maschine kann diese beauftragen, wenn die
Deklaration es erlaubt. Ihre Adresse von außen ist der Name des Arbeitsbereichs und das Handle der
Persona: `nxc send --to nxsflow/nexus-flow/pm`. Der Name eines Arbeitsbereichs ist
`<besitzer>/<repo>`; `nxs name` zeigt ihn (aus der Git-Herkunft abgeleitet und beim ersten Gebrauch
gespeichert), `nxs name <besitzer>/<repo>` setzt ihn, wo es keine Herkunft gibt. Innerhalb eines
Arbeitsbereichs ändert sich nichts: `pm` bleibt `pm`.

```yaml
nxs:
  addressable:
    humans: true
    external:
      - "*/pm"                     # der PM jedes Arbeitsbereichs, dem dieser vertraut
      - nxsflow/manufakt-io/pm     # oder genau dieser
```

- **Kein `external`, kein Weg von außen herein** — in jeder Form, auch unter `general`: „jeder“
  heißt jeder in diesem Arbeitsbereich. Ein bloßes Handle in `personas:` meint die gleichnamige
  Persona dieses Arbeitsbereichs und lässt nie eine fremde herein: `personas: [pm]` lässt den `pm`
  eines anderen Arbeitsbereichs nicht durch.
- **Zwei Prüfungen, beide nötig.** Der Arbeitsbereich des Aufrufers steht auf der Vertrauensliste
  dieses Arbeitsbereichs, UND seine Rolle steht in `external`. Einen Nachbarn nehmen Sie beim Namen
  auf mit `nxs sync trust add --workspace <besitzer>/<repo>` — **in beiden Arbeitsbereichen**: Die
  Antwort kommt mit dem Schlüssel des Empfängers signiert zurück, und eine Antwort aus einem
  Arbeitsbereich, dem der Aufrufer nicht vertraut, weckt niemanden. `send` lehnt deshalb vorab ab,
  bis er vertraut ist.
- **Die Rolle stempelt der Koordinator des Aufrufers**, aus der Sitzung, die er selbst gestartet
  hat — nie der Agent. Ein Mensch kommt von außen nicht herein: Er geht in jenen Arbeitsbereich und
  schreibt dort.
- **Eine Ablehnung nennt ihren Grund**, in der Antwort an den Aufrufer: *workspace unknown*, *not
  on this machine*, *not trusted*, *role not admitted*, *no such persona* oder *depth limit
  reached*.

Was dann geschieht, ist eine Konsultation mit einer Übergabe in der Mitte. Der Auftrag des Aufrufers
ist ein Faden in dessen eigenem Log, und nur dieser Faden geht hinüber: Der Empfänger startet seine
Persona in SEINER Arbeitskopie, mit SEINEM Board und Gedächtnis, und was diese Persona beauftragt,
um zu antworten, bleibt in ihrem Arbeitsbereich. Eine Rückfrage (`--escalate`) geht an den
Auftraggeber zurück — nie an ihm vorbei zum Besitzer des anderen Arbeitsbereichs —, und die Antwort
des Auftraggebers auf demselben Faden setzt die Persona fort. Die Tiefengrenze gilt für die ganze
Kette über beide Arbeitsbereiche — auf das Wort des Aufrufers: Die Tiefe reist im signierten Stempel
des Auftrags, und der Empfänger kann sie nicht gegen eine Kette prüfen, die er nie sieht. Einem
Arbeitsbereich zu vertrauen heißt, seinem Koordinator zu vertrauen, und ein feindseliger könnte
Tiefe 0 stempeln; vertrauen Sie nur Arbeitsbereichen, deren Koordinator Sie selbst laufen ließen.
`nxc status` zeigt den Faden in beiden Arbeitsbereichen mit der Gegenseite und einem von vier
Zuständen: `submitted`, `working`, `input-required`, `completed` (oder `rejected` / `canceled`).
Zwei Stunden ohne Lebenszeichen — keine Nachricht auf dem Faden, nichts im Transkript der
empfangenden Persona — brechen ihn ab und wecken den Auftraggeber mit dem Grund; `nxc withdraw` auf
der Operation zieht ihn auch im anderen Arbeitsbereich zurück.

`send` und `reply` tragen einen Grenzfaden sofort hinüber, die Zustellung braucht also keinen
Hintergrunddienst; den Rest trägt der Dienst in seinem Takt. Ein Fall braucht ihn doch, wie
innerhalb eines Arbeitsbereichs: Eine Antwort, die eintrifft, während ihr Auftraggeber seinen Zug
noch beendet, wird zurückgehalten und vom Dienst übergeben, sobald der Zug endet.

**Ausrollen.** Die Mapping-Form von `addressable` lehnt Schlüssel ab, die sie nicht kennt — das fängt
einen Tippfehler wie `persona:` —, darum kann eine Engine, die älter ist als `external`, eine
Persona-Datei mit dem Feld nicht laden. Setzen Sie `external:` nur in ein Repository, dessen Leser
alle — auch Apps, die eine Engine fest einbinden — einen Stand haben, der es kennt.

## Wen sie ansprechen darf

Niemand deklariert, wen eine Persona ansprechen darf. Ihr Sitzungsstart listet das Team auf, das aus
dem **abgeleitet** wird, was alle anderen deklarieren: jede Persona und jeder Kanal des
Arbeitsbereichs, die sie zulassen, jeweils in eigenen Worten beschrieben (`description`, bei einem
Kanal dessen `description`), mit dem Weg über den Kanal für eine Persona, die niemanden direkt
zulässt. Ein Ziel erscheint im Verzeichnis einer Persona, weil das Ziel es in seinem eigenen
`addressable` sagt — an einer Stelle, geschrieben von dem, der angesprochen wird.

Eines lässt die Liste weg: **die Kanäle, in denen die Persona selbst Mitglied ist.** Einem Prüfer
die `review`-Runde anzubieten, in der er sitzt, hilft in keiner Lesart.

Die Liste ist Orientierung, die eine Persona liest, keine Schranke: eine Schranke aus einer
ablegbaren Identität abzuleiten, wäre schlimmer als nutzlos (siehe oben).

**`address_book` ist abgeschafft.** Die ältere Form ließ eine Persona eine Liste tragen, wen sie
anspricht und warum. Das war eine zweite Stelle, an der der Weg aufgeschrieben stand, sie konnte dem
eigenen `addressable` des Ziels widersprechen, und ihre `why`-Zeilen veralteten, wenn sich die
Aufgabe eines Ziels änderte. Eine Deklaration, die es noch trägt, lädt; der Schlüssel wird
ignoriert, `nxs prime` sagt das, und `nxs personas migrate` lässt ihn weg. Auch
`address_book: []` — „diese Persona beauftragt niemanden" — hat keinen Nachfolger: Eine Persona
sieht die Ziele, die sie zulassen.

## Was sie zu brauchen angibt

```yaml
nxs:
  requires: [board, memory, mail]
```

`nxs.requires` nennt die Fähigkeiten, die eine Persona braucht, in freien Worten. **In diesem
Release wird es gelesen und gezeigt, und sonst nichts:** Kein Vokabular prüft die Namen, und nichts
bindet einen an ein Werkzeug, ein Abonnement oder ein Konto. `nxs prime` sagt das einmal, sobald
irgendeine Persona eine nennt, und `nxc list --json` trägt die Liste. Eine Fähigkeit je Umgebung zu
binden — „mail" als Outlook auf der einen Maschine und Gmail auf der anderen — kommt in einem
späteren Release; bauen Sie noch nicht darauf.

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
nxs:
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
nxs:
  machine: studio   # eine Maschinen-Kennung oder ein Name aus `nxs sync machines`; Standard: wo der Chat beginnt
```

Ein Chat mit dieser Persona läuft auf genau **einer** Maschine. Jede andere Maschine, die den
Arbeitsbereich synchronisiert, sieht den Chat und startet nichts. Die Maschine wird festgelegt, wenn
der Chat beginnt, in dieser Reihenfolge: `nxc send --to <persona> --machine <m>` für diesen einen
Chat, dann dieses `nxs.machine`, dann die Maschine, die den Chat beginnt. Die Antwort steht im Chat
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

`nxs.machine` wird nur gelesen, wenn ein Mensch einen Chat beginnt. Ein Chat, den eine laufende
Persona beginnt, läuft auf deren Maschine, weil die Antwort zu der Sitzung zurückkommen muss, die
gefragt hat. Eine Persona, die als Mitglied eines Kanals beauftragt wird, läuft dort, wo der Kanal
begonnen wurde.

## Ein ausgearbeitetes Beispiel

```markdown
---
name: coder
description: Implements a work order on a branch and merges it.
allowed-tools: Bash Read Write
nxs:
  title: Coder
  working_tree: exclusive
  permissions: acceptEdits
---
You are the coder. Implement the work order in the message you were handed: on a branch of
its own, with the project's own gates green before you merge it.

An answer from you means the branch is merged and those gates were green on the tree that was
merged. If you cannot get there — something is missing, or a decision came up that is not
yours to make — say what you are missing in your thread rather than answering.
```

**Drei Dinge in diesem Rumpf lohnt es sich zu übernehmen, und eine Leerstelle trägt alle drei.** Er
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

Vertrauenswürdige Dokumentation sagt, welche Felder wirkungslos sind. Drei Schlüssel der älteren
Form hatten nie eine Wirkung und werden mit einer Warnung von `nxs prime` ignoriert;
`nxs personas migrate` lässt sie weg:

- **`session: fresh | continue`** — wirkungslos, weil **nicht die Persona es entscheidet**. `nxc
  send --to <persona>` prägt eine frische Sitzung; eine `nxc reply --thread` in den eigenen Faden
  dieser Persona setzt die Sitzung fort, die sie bereits hat — mit allem, was sie bereits weiß; und
  in einem Kanal mit `steps:` entscheidet der betretene Schritt es mit seinem eigenen `resume:`
  (`nxc guide channels`).
- **`sub_agents: true | false`**.
- **`reports_to: <handle>`**.

**Bei `nxs.claude_md: override` ist Vorsicht geboten**, denn es ist nur ZUR HÄLFTE ungelesen. Das
damit gemeinte Ersatzdokument je Persona ist noch nicht spezifiziert, also setzt nichts eines
zusammen — untätig ist die Angabe deswegen aber nicht: sie ist schlicht nicht `inherit`, also wird
eine Persona, die sie deklariert, OHNE die `CLAUDE.md` des Projekts zusammengesetzt, genau wie bei
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
Kanaldatei, die auf demselben Weg zurückgedreht wurde — andere Mitglieder, ein anderes
`working_tree:`, ein anderes `timeout:` — steuert Ihre Agenten genauso und wird von beidem **nicht**
gemeldet. Der Rat oben heißt deshalb nicht „das Werkzeug passt schon auf": es ist die
Versionsverwaltung, die aufpasst, und dies ist ein zweites Paar Augen auf der Hälfte des Ordners, die
es sehen kann.

## Ein veröffentlichter Skill ist eine Persona

Ein Skill, den Sie gefunden haben — ein Ordner mit einer `SKILL.md`, vielleicht mit `references/`
und `scripts/` daneben —, ist eine Persona, sobald er in `.nxs-personas/` liegt: Kopieren Sie den
Ordner unverändert hinein, und das nächste `nxc list` zeigt ihn. Nichts muss umgeschrieben werden,
und kein Befehl muss laufen. Ihn dorthin zu legen ist die Entscheidung; `nxc` liest von sich aus
keinen anderen Skill-Ort (auch nicht `.claude/skills/`).

- **Ohne `nxs:` läuft er mit lauter Vorgabewerten:** ansprechbar für jeden in diesem
  Arbeitsbereich und für niemanden außerhalb, auf der Vorgabestufe, mit den Werkzeugen, die sein
  `allowed-tools` nennt, oder dem vollen Standardsatz, wenn es keine nennt. **Achten Sie auf den
  letzten Fall:** Ein veröffentlichter Skill nennt selten welche, und eine Persona ohne
  `allowed-tools` bekommt, wenn sie beauftragt wird, auch die Shell freigegeben, weil ihre Antwort
  über `nxc reply` läuft (siehe [Womit sie läuft](#womit-sie-lauft)). `nxs prime` warnt bei jedem
  Skill ohne `allowed-tools`; geben Sie ihm, was er braucht — `allowed-tools: Read` für einen Skill,
  der nur seine eigenen Dateien liest.
- **Felder, die nexus-flow nicht liest, bleiben unangetastet.** Claude Code und andere Laufzeiten
  fügen dem Frontmatter eigene Felder hinzu (`when_to_use`, `hooks`, …); sie sind kein Fehler.
  `license`, `compatibility` und `metadata`, die eigenen der Spezifikation, werden durchgereicht:
  nicht gelesen, von `nxs personas migrate` behalten und in `nxc list --json` unter `declaration`
  gezeigt, neben der Datei, ihrer Form und ihrem Ordner. Für einen Menschen ist eine Persona eine
  Persona — `nxc list` zeigt die Form nicht.
- **Ein nexus-flow-Schlüssel auf oberster Ebene wird nicht gelesen.** `addressable: none` gehört
  unter `nxs:`; oben geschrieben würde es ignoriert, deshalb warnt `nxs prime` davor. Vor einem
  Schlüssel unter `nxs:`, der kein Feld benennt, wird ebenfalls gewarnt.
- **Ein Name ist ein Handle.** Er darf keine Leer- oder Steuerzeichen enthalten, und eine `SKILL.md`
  über 1 MiB wird abgelehnt — einen Skill kopiert man herein, und den Ordner auf Benutzerebene liest
  jeder Arbeitsbereich neu.

## Eine Definition für jedes Repo

Eine Persona, die Sie in jedem Repo haben wollen — denselben `pm` überall —, müssen Sie nicht in
jedes kopieren. Neben dem eigenen `.nxs-personas/` eines Workspace gibt es einen **Ordner auf
Benutzerebene**, `~/.nexusflow/personas/`, genauso aufgebaut: eine `<name>/SKILL.md` je Persona und
eine `channels/<name>.yaml` je Kanal (oder die älteren `<handle>.yaml` und `channels.yaml`). Jeder
Workspace auf der Maschine liest ihn.

- **Zusammengeführt je Name.** Eine Persona oder ein Kanal, den das Repo selbst deklariert,
  **verdeckt** den gleichnamigen Eintrag auf Benutzerebene; alles andere kommt hinzu. Ein Kanal
  kommt mit den Personas, die er voraussetzt, und seine Mitglieder lösen gegen das zusammengeführte
  Team auf — ein Kanal `planning` auf Benutzerebene, der `coder` nennt, erreicht den eigenen `coder`
  jedes Repos.
- **Die Identität bleibt je Repo.** Aus einer Definition `pm` wird in jedem Repo, in dem sie läuft,
  ein eigener Teilnehmer, mit Board, Gedächtnis und Arbeitskopie dieses Repos.
- **Woher etwas kommt, wird gezeigt.** `nxc list` markiert jeden Eintrag aus dem Ordner auf
  Benutzerebene und sagt, welche Einträge das Repo verdeckt; `nxc list --json` trägt an einem
  solchen Eintrag `"origin": "user"` und unter `declarations` die Felder `user_path`, `from_user`
  und `shadowed`. `nxs prime` sagt dasselbe unter „Declarations". Eine Verdeckung wird dort gemeldet
  und nirgends sonst — nicht beim Öffnen eines Vorgangs, nicht in der Sitzung, die er startet. Ein
  Repo, das eine eigene Kopie hält, führt diese Kopie aus, mit ihren eigenen Hürden, Mitgliedern und
  Zugangsregeln; der Ordner auf Benutzerebene kann das nicht verbieten.
- **Wie der Ordner gefüllt wird, entscheiden Sie** — ein Git-Klon, eine symbolische Verknüpfung. Der
  Ordner ist der Vertrag, nicht seine Herkunft.

**Der Preis, ausgesprochen:** Eine Deklaration außerhalb des Repos ist nicht mit ihm versioniert,
und ein Klon auf einer Maschine ohne den Ordner hat keinen `pm`. Alles, was oben über
Versionsverwaltung steht, gilt für diesen Ordner genauso — halten Sie ihn in einem eigenen Repo.

Drei Dinge gelten genau wie für den eigenen Ordner des Repos:

- **Er wird je Vorgang eingefroren.** Ein Vorgang läuft unter dem zusammengeführten Katalog, wie er
  beim Öffnen des Vorgangs stand — Hürden (`preconditions:`) aus dem Ordner auf Benutzerebene
  eingeschlossen. Eine Änderung dort erreicht den nächsten Vorgang, nie einen laufenden.
- **Ein relativer Pfad folgt der Form.** In einer `SKILL.md` meint er den eigenen Ordner der
  Persona, sodass ein Skill auf Benutzerebene seine `references/` in jedem Repo findet, in dem er
  läuft — das ist die Regel der Agent Skills, und die Sitzung erfährt, wo der Ordner liegt
  („Declared in"). In der älteren YAML-Form meint er das Repo, in dem die Persona läuft: Ein Prompt,
  der sagt „lies `knowledge/x.md`, relativ zur Wurzel dieses Arbeitsbereichs", wird von einer
  Sitzung gelesen, deren Arbeitsverzeichnis das Repo ist, und eine Hürde läuft ebenfalls dort. Auch
  eine YAML-Persona aus dem Ordner auf Benutzerebene erfährt, wo ihre Deklaration liegt, damit ihre
  Anweisungen dorthin zeigen können.
- **Eine einbettende App liest denselben Ordner.** Sie wählt keinen anderen und kann es nicht: Die
  Personas, die sie startet, laufen mit `nxc`, das diesen Ordner liest, und zwei Antworten auf „wer
  existiert" würden eine App von ihren eigenen Sitzungen trennen.

**Ein `external` aus dem Ordner auf Benutzerebene öffnet kein Repo von selbst.** Eine Persona auf
Benutzerebene, die Aufrufer von außen zulässt, ist in jedem Repo zulässig, das sie führt, aber nur
aus den Arbeitsbereichen, denen dieses Repo in seiner eigenen Vertrauensliste beim Namen vertraut
(`nxs sync trust add --workspace`). Ein Repo, das niemandem vertraut, lässt von außen niemanden
herein.

Ein Entwicklungs-Build liest einen Ordner auf Benutzerebene nur unter einer benannten Dienst-Instanz
— `~/.nexusflow-<name>/personas/`, wenn `NXS_SERVICE_INSTANCE` eine benennt. Ein Build ohne sie (CI,
eine Shell ohne `direnv`) liest keinen, sodass ein Build im Test nie die Personas aufgreift, mit
denen Ihre installierte Suite läuft.

## Die ältere YAML-Form

Vor der Skill-Form war eine Persona eine `<handle>.yaml`. Sie wird weiterhin gelesen, im Ordner des
Repos ebenso wie im Ordner auf Benutzerebene, und bildet sich Feld für Feld ab:

| YAML-Form | Skill-Form |
| --- | --- |
| `handle` | `name` |
| `job_description` | `description` |
| `system_prompt` | der Rumpf der `SKILL.md` |
| `tools` | `allowed-tools`, mit denselben drei Zuständen |
| `job_title` | `nxs.title` |
| `expected_output`, `stage`, `model`, `addressable`, `prime`, `claude_md`, `base_prompt`, `permissions`, `working_tree`, `machine` | `nxs.<derselbe Name>` |
| `address_book`, `session`, `sub_agents`, `reports_to` | weggelassen — ignoriert, mit einer Warnung |

**Ein Name, eine Deklaration.** Eine `pm.yaml` neben einer `pm/SKILL.md` wird beim Lesen des Ordners
abgewiesen, und der Fehler nennt beide Dateien: Zwei Deklarationen eines Namens dürfen nicht still
eine gewinnen lassen.

**`nxs personas migrate`** schreibt einen Ordner aus der älteren Form in die neue um: jede
`<handle>.yaml` in `<handle>/SKILL.md`, die Liste in `channels.yaml` in je eine
`channels/<name>.yaml` pro Kanal, und es entfernt die alten Dateien. Es behält Ihre Kommentare bei
den Schlüsseln, über denen sie stehen, verschiebt die Notizen am Kopf von `channels.yaml` nach
`channels/README.md` und meldet je Datei, was es weggelassen hat. Jede umgeschriebene Datei wird
zurückgelesen, bevor irgendetwas geschrieben wird, und muss dieselbe Deklaration ergeben; ein
Schlüssel, den es nicht kennt, hält den Lauf an und nennt die Datei, bevor sich auch nur eine Datei
ändert. `--dry-run` zeigt den Plan und schreibt nichts, `--user` migriert den Ordner auf
Benutzerebene, und ein zweiter Lauf findet nichts mehr zu tun.

Eine App, die eine Engine einbettet, die älter ist als diese, sieht einen Skill-Ordner überhaupt
nicht — migrieren Sie ein Repo, das eine App liest, erst, wenn die App diese Version ausführt.

## Wenn eine Deklaration falsch ist

Eine fehlerhafte Datei ist ein lauter `validation`-Fehler, der den Pfad nennt — nie eine still
übersprungene Persona. Eine `SKILL.md` ohne Frontmatter oder ohne `name` ist einer; ein Unterordner
ohne `SKILL.md` ist gar keine Deklaration und bleibt unangetastet, sodass ein Team dort sein
`knowledge/` halten kann. Ein fehlender Ordner ist *kein* Fehler: ein Workspace ohne Deklarationen
löst sauber auf und hat schlicht niemanden anzusprechen, und `nxc list` und `nxs prime` sagen genau
das.

Referenzielle Probleme — ein Kanal, der eine nicht existierende Persona nennt; eine Persona, die sich
über einen Kanal erreichbar erklärt, in dem sie kein Mitglied ist — erscheinen in `nxs prime`, und
zwar nur im interaktiven Kontext. Einer gestarteten Persona werden die Fehler ihres Autors nicht
vorgehalten; einem Menschen an der Tastatur schon.

**Qualitätswarnungen kommen an derselben Stelle an, nach derselben Regel.** Eine Deklaration, die
Text kopiert, den die Engine ohnehin einspielt, deren `description` einem Rufer nicht sagen kann,
wann er ruft, die noch `address_book` oder einen wirkungslosen Schlüssel trägt, deren Name die Regel
der Agent Skills verletzt oder die unter `nxs.requires` Fähigkeiten nennt, die noch nichts bindet,
ist nicht kaputt — ebenso wenig ein Skill ohne `allowed-tools` oder eine Persona, die
Rückfragen überspringt (`bypassPermissions`, `dontAsk`) und zugleich Rufer aus anderen
Arbeitsbereichen zulässt (`external`); beides verdient aber einen zweiten Blick — deshalb ist sie eine *Warnung*: Sie wird dem Menschen
aufgelistet, sie schließt nichts aus, und die Deklaration lädt und läuft genau wie sonst. Was
geprüft wird, was bewusst nicht, und die vier Fehlerklassen, die keine Prüfung entscheiden kann,
stehen in [Deklarationen schreiben](nxc-writing-declarations).

## Weiter

- [Deklarationen schreiben](nxc-writing-declarations) — wie Sie diese Felder füllen, damit die
  Antworten brauchbar sind: sieben gemessene Fehlerklassen und eine Checkliste.
- [Kanäle](nxc-channels) — mehrere Personas zusammen arbeiten lassen.
- [Befehle](nxc-commands) — ansprechen, was Sie gerade deklariert haben.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — die Kappen rund um eine laufende Persona.
