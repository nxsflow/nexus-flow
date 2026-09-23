---
type: removed
---
[en]
**A channel is a declaration now, or it is nothing.** The verbs that minted or addressed an undeclared channel are gone: `nxc ask` and the whole `nxc channels` group (`create`, `dm`, `join`, `leave`, `list`, `public`). Declare the channel and use `send --to <name>`; the declaration carries who is asked and what is expected, which is exactly what `ask --expect` and `channels create` used to say per call. The channel declaration key `member_session` is gone too — the call decides when a persona is continued.
[de]
**Ein Kanal ist jetzt eine Deklaration, oder er ist nichts.** Die Verben, die einen undeklarierten Kanal anlegten oder ansprachen, sind weg: `nxc ask` und die ganze Gruppe `nxc channels` (`create`, `dm`, `join`, `leave`, `list`, `public`). Deklarieren Sie den Kanal und benutzen Sie `send --to <Name>`; die Deklaration trägt, wer gefragt wird und was erwartet wird — genau das, was `ask --expect` und `channels create` vorher je Aufruf sagten. Der Deklarationsschlüssel `member_session` ist ebenfalls entfallen: wann eine Persona fortgesetzt wird, entscheidet der Aufruf.
