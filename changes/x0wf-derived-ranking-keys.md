---
type: added
facade: breaking
---
[en]
A plugin can now rank items by their neighbours in the graph, not only by their own fields.
Besides field keys, a per-type order (`[ranking.next.<type>]`, `[ranking.list.<type>]`, and the
`[ranking.list]` default order) takes three derived keys:

- `related` reads a value on the neighbours over one outbound edge: their rank in their own type's
  order (`of = "rank"`) or a field, a custom field included (`of = { field = "..." }`), picking the
  smallest or largest.
- `count` counts the neighbours over one edge kind and direction (`children`, `contributors`,
  `dependents`, `mentioned_by`, or an outbound kind) that pass its `type`, `status` and `label`
  filters.
- `sum` adds several counts with equal weight.

So actions can rank by their project's rank, projects by their goals' declared order and their
open children, and persons by how often projects and open waiting actions mention them. Ranks are
computed bottom-up through the chain of types, once per query, with one pass over the edges.
`count = "mentioned_by"` reads the same reverse mention lookup as `nxf mention by`. A plugin is
refused at load when a key mixes kinds, names an unknown edge, type, status or field, carries a
part its kind does not take, reads the rank of a type without an order, or when the types rank by
each other in a cycle. Derived keys are not allowed in the `[ranking.next]` default order, which
also ranks `blocked` and the lanes. A plugin without derived keys ranks exactly as before. For an
embedding app: `RankKey` gains the derived parts and is now `#[non_exhaustive]` and refuses
unknown parts (build one with `RankKey::field_key`); `ListQuery::now` and `read::list_at` take
the reference time a `list` order needs when it reads the rank of a type `next` ranks; and the
store-free `read::next_active` refuses a plugin whose `[ranking.next]` has derived keys until the
server path computes them.
[de]
Ein Plugin kann Items jetzt nach ihren Nachbarn im Graphen ranken, nicht nur nach ihren eigenen
Feldern. Neben Feldschlüsseln nimmt eine Reihenfolge pro Typ (`[ranking.next.<typ>]`,
`[ranking.list.<typ>]` und die Standardreihenfolge von `[ranking.list]`) drei abgeleitete
Schlüssel:

- `related` liest einen Wert an den Nachbarn über eine ausgehende Kante: ihren Rang in der
  Reihenfolge ihres eigenen Typs (`of = "rank"`) oder ein Feld, auch ein eigenes Feld
  (`of = { field = "..." }`), und nimmt den kleinsten oder größten.
- `count` zählt die Nachbarn über eine Kantenart und Richtung (`children`, `contributors`,
  `dependents`, `mentioned_by` oder eine ausgehende Art), die seine Filter `type`, `status` und
  `label` bestehen.
- `sum` addiert mehrere Zählungen mit gleichem Gewicht.

So können Aktionen nach dem Rang ihres Projekts ranken, Projekte nach der deklarierten
Reihenfolge ihrer Ziele und ihren offenen Kindern, und Personen danach, wie oft Projekte und offene
wartende Aktionen sie erwähnen. Ränge werden von unten durch die Kette der Typen berechnet, einmal
pro Abfrage, mit einem Durchgang über die Kanten. `count = "mentioned_by"` liest dieselbe
Rückwärtssuche der Erwähnungen wie `nxf mention by`. Ein Plugin wird beim Laden abgelehnt, wenn
ein Schlüssel Arten mischt, eine unbekannte Kante, einen unbekannten Typ, Status oder ein
unbekanntes Feld nennt, einen Teil trägt, den seine Art nicht nimmt, den Rang eines Typs ohne
Reihenfolge liest, oder wenn die Typen sich im Kreis gegenseitig ranken. Abgeleitete Schlüssel sind
in der Standardreihenfolge von `[ranking.next]` nicht erlaubt, die auch `blocked` und die Lanes
rankt. Ein Plugin ohne abgeleitete Schlüssel rankt genau wie bisher. Für eine einbettende App:
`RankKey` bekommt die abgeleiteten Teile, ist jetzt `#[non_exhaustive]` und lehnt unbekannte Teile
ab (bauen mit `RankKey::field_key`); `ListQuery::now` und `read::list_at` nehmen die Bezugszeit,
die eine `list`-Reihenfolge braucht, wenn sie den Rang eines Typs liest, den `next` rankt; und das
store-freie `read::next_active` lehnt ein Plugin ab, dessen `[ranking.next]` abgeleitete Schlüssel
hat, bis der Server-Pfad sie berechnet.
