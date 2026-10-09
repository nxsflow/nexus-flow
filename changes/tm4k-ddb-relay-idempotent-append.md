---
type: fixed
---
[en]
The relay's DynamoDB backend now stores a re-pushed op once, as the SQLite and Postgres backends
always have: an op that is already in the stream answers with the position it already has, and the
log does not grow. Before, every re-push — a writer pushing its own ops again after a cold start,
two sync passes racing, or the SDK retrying a write whose answer was lost — stored the op again.
Each op now carries a small claim item under a negative `seq` in the same table, written in one
transaction with the op; no new table, index or IAM action is needed. A consumer of a DynamoDB
stream on the op table must skip items with `seq <= 0`, as it already had to for the counter. Ops
stored twice before this change stay as they are.
[de]
Das DynamoDB-Backend des Relays speichert eine erneut gepushte Op jetzt nur einmal, wie es die
SQLite- und Postgres-Backends schon immer tun: Eine Op, die schon im Strom liegt, antwortet mit der
Position, die sie bereits hat, und das Log wächst nicht. Bisher wurde die Op bei jedem erneuten
Push wieder gespeichert, etwa wenn ein Schreiber nach einem Kaltstart seine eigenen Ops noch einmal
pusht, wenn zwei Sync-Durchläufe gleichzeitig laufen oder wenn das SDK einen Schreibvorgang
wiederholt, dessen Antwort verloren ging. Jede Op trägt jetzt einen kleinen Anspruchseintrag unter
einer negativen `seq` in derselben Tabelle, geschrieben in einer Transaktion mit der Op; eine neue
Tabelle, ein neuer Index oder eine neue IAM-Aktion ist nicht nötig. Wer einen DynamoDB-Stream auf
der Op-Tabelle liest, muss Einträge mit `seq <= 0` überspringen, wie bisher schon den Zähler. Ops,
die vor dieser Änderung doppelt gespeichert wurden, bleiben, wie sie sind.
