---
type: fixed
facade: changed
---
[en]
A commission that was waiting for the working copy and could not be started when the copy came free
is no longer lost. Before, the failure reached only the standard error of whichever process handed
the copy on — for the background service, a log file — and the commission was left with nothing
running, nothing queued and no way to take it back: `nxc withdraw` answered that nothing was there.
Now the receipt of the call that handed the copy on names it (`start_failed`; a commission that went
straight back into the queue is named as `start_requeued`), `nxc status` marks its operation
`START FAILED` with the error and what happens next, and a start that failed on an `io` error — a
timeout, a busy disk — is tried again by the background service, up to three attempts in all, a
minute or two apart. Any other failure, or one that outlasts those attempts, stays on the operation
until `nxc withdraw` discards it. A tick that completes a channel now also lets a finished chain's
working copy go, as a reply and a session end already did, and a reply that cannot decide the
release now says so on its receipt (`lease_undecided`). For embedding apps: `ConsequenceClass`
gains `StartFailed` and `StartRequeued`, `StatusOperation` gains `start_failed`
(`working_tree::FailedStart`), and `orchestration::MAX_START_ATTEMPTS` names the bound. A retry
whose thread has been answered in the meantime is dropped instead of started, a retry that takes
the working copy and fails again hands it on, and withdrawing such a commission lets the copy go
when nothing else in its operation still needs it.
[de]
Eine Beauftragung, die auf die Arbeitskopie wartete und nicht gestartet werden konnte, als die Kopie
frei wurde, geht nicht mehr verloren. Bisher erreichte der Fehler nur die Standardfehlerausgabe des
Prozesses, der die Kopie weitergab — beim Hintergrunddienst eine Logdatei —, und die Beauftragung
blieb ohne laufenden Prozess, ohne Platz in der Warteschlange und ohne Weg zurück: `nxc withdraw`
antwortete, es gebe nichts. Jetzt nennt sie die Quittung des Aufrufs, der die Kopie weitergab
(`start_failed`; eine Beauftragung, die sofort wieder in die Warteschlange ging, heißt
`start_requeued`), `nxc status` markiert ihren Vorgang mit `START FAILED`, samt Fehler und dem, was
als Nächstes geschieht, und ein Start, der an einem `io`-Fehler scheiterte — einer
Zeitüberschreitung, einer belegten Platte —, wird vom Hintergrunddienst erneut versucht, insgesamt
bis zu drei Mal, im Abstand von ein bis zwei Minuten. Jeder andere Fehler, oder einer, der diese
Versuche übersteht, bleibt am Vorgang stehen, bis `nxc withdraw` ihn verwirft. Ein Tick, der einen
Channel abschließt, gibt die Arbeitskopie einer fertigen Kette jetzt ebenfalls frei, wie es eine
Antwort und ein Session-Ende schon taten, und eine Antwort, die über die Freigabe nicht entscheiden
kann, sagt das jetzt in ihrer Quittung (`lease_undecided`). Für einbettende Apps: `ConsequenceClass`
bekommt `StartFailed` und `StartRequeued`, `StatusOperation` bekommt `start_failed`
(`working_tree::FailedStart`), und `orchestration::MAX_START_ATTEMPTS` benennt die Grenze. Ein
erneuter Versuch, dessen Faden inzwischen beantwortet ist, wird verworfen statt gestartet; ein
Versuch, der die Arbeitskopie bekommt und wieder scheitert, gibt sie weiter; und wer eine solche
Beauftragung zurücknimmt, gibt die Kopie frei, wenn in ihrem Vorgang nichts anderes sie noch braucht.
