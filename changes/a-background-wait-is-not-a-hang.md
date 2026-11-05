---
type: fixed
facade: none
---
[en]
A session that ended its turn to wait for a background job is no longer stopped while that job works. Until now the liveness ladder saw no open tool call and stopped it after 15 minutes, killing a 15-minute test run or a merger watching CI. Processes the session's calls started now count as movement, as they do under an open call. A wait with nothing moving is still stopped: 35 minutes after the last movement, or 15 minutes after the session's last output when nothing runs under it. Work that keeps moving is escalated as hung after 3 hours. The escalation no longer reports an earlier call's movement as the current one.
[de]
Eine Sitzung, die ihren Zug beendet hat, um auf einen Hintergrundjob zu warten, wird nicht mehr gestoppt, solange dieser Job arbeitet. Bisher sah die Liveness-Leiter keinen offenen Tool-Aufruf und stoppte sie nach 15 Minuten, was einen 15-minütigen Testlauf oder einen Merger beim Warten auf die CI abbrach. Prozesse, die die Aufrufe der Sitzung gestartet haben, zählen jetzt als Bewegung, wie unter einem offenen Aufruf. Ein Warten ohne jede Bewegung wird weiterhin gestoppt: 35 Minuten nach der letzten Bewegung, oder 15 Minuten nach der letzten Ausgabe der Sitzung, wenn nichts unter ihr läuft. Arbeit, die sich immer weiter bewegt, wird nach 3 Stunden als hängend eskaliert. Die Eskalation meldet die Bewegung eines früheren Aufrufs nicht mehr als die aktuelle.
