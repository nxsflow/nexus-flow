---
type: changed
---
[en]
**`nxc` has one way to start a conversation and one way to answer it.** `send --to` opens a thread and always starts a fresh persona session; `reply --thread` posts into an existing one and continues the session it already has. The call decides, not the declaration. The per-call options that used to sit beside them — `--kind`, `--priority`, `--disposition`, `--model`, `--deadline` — are gone: what a message is, and how long a round may wait, is declared once on the channel rather than re-answered on every call.
[de]
**`nxc` hat einen Weg, ein Gespräch zu beginnen, und einen, es zu beantworten.** `send --to` eröffnet einen Faden und startet immer eine frische Persona-Sitzung; `reply --thread` postet in einen bestehenden und setzt die Sitzung fort, die er schon hat. Der Aufruf entscheidet, nicht die Deklaration. Die Optionen, die daneben standen — `--kind`, `--priority`, `--disposition`, `--model`, `--deadline` — sind weg: was eine Nachricht ist und wie lange eine Runde warten darf, wird einmal am Kanal deklariert statt bei jedem Aufruf neu beantwortet.
