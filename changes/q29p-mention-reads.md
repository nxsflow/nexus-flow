---
type: added
facade: changed
---
[en]
Mentions can now be read in both directions. `nxf mention by <id>` lists the items that mention an
item, in id order, as `list` records (closed and archived mentioners included). `--type`, `--in` and
`--label` narrow the list as they narrow `next`. `--limit` cuts it and says so, and `--paginate`
pages through it with a token, exactly as `next` does. Under `--json` you get a plain array, or
with `--limit` the `{"items", "total"}` envelope, plus `next_token` and `restarted` with
`--paginate`. An embedding app gets the same reads from the engine: `Engine::mentions(id)` returns
an item's outgoing targets, and `Engine::mentioned_by(id, &MentionedByQuery)` /
`Engine::mentioned_by_value` return the items that mention it. `read::mentioners_of` is the bulk
reverse lookup behind them. Everything is additive; no existing signature changes.
[de]
Erwähnungen lassen sich jetzt in beide Richtungen lesen. `nxf mention by <id>` listet die Items, die
ein Item erwähnen, in Id-Reihenfolge und als `list`-Datensätze (geschlossene und archivierte
eingeschlossen). `--type`, `--in` und `--label` grenzen die Liste ein wie bei `next`. `--limit`
kürzt sie und sagt das an, und `--paginate` blättert mit einem Token durch sie, genau wie bei
`next`. Mit `--json` kommt ein schlichtes Array, mit `--limit` die Hülle `{"items", "total"}`, und
mit `--paginate` zusätzlich `next_token` und `restarted`. Eine einbettende App bekommt dieselben
Lesezugriffe von der Engine: `Engine::mentions(id)` liefert die ausgehenden Ziele eines Items,
`Engine::mentioned_by(id, &MentionedByQuery)` und `Engine::mentioned_by_value` die Items, die es
erwähnen. `read::mentioners_of` ist die Massen-Rückwärtssuche dahinter. Alles ist additiv; keine
bestehende Signatur ändert sich.
