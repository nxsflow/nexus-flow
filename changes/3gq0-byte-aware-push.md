---
type: fixed
---
[en]
A sync push now splits a batch by size as well as by count, so a run of large ops (a long body, a
big note) no longer gets stuck at the relay's 8 MiB request limit: before, such a batch was refused
with 413 and sent again unchanged on every pass, and nothing after it was pushed. A single op too
large to send even on its own now fails the pass with an error that names it. Every op before it
is still pushed, and the pass still pulls what others wrote. The op is never cut short.
[de]
Ein Sync-Push teilt einen Stapel jetzt nach Größe und nicht nur nach Anzahl auf. Eine Folge großer
Ops, etwa ein langer Text oder eine große Notiz, bleibt deshalb nicht mehr an der 8-MiB-Grenze des
Relays hängen. Bisher wurde so ein Stapel mit 413 abgelehnt und bei jedem Durchlauf unverändert
erneut geschickt, und nichts danach kam an. Eine einzelne Op, die selbst allein zu groß ist, lässt
den Durchlauf jetzt mit einer Fehlermeldung scheitern, die sie benennt. Alle Ops davor werden
trotzdem gepusht, und der Durchlauf holt weiter ab, was andere geschrieben haben. Die Op wird nie
gekürzt.
