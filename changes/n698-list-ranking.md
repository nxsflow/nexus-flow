---
type: added
facade: changed
---
[en]
`list` can now be ranked per type, narrowed like `next`, and paged. A plugin may declare
`[ranking.list.<type>]` tables (and `types = [...]` for the order of the types, plus an optional
default `order`) under `[ranking.list]` — the same keys, the same load-time checks and the same
comparator as `[ranking.next]`. `nxf list --sort rank` ranks by them; a type without a table, and
every plugin without `[ranking.list]`, ranks as before by the `next` default order. The default
`list` order stays id. `nxf list` also takes `--type` more than once (the types are alternatives),
`--in <id>` (the container's children and contributors), `--limit` (with a `showing <n> of <total>`
notice, and the `{"items", "total"}` envelope under `--json`) and `--paginate` / `--token`, which
page through the same machine-local cache `next` uses. Without `--limit`, `list --json` is the same
plain array as before. An embedding app gets the same read from the engine:
`Engine::list_query(&ListQuery)` and `Engine::list_query_value`; `Ranking::list` carries the
resolved list policy. Everything is additive; no existing signature changes.
[de]
`list` lässt sich jetzt pro Typ ranken, wie `next` eingrenzen und blättern. Ein Plugin kann unter
`[ranking.list]` Tabellen `[ranking.list.<typ>]` deklarieren (dazu `types = [...]` für die
Reihenfolge der Typen und optional eine eigene Standard-`order`) — dieselben Schlüssel, dieselben
Prüfungen beim Laden und derselbe Vergleich wie bei `[ranking.next]`. `nxf list --sort rank` rankt
danach; ein Typ ohne Tabelle und jedes Plugin ohne `[ranking.list]` rankt wie bisher nach der
Standardreihenfolge von `next`. Die Standardreihenfolge von `list` bleibt die Id. `nxf list` nimmt
außerdem `--type` mehrfach (die Typen sind Alternativen), `--in <id>` (die Kinder und Beitragenden
des Containers), `--limit` (mit dem Hinweis `showing <n> of <total>` und unter `--json` der Hülle
`{"items", "total"}`) und `--paginate` / `--token`, die durch denselben maschinenlokalen Cache
blättern wie `next`. Ohne `--limit` ist `list --json` dasselbe schlichte Array wie bisher. Eine
einbettende App bekommt denselben Lesezugriff von der Engine: `Engine::list_query(&ListQuery)` und
`Engine::list_query_value`; `Ranking::list` trägt die aufgelöste Listen-Policy. Alles ist additiv;
keine bestehende Signatur ändert sich.
