---
type: changed
facade: breaking
---
[en]
The views no longer lean on the op log. The parent of an item, the weight of a thread link, the
created and updated instants of an item and the order and date of a note are now kept in the folded
rows themselves instead of being looked up in the log on every read, so the same views can be folded
where no log is kept beside them. A message and a new thread now belong to the op that creates
them: their id is minted from that op's own id, and another op claiming the same id never takes the
message or the thread over, whatever its coordinate — where it used to be whichever op arrived
first, so two replicas could show different messages under one id. For an id minted before this
version, the claimant with the lowest coordinate is the message, and an op numbered below 1 is kept
but never shown. The first time this version opens a workspace it rebuilds the board's and the
chat's views once from the log; that also corrects messages an old build had recorded under the
sender their payload claimed. An item's created and
updated instants now count the ops this version folds; an op it keeps but cannot fold no longer
moves them. The workspace schema goes to v8; older versions keep working on the same file — an item
still shows one parent and a thread link one weight, and a parent or weight they set there takes
effect when this version next opens the file — and that open rebuilds whatever they wrote. For embedding
apps: a `Reducer` no longer folds into a connection — it returns the changes an op makes
(`Reducer::changes`, built from `nxs_foundation::change`), and the substrate carries them out; a
reducer declares `fold_revision` when its rules change. New: `Change::register_ranked`,
`Store::emit_owned`, `Op::owns_target` and `ChatStore::open_new_thread`. `Image` gains
`fold_revision` and `RefoldReason` gains `OtherRevision`.
[de]
Die Ansichten stützen sich nicht mehr auf das Op-Log. Das Elternteil eines Tickets, das Gewicht
einer Faden-Verknüpfung, der Erstellungs- und Änderungszeitpunkt eines Tickets sowie Reihenfolge und
Datum einer Notiz stehen jetzt in den gefalteten Zeilen selbst, statt bei jedem Lesen im Log
nachgeschlagen zu werden. So lassen sich dieselben Ansichten auch dort falten, wo kein Log
danebenliegt. Eine Nachricht und ein neuer Faden gehören jetzt der Op, die sie anlegt: Ihre ID wird
aus der ID dieser Op geprägt, und eine andere Op, die dieselbe ID beansprucht, übernimmt Nachricht
oder Faden nie, gleich mit welcher Koordinate. Bisher gewann die zuerst angekommene Op, sodass zwei
Replikate unter einer ID verschiedene Nachrichten zeigen konnten. Für eine vor dieser Version
geprägte ID ist die Op mit der kleinsten Koordinate die Nachricht, und eine Op mit einer Nummer
unter 1 wird aufbewahrt, aber nie angezeigt. Beim ersten Öffnen eines Workspaces baut diese Version die Ansichten von Board und Chat einmal
aus dem Log neu auf; dabei werden auch Nachrichten berichtigt, die ein alter Build unter dem in ihrer
Nutzlast behaupteten Absender abgelegt hatte. Erstellungs- und Änderungszeitpunkt eines Tickets
zählen jetzt die Ops, die diese Version faltet; eine Op, die sie aufbewahrt, aber nicht falten kann,
verschiebt sie nicht mehr. Das Workspace-Schema geht auf v8; ältere Versionen arbeiten auf derselben
Datei weiter — ein Ticket zeigt weiter ein Elternteil und eine Faden-Verknüpfung ein Gewicht, und
ein Elternteil oder Gewicht, das sie dort setzen, wirkt, sobald diese Version die Datei das nächste
Mal öffnet —, und dieses Öffnen baut neu auf, was sie geschrieben haben.
Für einbettende Apps: Ein `Reducer` faltet nicht mehr selbst in eine Verbindung, sondern gibt die
Änderungen zurück, die eine Op bewirkt (`Reducer::changes`, gebaut aus `nxs_foundation::change`);
das Substrat setzt sie um. Ändern sich seine Regeln, erhöht ein Reducer `fold_revision`. Neu:
`Change::register_ranked`, `Store::emit_owned`, `Op::owns_target` und `ChatStore::open_new_thread`.
`Image` bekommt `fold_revision`, `RefoldReason` bekommt `OtherRevision`.
