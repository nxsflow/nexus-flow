# Eine Antwort über zwei Maschinen

[Die Reise einer einzelnen Operation](develop-the-journey-of-one-op) bleibt mit Absicht auf einer
Maschine: ein Verb, ein Log, ein Refold — damit die vier Behauptungen darunter sichtbar werden, ohne
dass eine zweite Handlung danebenläuft. Dies ist dieselbe Fahrt mit der Achse, die dort fehlt: zwei
Repliken, und eine Op, die von woanders eintrifft.

**Die Szene.** Auf dem Laptop lief ein Agent, dem eine Antwort zusteht. Du sitzt am Desktop und
antwortest dort. Die beiden Maschinen verbindet nichts als ein Relay und ein append-only Log.

![Eine Antwort, die von einer Maschine zur anderen wandert: nxc reply hängt Ops am Desktop an, das Relay trägt sie, der Laptop faltet seine messages-Sicht nach — und das Aufwecken bleibt zurück, an der einen Tabelle, die nicht reisen kann](/nxs/docs/assets/a-reply-across-two-machines.webp)

## Du antwortest auf der Maschine, an der du gerade sitzt

```
nxc reply --thread m-7f2 "ausliefern — das Flag standardmäßig aus"
```

`reply` hängt Nachrichten-Ops an das Log **dieser** Maschine an, genau wie `nxf close` in der ersten
Reise Feld-Ops anhängte, und die Quittung kommt zurück, sobald sie dauerhaft sind. Auch hier hält die
CLI kein eigenes Modell: sie ruft `Engine::reply_thread` auf `crates/chat`, dieselbe Naht, die eine
einbettende App ruft. Den Desktop hat bis hierher nichts verlassen.

## Der Desktop kann gar nicht wissen, dass jemand wartet

Die Quittung sagt `woke: null` — und sie meldet **keinen** fehlgeschlagenen Versuch, sondern dass nie
ein Aufwecken geschuldet war.

Die Rückadresse eines Fadens benennt eine *Session*, und Sessions liegen in `session_map`: einer
gewöhnlichen Tabelle, die der Reducer nie anfasst und die der Sync nie mitnimmt. Der Grund steht im
ersten Absatz jenes Moduls — eine Laufzeit-Session-Id „benennt eine lebende Session auf DIESER
Maschine", für einen Peer ist sie also bedeutungslos. Fragt man die Map des Desktops, wer hinter
dieser Adresse steht, lautet die Antwort *niemand*, und der Resume-Pfad liest das als den gewöhnlichen
Fall, nicht als ein Überspringen.

Damit steht die Grenze, für die es diese Seite gibt, schon nach einem Schritt da:

> **Das Log trägt, was gesagt wurde. Es trägt nicht, wer darauf wartet, es zu hören.**

## Das Relay bewegt Ops, niemals Sichten

Der Hintergrunddienst des Desktops schiebt bei seinem nächsten Durchgang, der des Laptops zieht bei
seinem eigenen. Dieser Austausch ist Anti-Entropie, keine Zustellung: jede Seite fragt, was die andere
hat und ihr selbst fehlt, und nimmt sich die Ops. Keine Op ist an eine Maschine adressiert, und kein
Server hält eine Warteschlange je Empfänger — das Relay hätte gar keinen Ort dafür, ihm hat nie jemand
gesagt, dass sich ein Laptop für diesen Faden interessiert.

Das ist die Eigenschaft der ersten Reise, von der anderen Seite gesehen. Die Sicht ist ein
Zwischenspeicher des Logs, also verschickt der Sync das Log; und weil er das Log verschickt,
konvergiert eine Replik, die eine Woche aus war, indem sie fragt — nicht indem man sich an sie
erinnert hat.

## Der Laptop faltet nach, und sagt es

Die gezogenen Ops landen in der `ops`-Tabelle des Laptops. Ein Durchgang bewegt das Log **jeder**
Domäne, faltet aber nur die Sichten von flow; die Nachricht liegt also da, während die
`messages`-Sicht noch nicht materialisiert ist. Das Öffnen eines Chat-Stores lässt den vorhandenen
Refold-wenn-hinterher laufen, und die Sicht holt auf. Nichts im Daemon weiß, wie eine Nachricht
faltet — der Auslöser ist allgemein, das Falten gehört dem Reducer.

Jede gezogene Op wird **beim Hereinkommen geprüft**: Der Laptop prüft ihre Unterschrift gegen den
Schlüssel, den sie nennt, und hält das Ergebnis daneben fest. Das Falten ignoriert dieses Ergebnis —
das Brett des Laptops kommt so oder so beim selben Stand an —, die Entscheidungs-Lesepfade aber
nicht. Deine Antwort erledigt den Thread des Laptops nur, wenn der Schlüssel des Schreibtisch-Rechners
auf der Vertrauensliste des Laptops steht (dort `nxs sync trust add`, einmal je Arbeitsbereich);
sonst steht sie als `unvouched` markiert im Thread, und die Runde wartet. Genau darum wird
unterschrieben: Der Relay in der Mitte kann nicht an deiner Stelle antworten.

Dieser Commit erhöht `PRAGMA data_version`, und dieses Pragma ist der ganze
Änderungs-Benachrichtigungsmechanismus: ein Hintergrund-Thread pollt es auf seiner eigenen
Leseverbindung, und `Engine::subscribe` gibt einen zusammengefassten *„jemand hat geschrieben — lies
neu"*-Tick heraus. Kein eigener Daemon, kein Socket, kein Push.

## Was weitergeht, und was nicht

Eine App, die **bereits lebt** — ein Desktop-Client, ein TUI, alles, was die Engine mit einem
`subscribe()` offen hält — bekommt diesen Tick, liest ihren Posteingang neu und zeigt die Antwort. Das
ist der Mechanismus, genau wie entworfen, über zwei Maschinen hinweg, ohne einen Netzwerkaufruf
zwischen Mensch und App.

Eine Agent-Session, die ihren Zug beendet hat, wird von nichts davon fortgesetzt. Der Dienst des
Laptops kennt genau einen Chat-Job, und das ist eine Frist, die der Laptop selbst gestellt hat; eine
Op, die von anderswo eintrifft, stellt keine. Das Aufwecken aus dem zweiten Abschnitt ist nicht am
Übergang gescheitert — es gab nie eines zu übertragen, denn dass eine Session wartet, steht nicht im
geteilten Log und kann dort bauartbedingt nicht stehen.

Wer darauf aufbaut, sollte diesen Satz behalten:

> **Der Sync liefert deine Daten, nicht deinen Kontrollfluss.**

Die Arbeit auf der Maschine fortzusetzen, auf der sie liegengeblieben ist, ist eine Entscheidung, die
deine App trifft — aus der Nachricht, die sie gerade gelesen hat.
