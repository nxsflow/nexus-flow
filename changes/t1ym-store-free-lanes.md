---
type: added
facade: changed
---
[en]
A server that folds a board into DynamoDB can now answer `next`, `blocked` and `deferred` exactly as
a local workspace does, with the same order and the same JSON, without a store. `nxs_fold_ddb::records::active_board`
reads the active tickets and their labels, custom values, timestamps, conversations and parent from
the folded tables, with a few reads per active ticket and never a scan. The facade then ranks and
projects them with `read::next_active_value`, `read::blocked_active_value` and
`read::deferred_active_value` (and their typed siblings `next_active`, `blocked_active`,
`deferred_active`). This is the same code that `Engine::next_value`, `Engine::blocked_value` and
`Engine::deferred_value` run, so the server never re-implements the ranking. `read::active_board`
builds the same input from a local store. A server stream folds one more entry per label and per
thread link, so that a ticket's labels and links can be found without a scan.
[de]
Ein Server, der ein Board in DynamoDB faltet, beantwortet `next`, `blocked` und `deferred` jetzt
genau wie ein lokaler Arbeitsbereich, mit derselben Reihenfolge und demselben JSON, ohne Store.
`nxs_fold_ddb::records::active_board` liest die aktiven Tickets mit ihren Labels, eigenen Feldern,
Zeitstempeln, Unterhaltungen und ihrem Elternelement aus den gefalteten Tabellen, mit wenigen
Zugriffen pro aktivem Ticket und nie mit einem Scan. Die Fassade ordnet und projiziert sie dann mit
`read::next_active_value`, `read::blocked_active_value` und `read::deferred_active_value` (und den
typisierten Gegenstücken `next_active`, `blocked_active`, `deferred_active`). Das ist derselbe Code,
den `Engine::next_value`, `Engine::blocked_value` und `Engine::deferred_value` ausführen, der Server
implementiert das Ranking also nie nach. `read::active_board` baut dieselbe Eingabe aus einem lokalen
Store. Ein Server-Stream faltet pro Label und pro Thread-Verknüpfung einen Eintrag mehr, damit die
Labels und Verknüpfungen eines Tickets ohne Scan auffindbar sind.
