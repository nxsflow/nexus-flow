---
type: fixed
---
[en]
**Sync finishes what it started.** A pull no longer ends a pass because a page came back empty or shorter than asked for — it ends when the relay says there is no more. A stream id containing `/`, `#` or `?` used to produce a 404 or a silently truncated id instead of syncing. And the relay now carries envelope fields it does not itself understand, so a newer client and an older relay keep working together.
[de]
**Sync bringt zu Ende, was es beginnt.** Ein Pull beendet einen Durchlauf nicht mehr, weil eine Seite leer oder kürzer als angefordert zurückkam — er endet, wenn das Relay sagt, dass nichts mehr da ist. Eine Stream-Id mit `/`, `#` oder `?` erzeugte bisher einen 404 oder eine still abgeschnittene Id, statt zu synchronisieren. Und das Relay trägt jetzt Umschlagfelder weiter, die es selbst nicht versteht — ein neuerer Client und ein älteres Relay arbeiten damit weiter zusammen.
