---
type: added
facade: changed
---
[en]
A plugin can now rank `next` differently for each item type. Under `[ranking.next]`, a table
`[ranking.next.<type>]` with its own `order = [...]` ranks the items of that type, in the same key
language as the default order (field keys and precedence keys). A type without its own table keeps
the default `order`. Items of different types are compared by `types = [...]` under
`[ranking.next]`, the order of the types; without it, the default order's precedence key on `type`
is used. The finish-first tiers are unchanged: the per-type orders decide the order within a tier.
The plugin is refused at load if a per-type table names a type the plugin does not declare, if it
holds a bad key, or if there is no order of the types. Plugins with only `[ranking.next]` and
`order` rank exactly as before, and so do both bundled plugins. A page token from
`next --paginate` restarts when the plugin's ranking has changed. For apps, `RankSpec` gains
`types`, `by_type` and the helpers `order_for`, `type_ordinal`, `cross_type_order` and
`splits_by_type`; `list --sort rank` and `blocked` still use the default order.
[de]
Ein Plugin kann `next` jetzt für jeden Eintragstyp anders ordnen. Unter `[ranking.next]` ordnet eine
Tabelle `[ranking.next.<typ>]` mit eigenem `order = [...]` die Einträge dieses Typs, in derselben
Schlüsselsprache wie die Standardordnung (Feldschlüssel und Präzedenzschlüssel). Ein Typ ohne eigene
Tabelle behält die Standard-`order`. Einträge verschiedener Typen werden über `types = [...]` unter
`[ranking.next]` verglichen, die Reihenfolge der Typen; fehlt sie, gilt der Präzedenzschlüssel auf
`type` aus der Standardordnung. Die Finish-first-Stufen bleiben unverändert: Die Ordnungen pro Typ
entscheiden die Reihenfolge innerhalb einer Stufe. Das Plugin wird beim Laden abgelehnt, wenn eine
Tabelle pro Typ einen Typ nennt, den das Plugin nicht deklariert, wenn sie einen ungültigen
Schlüssel enthält oder wenn keine Reihenfolge der Typen angegeben ist. Plugins mit nur
`[ranking.next]` und `order` ordnen genau wie bisher, ebenso beide mitgelieferten Plugins. Ein
Seiten-Token von `next --paginate` beginnt neu, wenn sich die Ordnung des Plugins geändert hat. Für
Apps erhält `RankSpec` die Felder `types` und `by_type` sowie die Hilfsfunktionen `order_for`,
`type_ordinal`, `cross_type_order` und `splits_by_type`; `list --sort rank` und `blocked` nutzen
weiterhin die Standardordnung.
