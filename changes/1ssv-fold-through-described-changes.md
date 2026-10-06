---
type: changed
facade: breaking
---
[en]
The views no longer lean on the op log. The parent of an item, the weight of a thread link, the
created and updated instants of an item and the order and date of a note are now kept in the folded
rows themselves instead of being looked up in the log on every read, so the same views can be folded
where no log is kept beside them. Two rules that depended on the order ops arrived in now follow the
ops alone: when two ops claim one message id, the one with the lowest coordinate is the message, and
when two ops open one thread, the lowest one keeps its root — every replica now shows the same
message and the same thread, whatever order it received them in. The first time this version opens
a workspace it rebuilds the board's and the chat's views once from the log; that also corrects
messages an old build had recorded under the sender their payload claimed. An item's created and
updated instants now count the ops this version folds; an op it keeps but cannot fold no longer
moves them. The workspace schema goes to v8; older versions keep working on the same file, and
whatever they write there is folded again the next time this version opens it. For embedding
apps: a `Reducer` no longer folds into a connection — it returns the changes an op makes
(`Reducer::changes`, built from `nxs_foundation::change`), and the substrate carries them out; a
reducer declares `fold_revision` when its rules change. `Image` gains `fold_revision` and
`RefoldReason` gains `OtherRevision`.
[de]
Die Ansichten stützen sich nicht mehr auf das Op-Log. Das Elternteil eines Tickets, das Gewicht
einer Faden-Verknüpfung, der Erstellungs- und Änderungszeitpunkt eines Tickets sowie Reihenfolge und
Datum einer Notiz stehen jetzt in den gefalteten Zeilen selbst, statt bei jedem Lesen im Log
nachgeschlagen zu werden. So lassen sich dieselben Ansichten auch dort falten, wo kein Log
danebenliegt. Zwei Regeln, die von der Ankunftsreihenfolge der Ops abhingen, folgen jetzt allein den
Ops: Beanspruchen zwei Ops dieselbe Nachrichten-ID, ist die mit der kleinsten Koordinate die
Nachricht, und eröffnen zwei Ops denselben Faden, behält die kleinste seine Wurzel. Jedes Replikat
zeigt damit dieselbe Nachricht und denselben Faden, gleich in welcher Reihenfolge es sie empfangen
hat. Beim ersten Öffnen eines Workspaces baut diese Version die Ansichten von Board und Chat einmal
aus dem Log neu auf; dabei werden auch Nachrichten berichtigt, die ein alter Build unter dem in ihrer
Nutzlast behaupteten Absender abgelegt hatte. Erstellungs- und Änderungszeitpunkt eines Tickets
zählen jetzt die Ops, die diese Version faltet; eine Op, die sie aufbewahrt, aber nicht falten kann,
verschiebt sie nicht mehr. Das Workspace-Schema geht auf v8; ältere Versionen arbeiten auf derselben
Datei weiter, und was sie dort schreiben, wird beim nächsten Öffnen durch diese Version neu gefaltet.
Für einbettende Apps: Ein `Reducer` faltet nicht mehr selbst in eine Verbindung, sondern gibt die
Änderungen zurück, die eine Op bewirkt (`Reducer::changes`, gebaut aus `nxs_foundation::change`);
das Substrat setzt sie um. Ändern sich seine Regeln, erhöht ein Reducer `fold_revision`. `Image`
bekommt `fold_revision`, `RefoldReason` bekommt `OtherRevision`.
