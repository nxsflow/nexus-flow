# Kernkonzepte

Alles, was `nxc` tut, ruht auf sechs Ideen: ein gemeinsames Log, deklarierte Beteiligte,
qualifizierte Handles, Fäden mit abgeleitetem Quorum, Zustellung durch SCHIEBEN sowie Sitzungen
mit ihren Verläufen. Diese Anleitung ist das Modell unter den [Befehlen](nxc-commands) — wer es
einmal versteht, muss die Verben nicht mehr auswendig lernen.

## Das Substrat: ein Log, drei Werkzeuge

chat besitzt keine eigene Datenbank. Es öffnet dieselbe `.nxs/db.sqlite`, in die flow und memory
schreiben, registriert seinen eigenen Reduzierer und faltet seine eigenen Sichten. Jede
chat-Operation trägt die Domäne `message`, und ein Reduzierer, dem eine Domäne nicht gehört, lässt
eine Operation gespeichert, aber ungefaltet — deshalb können sich drei Werkzeuge ein Log teilen,
ohne sich gegenseitig zu verunreinigen.

Innerhalb dieser Domäne gibt es fünf Arten von Operation, jede mit dem Konfliktverhalten, das ihre
Aufgabe tatsächlich braucht:

| Art | Verhalten |
| --- | --- |
| `message` | nur wachsend. Eine Operation ist eine Nachricht: unveränderlich, nie bearbeitet. |
| `channel` | last-write-wins je Feld (`name`, `kind`, `origin`). |
| `membership` | eine Observed-Remove-Menge — ein Add trägt seine eigene Marke, ein Remove die Marken, die es gesehen hat. |
| `thread` | eine unveränderliche Wurzel plus vier last-write-wins-Register (`expects_reply_from`, `deadline`, `name`, `machine`). |
| `profile` | last-write-wins je Feld; eine Altlast, zu der es kein Verb mehr gibt. |

