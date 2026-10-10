---
type: added
facade: breaking
---
[en]
A plugin can now rank `next` differently for each item type. Under `[ranking.next]`, a table
`[ranking.next.<type>]` with its own `order = [...]` ranks the items of that type, in the same key
language as the default order (field keys and precedence keys). A type without its own table keeps
the default `order`. Once a plugin has such a table, items of different types are compared by type
first: by `types = [...]` under `[ranking.next]`, or, without it, by the precedence list of the
default order's first key on `type`. Every type with its own table must be in that list; a type
without one may be left out, and then sorts after the listed types, by name. The finish-first
tiers are unchanged: the per-type orders decide the order within a tier. `list --sort rank` and
`blocked` still use the default order. Plugins with only `[ranking.next]` and `order` rank exactly
as before, and so do both bundled plugins. A page token from `next --paginate` restarts when the
plugin's ranking has changed.

Plugins are now checked more strictly at load, which can refuse a plugin that loaded before: a
ranking key must name a field the ranking can read (a misspelt field, or a custom field, is
refused); any key under `[ranking.next]` other than `order`, `types` and per-type tables is
refused; `types` without any per-type table is refused; and a precedence list on `type` may only
name declared types, each once. Numbers now sort before text in a field key, so a field holding
both orders consistently. For apps, `RankSpec`, `TypeOrder` and `Ranking` are `#[non_exhaustive]`,
`RankSpec` gains the fields `types` and `by_type` and the helpers `order_for`, `type_ordinal`,
`cross_type_order`, `splits_by_type` and `validate_types`, and `record::RANKABLE_FIELDS` lists the
fields a ranking key may name.
[de]
Ein Plugin kann `next` jetzt für jeden Eintragstyp anders ordnen. Unter `[ranking.next]` ordnet eine
Tabelle `[ranking.next.<typ>]` mit eigenem `order = [...]` die Einträge dieses Typs, in derselben
Schlüsselsprache wie die Standardordnung (Feldschlüssel und Präzedenzschlüssel). Ein Typ ohne eigene
Tabelle behält die Standard-`order`. Sobald ein Plugin eine solche Tabelle hat, werden Einträge
verschiedener Typen zuerst nach dem Typ verglichen: über `types = [...]` unter `[ranking.next]`
oder, wenn das fehlt, über die Präzedenzliste des ersten Schlüssels auf `type` in der
Standardordnung. Jeder Typ mit eigener Tabelle muss in dieser Liste stehen; ein Typ ohne eigene
Tabelle darf fehlen und kommt dann nach den aufgeführten Typen, nach Namen geordnet. Die
Finish-first-Stufen bleiben unverändert: Die Ordnungen pro Typ entscheiden die Reihenfolge innerhalb
einer Stufe. `list --sort rank` und `blocked` nutzen weiterhin die Standardordnung. Plugins mit nur
`[ranking.next]` und `order` ordnen genau wie bisher, ebenso beide mitgelieferten Plugins. Ein
Seiten-Token von `next --paginate` beginnt neu, wenn sich die Ordnung des Plugins geändert hat.

Plugins werden beim Laden jetzt strenger geprüft, was ein bisher ladbares Plugin ablehnen kann: Ein
Ordnungsschlüssel muss ein Feld nennen, das die Ordnung lesen kann (ein falsch geschriebenes Feld
oder ein eigenes Feld wird abgelehnt); jeder Schlüssel unter `[ranking.next]` außer `order`, `types`
und Tabellen pro Typ wird abgelehnt; `types` ohne jede Tabelle pro Typ wird abgelehnt; und eine
Präzedenzliste auf `type` darf nur deklarierte Typen nennen, jeden einmal. Zahlen kommen in einem
Feldschlüssel jetzt vor Text, sodass ein Feld mit beidem widerspruchsfrei sortiert. Für Apps sind
`RankSpec`, `TypeOrder` und `Ranking` `#[non_exhaustive]`, `RankSpec` erhält die Felder `types` und
`by_type` sowie die Hilfsfunktionen `order_for`, `type_ordinal`, `cross_type_order`,
`splits_by_type` und `validate_types`, und `record::RANKABLE_FIELDS` listet die Felder, die ein
Ordnungsschlüssel nennen darf.
[migration.en]
Automatic: the bundled plugins and any plugin whose `[ranking.next]` holds only `order` over
canonical fields keep loading and ranking as before. Manual: a plugin refused at load must drop
stray keys under `[ranking.next]`, fix misspelt or custom fields in its ranking keys, and either add
a per-type table or drop `types`. Code that builds `RankSpec` or `Ranking` with a struct literal must
deserialize them from TOML instead, as `#[non_exhaustive]` forbids the literal outside the facade.
[migration.de]
Automatisch: Die mitgelieferten Plugins und jedes Plugin, dessen `[ranking.next]` nur `order` über
kanonische Felder enthält, laden und ordnen weiter wie bisher. Von Hand: Ein beim Laden abgelehntes
Plugin muss überzählige Schlüssel unter `[ranking.next]` entfernen, falsch geschriebene oder eigene
Felder in seinen Ordnungsschlüsseln korrigieren und entweder eine Tabelle pro Typ ergänzen oder
`types` entfernen. Code, der `RankSpec` oder `Ranking` als Struct-Literal baut, muss sie stattdessen
aus TOML deserialisieren, weil `#[non_exhaustive]` das Literal außerhalb der Fassade verbietet.
