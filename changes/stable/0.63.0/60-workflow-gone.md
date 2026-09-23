---
type: removed
---
[en]
**The `nxc workflow` command group is gone, and so is the run engine behind it.** `workflow start`, `step done`, `status`, `list` and `liveness` no longer exist, and neither does the stored run record. A channel declares its flow now and `send --to <channel>` starts it; `nxc status` is where an operation's position is read. What has no successor is the step-liveness watchdog — a named loss, not an oversight.
[de]
**Die Befehlsgruppe `nxc workflow` ist weg, und mit ihr die Run-Engine dahinter.** `workflow start`, `step done`, `status`, `list` und `liveness` gibt es nicht mehr, und den gespeicherten Run-Datensatz auch nicht. Ein Kanal deklariert jetzt seinen Ablauf, `send --to <Kanal>` startet ihn, und `nxc status` ist die Stelle, an der man den Stand einer Operation liest. Ohne Nachfolger bleibt allein die Schritt-Lebendüberwachung — ein benannter Verlust, kein Versehen.
