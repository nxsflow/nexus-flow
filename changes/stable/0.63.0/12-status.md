---
type: added
---
[en]
**`nxc status` — where an operation stands.** An operation is a thread tree now: your question, the round it opened, the round that round opened. `nxc status` reads that tree from its root down, across every channel border, and says per thread who still owes an answer, whether a window has run out, and which session is working on it. `awaiting_human` marks the one place a person has to act. Whoever opened an operation can also read every message in it, whatever channel an agent opened along the way.
[de]
**`nxc status` — wo eine Operation steht.** Eine Operation ist jetzt ein Fadenbaum: Ihre Frage, die Runde, die sie eröffnet hat, die Runde, die jene eröffnet hat. `nxc status` liest diesen Baum von der Wurzel abwärts, über jede Kanalgrenze, und sagt je Faden, wer noch eine Antwort schuldet, ob ein Fenster abgelaufen ist und welche Sitzung daran arbeitet. `awaiting_human` markiert die eine Stelle, an der ein Mensch handeln muss. Wer eine Operation eröffnet hat, kann außerdem jede Nachricht darin lesen, in welchem Kanal ein Agent sie auch eröffnet hat.
