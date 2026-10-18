---
type: fixed
facade: changed
---
[en]
A member session that will not answer is now recorded as stranded, and its round is escalated to whoever commissioned it. Before, the flow was advanced past such a session, or waited on it for good. The session may never have started, never bound, ended without answering, or been killed. Every tick looks at each owed session two minutes after it was last started. A session that announced its end, or whose start was given up, is recorded at that look. A session with no end needs a second look one minute later on unchanged evidence, so it is recorded three minutes after its trigger. The record posts one escalation in the member's name: the round reads `needs_decision` and no next step opens. Nothing is restarted. A reply into the root commissions the round again, and `nxc withdraw` takes it back.

`nxc status` shows the record on the slot as `STRANDED: <what was seen>; recorded <when>, escalated to the commissioner`, in place of "ITS SESSION HAS ENDED, nothing is coming". The record stays until the same slot is triggered again.

A slot whose window ran out is no longer treated as settled while its answer is still coming. It waits while the step is queued for the working copy, has a retry due, is on a quota hold, waits on a round it commissioned itself, or has an answer held for it. It also waits while its session is not running, until the sweep records it. A session that is still running past its window (a hung process) is advanced past as before. A step that declares `input:` from a step that was asked in this run is not opened when that step stayed silent or was answered only by a substitute the runtime posted. Then the round escalates, naming the step, the source, its slot and its pass. A source with no slot in the run still contributes nothing, as before.

Library: `StatusThread::stranded` (`StrandedStatus`, `StrandedEscalation`, `Observation`), six `ConsequenceClass` variants (`LivenessUnknown`, `StepNotRunning`, `SessionStranded`, `StrandedNotEscalated`, `StrandedEscalationSkipped`, `StrandedNotJudged`), `Timer::schedule_stranded_look` (with a default), and `orchestration::STRANDED_GRACE_SECS`.

Limits:
- A slot opened before this version has no obligation record and keeps its old behaviour. So does a slot on a replica that never ran the trigger.
- A worker that cannot tell a live session from a dead one records nothing and says so once per tick (`liveness_unknown`).
- A process the worker cannot verify (an unverifiable pid claim) counts as running and is never recorded.
- A session driven by hand with `nxc session bind` holds no process claim. It must answer within the grace, or it is escalated.
- A paused member (hold, own sub-round, held delivery) has no time limit in this release. What a due hold that nothing takes up should become is 6j6v.21xy.
- When an escalation is given up, or a look cannot be scheduled and nothing else ticks the workspace, the round waits until a human withdraws it.
- A late answer after the record is tracked as 6j6v.av16.
- Over a channel's default edge, a substituted reply still advances the flow with its text. Only a declared `input:` refuses it.
- An embedding app that starts sessions with its own worker, while the background service runs the ticks, would have its sessions read as not running. Nothing in this repository mixes the two; the fix is tracked as 6j6v.j8sq.

Notes:
- Measured: the time from the obligation record to the spawn (a whole `nxc send`) is 0.15 s to 1.1 s, against a 120 s grace. That is the fast path; a slow `nxf prime` is bounded by its own 30 s timeout.
- One cold `nxc tick` takes 23 ms. A tick over about 200 threads has a median of 31 ms with 20 owed running sessions, and 258 ms with 200 (about 1.3 ms per owed session).
- The "never bound" observation has one wording, "no live process across two looks; it never bound", because the worker interface answers liveness only as yes or no and does not name the process claim.
- An escalation the runtime's teardown posted itself counts as the record's escalation, because the stamp is decided from what is on the thread.
[de]
Eine Mitglieds-Sitzung, die nicht antworten wird, wird jetzt als gestrandet festgehalten, und ihre Runde wird an den Auftraggeber eskaliert. Bisher wurde der Ablauf an einer solchen Sitzung vorbeigeschoben oder wartete für immer auf sie. Die Sitzung kann nie gestartet sein, sich nie angebunden haben, ohne Antwort geendet haben oder getötet worden sein. Jeder Tick sieht sich jede geschuldete Sitzung zwei Minuten nach ihrem letzten Start an. Eine Sitzung, die ihr Ende gemeldet hat oder deren Start aufgegeben wurde, wird bei diesem Blick festgehalten. Eine Sitzung ohne Ende braucht eine Minute später einen zweiten Blick mit unverändertem Befund und wird daher drei Minuten nach ihrem Trigger festgehalten. Der Eintrag postet eine Eskalation im Namen des Mitglieds: Die Runde liest `needs_decision`, und kein nächster Schritt öffnet. Nichts wird neu gestartet. Eine Antwort in den Wurzel-Faden beauftragt die Runde neu, und `nxc withdraw` nimmt sie zurück.

