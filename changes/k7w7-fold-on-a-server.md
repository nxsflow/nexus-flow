---
type: added
facade: changed
---
[en]
A server can now fold a stream itself, with the same rules as a local replica: the new
`nxs-fold-ddb` crate carries the board's, the chat's and the memory's described changes out as
conditional writes against one DynamoDB table, keeps two sparse indexes beside the rows — `active`
(the active tickets and the dependencies and parents that touch one) and `dated` (the closed and the
archived lanes, newest first) — and reads next, blocked and deferred through the same
`nexus_flow_core::graph` library a local board uses, with one Query and no Scan. The order ops
arrive in and ops delivered twice do not change the result, and a run that stops half way and folds
its batch again ends where an unbroken run ends. An op too large for DynamoDB's limits is refused
and reported instead of stopping the fold. A snapshot for starting such a fold is
the tables plus the relay position they reach, without the log; one can be taken from a server
stream or from a local replica's views. The table it expects is written down in the crate and in
the sync spec. The DynamoDB adapter is behind the crate's `dynamodb` feature, so a build without it
needs no AWS SDK. For embedding apps: `nxs_foundation::reducer::folder_for` is the one rule for
which reducer folds an op, and `nexus_flow_core::model::is_active` the active rule over three cells.
Finding dependency cycles now takes time linear in the board.
[de]
Ein Server kann einen Strom jetzt selbst falten, nach denselben Regeln wie ein lokales Replikat: Die
neue Crate `nxs-fold-ddb` setzt die beschriebenen Änderungen von Board, Chat und Memory als bedingte
Schreibvorgänge in einer DynamoDB-Tabelle um. Neben den Zeilen pflegt sie zwei dünn besetzte
Indizes: `active` (die aktiven Tickets und die Abhängigkeiten und Elternteile, die eines berühren)
und `dated` (die Spalten geschlossen und archiviert, neueste zuerst). Bereit, blockiert und
zurückgestellt liest sie über dieselbe Bibliothek `nexus_flow_core::graph`, die ein lokales Board
nutzt — mit einer Query und ohne Scan. Weder die Reihenfolge, in der Ops ankommen, noch doppelt
gelieferte Ops ändern das Ergebnis, und ein Lauf, der mittendrin abbricht und seinen Stapel erneut
faltet, endet dort, wo ein ununterbrochener endet. Eine Op, die die Grenzen von DynamoDB sprengt,
wird abgelehnt und gemeldet, statt das Falten anzuhalten. Ein Schnappschuss zum Anlaufen eines solchen Faltens besteht aus
den Tabellen und der Relay-Position, die sie erreichen, ohne das Log; er lässt sich aus einem
Server-Strom oder aus den Ansichten eines lokalen Replikats ziehen. Die erwartete Tabelle ist in der
Crate und in der Sync-Spezifikation festgehalten. Der DynamoDB-Adapter steckt hinter dem Feature
`dynamodb` der Crate; ein Build ohne braucht kein AWS-SDK. Für einbettende Apps:
`nxs_foundation::reducer::folder_for` ist die eine Regel, welcher Reducer eine Op faltet, und
`nexus_flow_core::model::is_active` die Aktivitätsregel über drei Zellen. Zyklen in Abhängigkeiten
zu finden dauert jetzt linear in der Größe des Boards.