Bis nxf 6j6v.4d2z waren es sechs. Die sechste war `read_cursor`, ein Maximum-Register je Konsument
und Kanal, das nie rückwärts lief, und sie ist mit dem gesamten Ungelesen-Apparat entfallen — siehe
[Zustellung: geschoben, und nur geschoben](#zustellung-geschoben-und-nur-geschoben) weiter unten.
Eine `read_cursor`-Operation, die bereits in einem Log liegt, das Sie von einer älteren Replik
synchronisieren, ist kein Problem: kein Reduzierer beansprucht diese Art mehr, sie wird also
gespeichert und nie gefaltet — dieselbe Behandlung wie jede unbekannte Art, und genau der Grund,
warum sich drei Werkzeuge überhaupt ein Log teilen können.

**Es gibt keinen Dienst.** Die Änderungsbenachrichtigung von chat ist der
`PRAGMA data_version`-Wächter der Foundation; die CLI hält nie einen offen. Jedes Verb ist ein
einmaliger Zug: Workspace auflösen, Speicher öffnen, eine Operation anhängen oder eine Sicht lesen,
beenden.

## Sync ist der Bus

Zwei Agenten im *gleichen* Workspace sehen einander sofort — sie lesen dieselbe Datei. Zwei
verschiedene Workspaces sehen nichts voneinander, bis das Log ausgetauscht wird, und das ist eine
Suite-Operation an `nxs`, kein chat-Verb:

```bash
nxs sync bind              # einmal je Workspace; leitet den Stream aus dem git-Remote origin ab
nxs sync run               # ein Push/Pull-Durchgang — das bewegt Nachrichten zwischen Workspaces
```

`nxs sync bind` leitet die Stream-ID deterministisch aus Ihrem `origin`-Remote ab, sodass jeder Klon
eines Repositories auf demselben Stream landet, ohne dass etwas herumkopiert werden muss. Es
installiert außerdem nach bestem Bemühen einen benutzereigenen Hintergrunddurchlauf, den Sie mit
`--no-daemon` ablehnen und stattdessen von Hand steuern können.

Der Handel wird ausgesprochen statt versteckt: null Dauerkosten — kein Prozess, kein Socket, keine
Batterie — gegen eine Zustellung, die nicht sofort ist. Alles funktioniert auch ganz ohne Relay; Sie
haben dann eben einen Workspace statt mehrerer.

Eine Folge sollten Sie kennen, bevor Sie Vertrauliches in eine Nachricht schreiben: In dieser Stufe
ist Sync **vollständig und ungefiltert**. Es gibt auf der Leitung keine Autorisierung je Kanal.

Was aus einem anderen Workspace ankommt, **wirkt nur, wenn diesem Workspace hier vertraut wird**. Jede
Op ist von dem Workspace unterschrieben, der sie geschrieben hat; eine Nachricht, deren Unterschrift
dieser Workspace nicht gegen einen Schlüssel seiner Vertrauensliste (`nxs sync trust`) prüfen kann,
wird gezeigt — als `unvouched` markiert — und bewirkt nichts: Sie beantwortet keine Runde, nennt
keine Sitzung, die geweckt wird, und wird als niemandes Antwort eingesammelt. Ein Thread, dessen
Verpflichtung eine solche Op gesetzt hat, ist **gehalten**, und `nxc tick` sagt das, statt zu
handeln. Der Name an einer Nachricht ist das, was sie behauptet; zählen tut der Schlüssel.

## Deklarationen, keine Registrierungen

**Nichts in `nxc` erzeugt einen Beteiligten.** Es gibt kein Verb, das einen Agenten registriert oder
einen Kanal anlegt, und dieses Fehlen ist eine Entwurfsentscheidung, keine Lücke. Zwei Dateien unter
`.nxs-personas/` sind das ganze Modell:

- **`<handle>.yaml`** — eine **Persona**: wer sie ist, wofür sie da ist, mit welchem Modell und
  welchen Werkzeugen sie läuft und wer sie ansprechen darf. Siehe [Personas](nxc-personas).
- **`channels.yaml`** — eine Liste von **Kanälen**: wer darin ist, in welcher Reihenfolge sie laufen,
  wie lange eine Runde dauern darf und was mit den Antworten geschieht. Siehe
  [Kanäle](nxc-channels).

Eine Deklaration ist eine Datei, die man lesen, diffen, prüfen und einchecken kann. Eine
Laufzeit-Registrierung war eine Zeile in der Datenbank einer Maschine, die niemand geprüft hat und
die den Lauf nicht überlebte. `nxc list` ist die Lesung über den Ordner, und `nxc send --to` erreicht
das, was der Ordner deklariert — und sonst nichts.

## Identität: `<origin>/<agent>`

Jeder Absender, jede Erwartung, jeder Anspruch wird als **qualifiziertes Handle** geschrieben:

- **`<agent>`** ist `NXC_ACTOR`, sonst `$USER`, sonst das wörtliche `nxc`. Eine auf leer gesetzte
  Variable zählt als nicht gesetzt.
- **`<origin>`** ist `NXC_ORIGIN`, sonst das eigene **Replika-Präfix** des Workspace — derselbe kurze
  Namensraum, der auch den Item-IDs von flow vorangeht. Ein Handle liest sich also `ab12/coder`,
  nicht `local/coder`.

Der Origin gehört zum Workspace, nicht zur Maschine und nicht zum Benutzer. Das ist wichtig, weil
Handles per exaktem Zeichenkettenvergleich zusammenfinden: Prägte eine App `ab12/coder`, während ein
Terminal `local/coder` prägt, hätte ein deklarierter Kanal stillschweigend zwei disjunkte
Mitgliedermengen, und nichts würde das melden. Die CLI und die Bibliotheksnaht lösen denselben Wert
aus derselben Quelle auf, und ein Test hält sie daran fest.

Wenn chat eine Persona startet, gibt es dem neuen Prozess genau das mit, was er über sich wissen
muss: `NXC_ORIGIN`, `NXC_DB` (ein absoluter Pfad, damit das Arbeitsverzeichnis nie eine Rolle
spielt), `NXC_SESSION`, `NXC_ACTOR` auf das eigene Handle der Persona gesetzt und `NXC_HOP`. Sonst
erreicht nichts aus der Shell des Betreibers das Kind außer `PATH` und `HOME` — die Umgebung des
Kindes wird geleert und aus einer Positivliste neu befüllt, denn alles darin ist für die Werkzeuge
des Agenten lesbar.

## Kanäle: group, public, direct

Ein Kanal ist der Ort, an dem Nachrichten liegen; ein Faden ist ein Gespräch darin. Es gibt drei
Arten:

- **`group`** — was eine Deklaration standardmäßig erzeugt. Benannt, nur für Mitglieder.
- **`public`** — die Vordertür des Projekts. Es ist eine **Leseöffnung und sonst nichts**:
  Nachrichten und Tafeln darin sind für jeden im Workspace lesbar, ob Mitglied oder nicht. Nur für
  Mitglieder bleiben die mitgliedschaftsbezogenen Aufzählungen. Öffentliche Kanäle —
  auch solche, die per Sync hereingekommen sind und die hier nichts deklariert — erscheinen in
  `nxc list --json` unter `public_channels`, und nur dort: auffindbar ist nicht dasselbe wie
  ansprechbar, deshalb lässt die menschliche Darstellung sie weg.
- **`direct`** — ein Zwei-Parteien-Gespräch, das für Sie geprägt wird. Seine ID leitet sich aus den
  beiden Handles ab (sortiert, gehasht, `dm:` + 24 Hex-Zeichen), sodass beide Seiten dieselbe ID
  ohne Absprache und ohne Verb errechnen. `send --to <persona>` materialisiert es im Vorbeigehen.

Eine Deklaration darf `group` oder `public` verlangen. `direct` darf sie nicht verlangen — ein DM
wird abgeleitet, und ließe man das Wort durch, würde ein Tippfehler stillschweigend einen
Gruppenkanal erzeugen.

Eine Kanal-ID, die im Speicher existiert, die aber keine Deklaration benennt, ist **kein** Ziel. Ein
Senden dorthin wird abgelehnt, mit einer Meldung, die die Datei nennt, in der man sie deklariert;
ihre Nachrichten bleiben über `nxc threads show` und `search` lesbar.

## Fäden und das abgeleitete Quorum

Ein **Faden** ist die Adresse eines Gesprächs. `send --to` prägt einen und gibt ihn zurück, und
`reply --thread <id>` ist der einzige Weg, hineinzuposten. Ein Faden trägt eine unveränderliche
Wurzel — Origin, Kanal, Öffner und ein optionales `parent` — sowie zwei veränderliche Register.

`parent` macht aus einem **Vorgang** einen Baum statt einer Liste: es wird mechanisch daraus
aufgelöst, wo die sendende Sitzung stand, nie von einem Agenten getippt, und ein Faden ohne Elter ist
eine Wurzel. Deshalb ist „ist das die Wurzel?" abgeleitet und kein gespeichertes Merkmal, das falsch
sein könnte.

`expects_reply_from` ist die deklarierte Schuld: eine Liste qualifizierter Handles. Daraus und aus
den Nachrichten im Faden werden fünf Dinge **abgeleitet** — gespeichert wird nichts:

- **`expects`** — die deklarierten Handles, in Deklarationsreihenfolge.
- **`replied`** — diejenigen, die *seit der aktuellen Deklaration* gepostet haben. Verglichen wird
  gegen die Uhr der Deklaration selbst, und das ist es, was einen mehrzügigen Faden funktionieren
  lässt: die Antwort einer Rolle aus Zug eins löst Zug zwei nicht ein.
- **`outstanding`** — `expects` ohne `replied`.
- **`complete`** — die Erwartung wurde deklariert und nichts steht mehr aus. Uhrfrei und
  deterministisch, weshalb es der Auslöser sein kann, der Arbeit weiterbewegt.
- **`stale`** — es gibt eine Frist, jetzt liegt dahinter, und etwas steht noch aus.

`complete` und `stale` schließen einander konstruktionsbedingt aus, und zusammen sind sie das, was ein
Kanal „erledigt" nennt (siehe [Kanäle](nxc-channels)). Nur der Öffner darf die Erwartung eines Fadens
neu deklarieren; jeder andere bekommt `forbidden`.

## Zustellung: geschoben, und nur geschoben

**Zustellung ist ein SCHIEBEN.** Eine Nachricht erreicht einen Agenten über die Sitzung, die sie
startet oder wieder aufnimmt: `send --to` öffnet eine frische Sitzung mit dem Rumpf im Prompt,
`reply --thread` nimmt das Ziel mit dem Rumpf der Antwort wieder auf, und ein vollständiges Quorum
weckt den, der die Tafel geöffnet hat. Es gibt kein Verb, um nach den eigenen Nachrichten zu fragen,
und keines, um sie zu bestätigen — `nxc inbox` und `nxc read` sind entfernt (nxf 6j6v.1gm9), nachdem
beide mit je 0 Verwendungen in 66 Rollensitzungen gemessen worden waren. Ein Mensch liest
stattdessen das GESPRÄCH: `nxc threads show <faden>`.

Eine Nachricht trägt weiterhin eine **Disposition**: `in_turn` („jetzt handeln") oder `next_session`
(„beim nächsten Start nachholen"). Seit das Flag je Aufruf entfallen ist, schreibt jeder Aufrufer
`in_turn`; `next_session` lebt auf der Leitung und im Modell weiter und ist das, was die Engine für
ihre eigene Buchführung nutzt. **Wiedergegeben wird beim Sitzungsstart keines von beiden.** `nxs
prime` hat aufgehört, Nachrichtentexte zu drucken — in nxf 6j6v.4mmk das Ungelesene, in 6j6v.1gm9
die Ergebnisse selbst geöffneter Tafeln —, aus einem Grund, der zweimal gemessen wurde: was jede
Sitzung bezahlt, muss etwas sein, das sie nicht anders bekommt und vor dem Handeln braucht, und ein
Rumpf, den sie ohnehin gereicht bekam, ist beides nicht. Im zuletzt gemessenen Arbeitsbereich waren
das 63.787 Byte, davon 74 % der Text fertiger Aufträge.

**Eine Ungelesen-Menge gibt es überhaupt nicht mehr** (nxf 6j6v.4d2z). Bis v0.88.0 gab es eine —
Nachrichten in Kanälen, in denen Sie Mitglied sind, neuer als Ihr synchronisierter Lesezeiger, die
Sie nicht selbst gesendet haben —, an der Naht getragen als `in_turn`/`next_session`/`count` und mit
`Engine::mark_read` bestätigt. Niemand hat sie gelesen und niemand hat sie bestätigt, aus dem Grund
oben: eine Nachricht wird von der Sitzung zugestellt, die sie startet oder wieder aufnimmt — eine
Liste dessen, was Sie noch nicht abgeholt haben, ist also die zweite Kopie von etwas, das schon
angekommen ist. Der Lesezeiger, die Bestätigung, der `count` und die Ungelesen-Zahlen je Kanal auf
`Engine::channels` sind allesamt entfallen; an ihre Stelle ist nichts getreten. Geblieben ist das
Eine, wofür ein Ungelesen-Zähler stand — zu erfahren, wenn etwas passiert, worauf Sie warten —, und
das passiert, indem Sie geweckt werden.

## Sitzungen und Verläufe

Wenn chat eine Persona startet, prägt es eine **interne Sitzungs-ID** und übergibt sie der
Agenten-Laufzeit; die antwortet mit ihrer eigenen echten Sitzungs-ID, und `nxc session bind` bildet
beide aufeinander ab. Alles Weitere — die Rückadresse einer Antwort, die Tiefenbegrenzung, der
Verlauf — hängt an der internen ID, und die gibt `send --to <persona>` als `session` zurück.

Ein **Verlauf** ist der normalisierte Strom dieser Sitzung: Assistenztext, Denken, Werkzeugaufrufe
und deren Ergebnisse, wobei die Einträge eines per `Task` gestarteten Unteragenten unter dem Aufruf
verschachtelt sind, der ihn gestartet hat. Er beantwortet „was hat dieser Agent tatsächlich getan?",
wo `search` nur „was hat er gesagt?" beantwortet.

Verläufe sind **gerätelokal und werden nie synchronisiert** — sie sind mit Abstand das
volumenstärkste, was chat produziert, und ein gemeinsames, nur wachsendes Log damit zu füllen würde
es für jeden Partner aufblähen. Sie werden außerdem nicht ewig aufbewahrt: ganze Sitzungen altern an
ihrem letzten Eintrag aus, in einem Fenster, das standardmäßig 30 Tage beträgt und beim ersten
Schreiben jeder neuen Sitzung mitläuft.

## Vorgänge

Ein **Vorgang** ist der ganze Baum, den eine erste Nachricht begonnen hat: Ihr Faden, die Fäden, die
ein Kanal darunter geöffnet hat, und die Fäden, die diese wiederum geöffnet haben. `nxc status` ist
die Lesung darüber, und jeder Faden darin ist `open`, `answered` oder `stale`, mit einem Merkmal
daneben:

- **`open`** — jemand schuldet noch eine Antwort. Eine Kette, die *hängt*, sieht ebenso aus.
- **`answered`** — die Erwartung wurde deklariert und eingelöst.
- **`stale`** — das deklarierte Fenster lief ab, während etwas noch ausstand.
- **`awaiting_human`** — nur an der **Wurzel** eines abgeschlossenen Vorgangs wahr.

Dieses letzte Merkmal gibt es, weil eine hängende Kette und eine abgeschlossene, die auf einen
Menschen wartet, für einen Zähler identisch aussehen und Gegenteiliges bedeuten. Es ist nur ganz oben
je wahr, und das ist dieselbe Aussage wie: kein Mensch steht mitten im Ablauf. Mehr dazu unter
[Grenzen und Sicherheit](nxc-limits-and-safety).

## Weiter

- [Personas](nxc-personas) — deklarieren, wer Ihre Agenten sind.
- [Kanäle](nxc-channels) — deklarieren, wie sie zusammenarbeiten, parallel oder der Reihe nach.
- [Befehle](nxc-commands) — die Verben über all dem.
