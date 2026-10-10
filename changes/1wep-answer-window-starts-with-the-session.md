---
type: fixed
facade: changed
---
[en]
A step of a declared channel that waited in line for the working copy used its answer window up
while it waited. In the field a build step waited six hours behind another operation's lease and
never started. When its window passed, the tick reported that the step "gave no answer" and opened
the verify step for a build that never ran. Now a step whose start still waits cannot lapse,
because it has not been asked yet. That covers a step in the working-copy queue, a later step of an
ordered run that waits for the step before it, and a step whose start failed and will be retried.
Its window starts when its session starts, and the clock that watches that window is armed at the
same moment. A step that started and then stayed silent, and a failed start that will not be
retried, still lapse on the declared `timeout:` as before. Once the window has passed, the tick
says why the flow is not moving instead of answering only `not_due`. The working-copy lease handed
over after a long wait is no longer expired at the moment it is granted: its bound now counts from
the start, not from windows that ran out in the queue. For embedding apps: `ConsequenceClass`
gains `StepStillWaiting`, the finding a tick reports for such a step.
[de]
Ein Schritt eines deklarierten Kanals, der in der Warteschlange auf die Arbeitskopie wartete,
verbrauchte dabei sein Antwortfenster. Im Einsatz wartete ein Build-Schritt sechs Stunden hinter
dem Lease eines anderen Vorgangs und startete nie. Als sein Fenster ablief, meldete der Tick, der
Schritt habe „keine Antwort gegeben“, und öffnete den Verify-Schritt für einen Build, der nie lief.
Jetzt kann ein Schritt, dessen Start noch aussteht, nicht ablaufen, denn er wurde noch nicht
gefragt. Das gilt für einen Schritt in der Warteschlange der Arbeitskopie, für einen späteren
Schritt eines geordneten Durchlaufs, der auf den Schritt davor wartet, und für einen Schritt, dessen
Start fehlschlug und wiederholt wird. Sein Fenster beginnt, wenn seine Session startet, und die
Uhr, die dieses Fenster überwacht, wird im selben Moment gestellt. Ein Schritt, der gestartet ist
und dann schweigt, und ein fehlgeschlagener Start, der nicht wiederholt wird, laufen wie bisher
nach dem deklarierten `timeout:` ab. Ist das Fenster vorbei, sagt der Tick, warum der Ablauf nicht
weitergeht, statt nur `not_due` zu antworten. Der Lease auf die Arbeitskopie, der nach langem
Warten übergeben wird, ist nicht mehr schon im Moment der Übergabe abgelaufen: Seine Frist zählt
jetzt ab dem Start, nicht nach Fenstern, die in der Warteschlange abgelaufen sind. Für einbettende
Apps: `ConsequenceClass` bekommt `StepStillWaiting`, den Befund, den ein Tick für einen solchen
Schritt meldet.