`nxc status` zeigt den Eintrag am Slot als `STRANDED: <Befund>; recorded <wann>, escalated to the commissioner`, anstelle von "ITS SESSION HAS ENDED, nothing is coming". Der Eintrag bleibt, bis derselbe Slot erneut getriggert wird.

Ein Slot, dessen Fenster abgelaufen ist, gilt nicht mehr als erledigt, solange seine Antwort noch kommt. Er wartet, solange der Schritt in der Warteschlange für die Arbeitskopie steht, ein Wiederholungsversuch fällig ist, er an einem Kontingent-Halt steht, auf eine selbst beauftragte Runde wartet oder eine Antwort für ihn zurückgehalten wird. Er wartet auch, solange seine Sitzung nicht läuft, bis der Sweep sie festhält. Eine Sitzung, die nach ihrem Fenster noch läuft (ein hängender Prozess), wird wie bisher übergangen. Ein Schritt, der `input:` von einem in diesem Lauf gefragten Schritt erklärt, wird nicht geöffnet, wenn jener schwieg oder nur mit einer vom Laufzeitsystem geposteten Ersatz-Antwort beantwortet wurde. Dann eskaliert die Runde und nennt den Schritt, die Quelle, ihren Slot und ihren Durchgang. Eine Quelle ohne Slot im Lauf trägt wie bisher nichts bei.

Bibliothek: `StatusThread::stranded` (`StrandedStatus`, `StrandedEscalation`, `Observation`), sechs `ConsequenceClass`-Varianten (`LivenessUnknown`, `StepNotRunning`, `SessionStranded`, `StrandedNotEscalated`, `StrandedEscalationSkipped`, `StrandedNotJudged`), `Timer::schedule_stranded_look` (mit Standard-Implementierung) und `orchestration::STRANDED_GRACE_SECS`.

Grenzen:
- Ein Slot, der vor dieser Version geöffnet wurde, hat keinen Verpflichtungs-Eintrag und behält sein altes Verhalten. Ebenso ein Slot auf einer Replik, die den Trigger nie ausgeführt hat.
- Ein Worker, der eine lebende nicht von einer toten Sitzung unterscheiden kann, hält nichts fest und sagt das einmal pro Tick (`liveness_unknown`).
- Ein Prozess, den der Worker nicht prüfen kann (ein nicht prüfbarer pid-Anspruch), gilt als laufend und wird nie festgehalten.
- Eine von Hand mit `nxc session bind` gebundene Sitzung hält keinen Prozess-Anspruch. Sie muss innerhalb der Frist antworten, sonst wird sie eskaliert.
- Ein pausiertes Mitglied (Halt, eigene Unter-Runde, zurückgehaltene Zustellung) hat in diesem Release keine zeitliche Grenze. Was aus einem fälligen Halt werden soll, den nichts aufnimmt, ist 6j6v.21xy.
- Wird eine Eskalation aufgegeben oder kann ein Blick nicht geplant werden und tickt nichts anderes den Arbeitsbereich, wartet die Runde, bis ein Mensch sie zurückzieht.
- Eine späte Antwort nach dem Eintrag ist als 6j6v.av16 erfasst.
- Über die Standard-Kante eines Kanals schiebt eine Ersatz-Antwort den Ablauf weiterhin mit ihrem Text weiter. Nur ein erklärtes `input:` lehnt sie ab.
- Startet eine einbettende App Sitzungen mit eigenem Worker, während der Hintergrunddienst die Ticks ausführt, würden ihre Sitzungen als nicht laufend gelesen. In diesem Repository mischt nichts die beiden; die Behebung ist als 6j6v.j8sq erfasst.

Hinweise:
- Gemessen: Die Zeit vom Verpflichtungs-Eintrag bis zum Start (ein ganzes `nxc send`) liegt bei 0,15 s bis 1,1 s, gegenüber einer Frist von 120 s. Das ist der schnelle Pfad; ein langsames `nxf prime` ist durch seine eigene Grenze von 30 s beschränkt.
- Ein kalter `nxc tick` dauert 23 ms. Ein Tick über etwa 200 Fäden hat mit 20 geschuldeten laufenden Sitzungen einen Median von 31 ms, mit 200 einen von 258 ms (etwa 1,3 ms je geschuldeter Sitzung).
- Der Befund "nie angebunden" hat eine einzige Formulierung, "no live process across two looks; it never bound", weil die Worker-Schnittstelle Lebendigkeit nur mit ja oder nein beantwortet und den Prozess-Anspruch nicht nennt.
- Eine Eskalation, die das Aufräumen des Laufzeitsystems selbst gepostet hat, zählt als Eskalation des Eintrags, weil der Stempel aus dem entschieden wird, was im Faden steht.
