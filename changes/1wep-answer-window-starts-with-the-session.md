---
type: fixed
facade: none
---
[en]
A step of a declared channel that waited in line for the working copy used its answer window up
while it waited. In the field a build step waited six hours behind another operation's lease and
never started. When its window passed, the tick reported that the step "gave no answer" and opened
the verify step for a build that never ran. Now a step whose start still waits in the working-copy
queue cannot lapse, because it has not been asked yet. That includes a later step of an ordered run
that waits for the step before it. Its window starts when its session starts, and the clock that
watches that window is armed at the same moment. A step that started and then stayed silent still
lapses on its declared `timeout:` as before. The working-copy lease handed over after a long wait
is also no longer expired at the moment it is granted: its bound now counts from the start, not
from windows that ran out in the queue.
[de]
Ein Schritt eines deklarierten Kanals, der in der Warteschlange auf die Arbeitskopie wartete,
verbrauchte dabei sein Antwortfenster. Im Einsatz wartete ein Build-Schritt sechs Stunden hinter
dem Lease eines anderen Vorgangs und startete nie. Als sein Fenster ablief, meldete der Tick, der
Schritt habe „keine Antwort gegeben“, und öffnete den Verify-Schritt für einen Build, der nie lief.
Jetzt kann ein Schritt, dessen Start noch in der Warteschlange der Arbeitskopie steht, nicht
ablaufen, denn er wurde noch nicht gefragt. Das gilt auch für einen späteren Schritt eines
geordneten Durchlaufs, der auf den Schritt davor wartet. Sein Fenster beginnt, wenn seine Session
startet, und die Uhr, die dieses Fenster überwacht, wird im selben Moment gestellt. Ein Schritt,
der gestartet ist und dann schweigt, läuft wie bisher nach seinem deklarierten `timeout:` ab. Auch
der Lease auf die Arbeitskopie, der nach langem Warten übergeben wird, ist nicht mehr schon im
Moment der Übergabe abgelaufen: Seine Frist zählt jetzt ab dem Start, nicht nach Fenstern, die in
der Warteschlange abgelaufen sind.
