---
type: fixed
facade: none
---
[en]
On Linux, a session waiting on a patient poll loop such as `sleep 60; gh ...` is no longer stopped as hung. The loop spends less than one clock tick of CPU per round, so the liveness ladder read it as unmoving and stopped it after 35 minutes, inside an open tool call or after a turn that ended to wait for background work. A new process under the session's calls now counts as movement too. A job that blocks and starts nothing is still stopped, as is a poll interval above 30 minutes.
[de]
Unter Linux wird eine Sitzung, die auf eine geduldige Abfrageschleife wie `sleep 60; gh ...` wartet, nicht mehr als hängend gestoppt. Die Schleife verbraucht pro Runde weniger als einen Clock-Tick CPU, deshalb las die Liveness-Leiter sie als unbewegt und stoppte sie nach 35 Minuten, in einem offenen Tool-Aufruf oder nach einem Zug, der endete, um auf Hintergrundarbeit zu warten. Ein neuer Prozess unter den Aufrufen der Sitzung zählt jetzt ebenfalls als Bewegung. Ein Job, der blockiert und nichts startet, wird weiterhin gestoppt, ebenso ein Abfrageintervall über 30 Minuten.
