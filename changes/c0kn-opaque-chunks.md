---
type: added
facade: changed
---
[en]
An item can now carry opaque chunks, a transport for content the engine does not read, such as the
updates of a text CRDT that the client assembles itself. `Engine::chunk_append` hangs a chunk on a
named chunk field of an item. `Engine::chunk_supersede` replaces chunks of that field with one new
chunk, which is how a writer compacts what it appended. `Engine::chunks` reads the field's live
chunks in a fixed order, and on a server `nxs_fold_ddb::chunks::chunks` reads the same list. Every
replica and the server converge on the same chunks whatever order the ops arrive in, and a supersede
that arrives first still hides what it replaces. A chunk op over 300 KiB (`MAX_CHUNK_BYTES`), an id
over 128 bytes, or a supersede of more than 1024 chunks is refused with a clear error, and a chunk
is never cut short. One that arrives from elsewhere anyway is kept in
the log but not folded, and the fold goes on. Chunks are not notes: they never appear in
`nxf show`, search, history or MCP text, and they do not change the item's `updated_at`. Opening a
workspace refolds its views once (task fold revision 2).
[de]
Ein Item kann jetzt opake Stücke tragen. Das ist ein Transportweg für Inhalte, die die Engine nicht
liest, etwa die Updates eines Text-CRDTs, die der Client selbst zusammensetzt.
`Engine::chunk_append` hängt ein Stück an ein benanntes Stückfeld eines Items.
`Engine::chunk_supersede` ersetzt Stücke dieses Feldes durch ein neues Stück, so verdichtet ein
Schreiber, was er angehängt hat. `Engine::chunks` liest die aktiven Stücke des Feldes in fester
Reihenfolge, auf einem Server liest `nxs_fold_ddb::chunks::chunks` dieselbe Liste. Alle Replikate
und der Server kommen bei denselben Stücken an, egal in welcher Reihenfolge die Ops eintreffen. Ein
Ersetzen, das zuerst ankommt, verbirgt das Ersetzte trotzdem. Eine Stück-Op über 300 KiB
(`MAX_CHUNK_BYTES`), eine ID über 128 Bytes oder ein Ersetzen von mehr als 1024 Stücken wird mit
klarer Fehlermeldung abgelehnt, und ein Stück wird nie gekürzt. Kommt trotzdem eine von
anderswo an, bleibt sie im Log, wird aber nicht gefaltet, und das Falten läuft weiter. Stücke sind
keine Notizen: Sie erscheinen nie in `nxf show`, in der Suche, im Verlauf oder im MCP-Text, und sie
ändern das `updated_at` des Items nicht. Beim Öffnen faltet ein Arbeitsbereich seine Ansichten
einmal neu (Task-Fold-Revision 2).
