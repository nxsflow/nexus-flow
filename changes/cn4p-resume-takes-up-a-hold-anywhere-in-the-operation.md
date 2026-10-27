---
type: fixed
facade: changed
---
[en]
`nxc resume --thread <id>` now takes up a hold on any session of the operation that thread belongs to, including the root that `nxc status` shows. Until now it only looked at sessions recorded on that exact thread. A channel root has no session of its own, so a step that hit a usage limit (even on its first turn) answered `nothing_on_hold`, with or without `--force`, while `nxc status` showed the operation as interrupted. If several sessions of one operation are on hold, the one that has waited longest is taken up first, and the receipt names its thread. `nxc status` now marks an operation interrupted exactly when `nxc resume --thread` finds a hold in it. `--session` is unchanged.

A resumed session that runs straight into the next limit is now recorded as on hold again. Before, its new hold could be lost while the resume was still recording the old one. That left nothing on hold and nothing coming. If the resumed session fails to start, its hold stays in place.
[de]
`nxc resume --thread <id>` nimmt jetzt einen Haltezustand auf jeder Sitzung des Vorgangs auf, zu dem dieser Faden gehört, auch auf der Wurzel, die `nxc status` zeigt. Bisher sah der Befehl nur Sitzungen an, die genau auf diesem Faden verzeichnet waren. Eine Kanal-Wurzel hat keine eigene Sitzung. Lief ein Schritt in ein Nutzungslimit (auch im ersten Zug), antwortete der Befehl deshalb mit `nothing_on_hold`, mit oder ohne `--force`, obwohl `nxc status` den Vorgang als unterbrochen zeigte. Stehen mehrere Sitzungen eines Vorgangs, wird die zuerst aufgenommen, die am längsten wartet, und die Quittung nennt ihren Faden. `nxc status` markiert einen Vorgang jetzt genau dann als unterbrochen, wenn `nxc resume --thread` darin einen Haltezustand findet. `--session` bleibt unverändert.

Läuft eine wieder aufgenommene Sitzung sofort ins nächste Limit, gilt sie jetzt wieder als angehalten. Bisher konnte ihr neuer Haltezustand verloren gehen, während das Fortsetzen noch den alten verbuchte. Dann stand nichts mehr an, und es kam auch nichts mehr. Startet die wieder aufgenommene Sitzung nicht, bleibt ihr Haltezustand stehen.
