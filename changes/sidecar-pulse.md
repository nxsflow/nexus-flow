---
type: added
facade: changed
---
[en]
`nexus-chat`: the `Worker` trait gains `session_pulse` (when a session's model last produced output, as `Pulse::{Unreported, NoneYet, At}`) and `withdraw_asks` (take back an ask the session has not read yet). Both have defaults (`Unreported`, a no-op `Ok`), so an existing host worker compiles unchanged. The bundled sidecar writes the pulse for model output only, and it now reads an ask at the running turn's result instead of at the signal, so a withdrawn ask is never delivered.
[de]
`nexus-chat`: Der Trait `Worker` erhält `session_pulse` (wann das Modell einer Sitzung zuletzt etwas ausgegeben hat, als `Pulse::{Unreported, NoneYet, At}`) und `withdraw_asks` (eine Nachfrage zurücknehmen, die die Sitzung noch nicht gelesen hat). Beide haben Vorgaben (`Unreported`, ein wirkungsloses `Ok`), ein bestehender Host-Worker kompiliert also unverändert. Der mitgelieferte Sidecar schreibt den Puls nur für Ausgaben des Modells und liest eine Nachfrage jetzt erst am Ergebnis des laufenden Zuges statt beim Signal, sodass eine zurückgenommene Nachfrage nie zugestellt wird.
