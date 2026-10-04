---
type: added
facade: breaking
---
[en]
A persona can commission a persona of another workspace on the same machine, and the answer comes
back. `nxc send --to <owner>/<repo>/<persona>` records the commission in the caller's own workspace
and hands over that one thread; the other workspace checks access, starts its persona in its own
working copy, and the answer — or a question, which always goes to whoever commissioned — wakes the
caller. Only the border thread crosses. Access is the receiver's: `addressable.external` admits a
role from trusted workspaces (`"*/pm"`, `nxsflow/manufakt-io/pm`), and without it nothing comes in.
New: `nxs name` (a workspace's stored `<owner>/<repo>` name), `nxs sync trust add --workspace
<owner>/<repo>` (trust a neighbouring workspace by name; needed on both sides), and `nxc status`
showing a border thread in both workspaces with the other side and its state (submitted, working,
input-required, completed, rejected, canceled). Two hours without a sign of life cancel it;
`nxc withdraw` takes it back on both sides. An engine older than this release cannot load a persona
file that carries `external:` — add it only once every reader of the repository is on this version.
For embedding apps: `EngineConfig::peers` and `Ctx::peers` (the host's way to reach other
workspaces; `None` keeps everything as before), `Engine::handover`, `Addressable::OnlyWithExternal`,
`Coordinator::Border`, and new `border` fields on `Refs`, `SendToReceipt`, `ReplyReceipt` and
`StatusThread` — a struct literal of `EngineConfig`, `Ctx`, `Adapter` or `Refs` without
`..Default::default()` needs the new field.
[de]
Eine Persona kann eine Persona eines anderen Arbeitsbereichs auf derselben Maschine beauftragen,
und die Antwort kommt zurück. `nxc send --to <besitzer>/<repo>/<persona>` legt den Auftrag im
eigenen Arbeitsbereich ab und übergibt genau diesen einen Faden; der andere Arbeitsbereich prüft den
Zugang, startet seine Persona in seiner eigenen Arbeitskopie, und die Antwort — oder eine
Rückfrage, die immer an den Auftraggeber geht — weckt den Aufrufer. Nur der Grenzfaden geht hinüber.
Den Zugang bestimmt der Empfänger: `addressable.external` lässt eine Rolle aus vertrauten
Arbeitsbereichen zu (`"*/pm"`, `nxsflow/manufakt-io/pm`), ohne das Feld kommt nichts herein. Neu:
`nxs name` (der gespeicherte Name `<besitzer>/<repo>` eines Arbeitsbereichs), `nxs sync trust add
--workspace <besitzer>/<repo>` (einem benachbarten Arbeitsbereich beim Namen vertrauen; auf beiden
Seiten nötig) und `nxc status`, das einen Grenzfaden in beiden Arbeitsbereichen mit der Gegenseite
und seinem Zustand zeigt (submitted, working, input-required, completed, rejected, canceled). Zwei
Stunden ohne Lebenszeichen brechen ihn ab; `nxc withdraw` zieht ihn auf beiden Seiten zurück. Eine
Engine, die älter ist als dieses Release, kann eine Persona-Datei mit `external:` nicht laden —
setzen Sie das Feld erst, wenn alle Leser des Repositorys diesen Stand haben. Für einbettende Apps:
`EngineConfig::peers` und `Ctx::peers` (der Weg des Hosts zu anderen Arbeitsbereichen; `None` lässt
alles wie bisher), `Engine::handover`, `Addressable::OnlyWithExternal`, `Coordinator::Border` und neue
Felder `border` an `Refs`, `SendToReceipt`, `ReplyReceipt` und `StatusThread` — ein Struct-Literal
von `EngineConfig`, `Ctx`, `Adapter` oder `Refs` ohne `..Default::default()` braucht das neue Feld.
