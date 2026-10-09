---
type: fixed
---
[en]
A sync push now splits a batch by size as well as by count, so a run of large ops (a long body, a
big note) no longer gets stuck at the relay's 8 MiB request limit: before, such a batch was refused
with 413 and sent again unchanged on every pass, and nothing after it was pushed. A single op too
large to send even on its own now fails the pass with an error that names it. Every op before it
is still pushed, and the pass still pulls what others wrote. The op is never cut short. If a relay,
or a proxy in front of it, refuses a request as too large anyway, the batch is halved and sent again.
The limit this covers is the relay's request size; a single op larger than what the relay's storage
holds per op (400 KB on DynamoDB) still fails as a server error.
[de]
Ein Sync-Push teilt einen Stapel jetzt nach Größe und nicht nur nach Anzahl auf. Eine Folge großer
Ops, etwa ein langer Text oder eine große Notiz, bleibt deshalb nicht mehr an der 8-MiB-Grenze des
Relays hängen. Bisher wurde so ein Stapel mit 413 abgelehnt und bei jedem Durchlauf unverändert
erneut geschickt, und nichts danach kam an. Eine einzelne Op, die selbst allein zu groß ist, lässt
den Durchlauf jetzt mit einer Fehlermeldung scheitern, die sie benennt. Alle Ops davor werden
trotzdem gepusht, und der Durchlauf holt weiter ab, was andere geschrieben haben. Die Op wird nie
gekürzt. Lehnt ein Relay, oder ein Proxy davor, eine Anfrage trotzdem als zu groß ab, wird der
Stapel halbiert und erneut geschickt. Abgedeckt ist damit die Anfragegröße des Relays; eine einzelne
Op, die größer ist, als der Speicher des Relays je Op fasst (400 KB bei DynamoDB), scheitert weiter
als Serverfehler.
