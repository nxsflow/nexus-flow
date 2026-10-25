---
type: added
facade: breaking
---
[en]
`nxs sync run --bearer-stdin` runs a sync pass with a bearer token read from stdin, as `nxs sync snapshot --bearer-stdin` already does: one line, never on the command line, in the environment or in any output, and the relay URL must then carry no user and password. A token the relay refuses on the first request is `forbidden` (exit 1). A token the relay accepted and then refuses part-way (HTTP 401) ends the pass as the new error kind `token_expired`, with exit code 75 and the message "token expired, run again with a fresh one".

Every sync pass, the background service's included, now writes its position to `.nxs/sync.toml` after each push batch the relay acknowledges and each pull page it applies, not only at the end. A pass that is killed or whose token runs out keeps everything that landed, and the next pass resumes there without pushing it again.

For embedders: `ErrorKind` gains `TokenExpired` (`"token_expired"`). `nxs::sync::run_pass_with_bearer` is the library seam under the flag, and `nxs_sync::engine::sync_checkpointed` with the `Checkpoint` trait reports each position as the pass reaches it.
[de]
`nxs sync run --bearer-stdin` führt einen Sync-Durchlauf mit einem Bearer-Token von stdin aus, wie es `nxs sync snapshot --bearer-stdin` bereits tut: eine Zeile, nie auf der Kommandozeile, in der Umgebung oder in einer Ausgabe, und die Relay-URL darf dann weder Benutzer noch Passwort tragen. Ein Token, das das Relay bei der ersten Anfrage ablehnt, ist `forbidden` (Exit 1). Ein Token, das das Relay angenommen hat und unterwegs ablehnt (HTTP 401), beendet den Durchlauf mit der neuen Fehlerart `token_expired`, Exit-Code 75 und der Meldung "token expired, run again with a fresh one".

Jeder Sync-Durchlauf, auch der des Hintergrunddienstes, schreibt seine Position jetzt nach jedem Push-Batch, den das Relay bestätigt, und nach jeder Pull-Seite, die er anwendet, nach `.nxs/sync.toml`, nicht erst am Ende. Ein abgebrochener Durchlauf oder einer, dessen Token abläuft, behält alles, was angekommen ist, und der nächste setzt dort fort, ohne es erneut zu pushen.

Für Einbettende: `ErrorKind` erhält `TokenExpired` (`"token_expired"`). `nxs::sync::run_pass_with_bearer` ist die Bibliotheksnaht unter dem Flag, und `nxs_sync::engine::sync_checkpointed` mit dem Trait `Checkpoint` meldet jede Position, sobald der Durchlauf sie erreicht.
