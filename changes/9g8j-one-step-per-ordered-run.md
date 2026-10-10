---
type: fixed
facade: changed
---
[en]
When the working copy came free, the hand-off started every waiting commission of the next
operation at once, including two steps of one ordered flow (`flow: sequential` or `steps:`). In the
field a coder and its verifier ran side by side in one checkout: the coder switched branches and
spoiled the verifier's gate run. Now the hand-off starts one step per ordered run. A later step that
was waiting together with an earlier one stays in line until the earlier step has answered or its
window has lapsed, and its session is over; an earlier step whose start failed and is to be retried
holds it too. A reply, a session end or a tick then starts it. With a worker that cannot tell
whether a session still runs, a session counts as running until it announces its end. The members
of a parallel round still start together. A waiting later step of the operation that holds the copy
no longer counts as somebody else waiting: it does not make the copy contended, and a park or a
reclaim hands the copy to a rival before it gives it back to that operation. For embedding apps:
`Promotions` gains `deferred`, the steps put back in line, and
`ChatStore::first_waiting_for_the_working_tree` answers whether somebody else is waiting for the
copy.
[de]
Wurde die Arbeitskopie frei, startete die Übergabe alle wartenden Beauftragungen des nächsten
Vorgangs auf einmal, auch zwei Schritte eines geordneten Ablaufs (`flow: sequential` oder
`steps:`). Im Einsatz liefen so ein Coder und sein Verifier nebeneinander in einer Arbeitskopie: Der
Coder wechselte den Branch und verdarb den Gate-Lauf des Verifiers. Jetzt startet die Übergabe einen
Schritt je geordnetem Durchlauf. Ein späterer Schritt, der zusammen mit einem früheren wartete,
bleibt in der Warteschlange, bis der frühere geantwortet hat oder sein Zeitfenster abgelaufen ist
und seine Session vorbei ist; ein früherer Schritt, dessen Start gescheitert ist und erneut versucht
wird, hält ihn ebenso. Danach startet ihn eine Antwort, ein Session-Ende oder ein Tick. Kann der
Worker nicht sagen, ob eine Session noch läuft, zählt sie als laufend, bis sie ihr Ende meldet. Die
Mitglieder einer parallelen Runde starten weiter zusammen. Ein wartender späterer Schritt des
Vorgangs, der die Kopie hält, zählt nicht mehr als jemand anderes, der wartet: Er macht die Kopie
nicht umkämpft, und ein Parken oder Zurückholen gibt die Kopie zuerst an einen anderen Vorgang
weiter, bevor es sie diesem Vorgang zurückgibt. Für einbettende Apps: `Promotions` bekommt
`deferred`, die zurückgestellten Schritte, und `ChatStore::first_waiting_for_the_working_tree`
beantwortet, ob jemand anderes auf die Kopie wartet.
