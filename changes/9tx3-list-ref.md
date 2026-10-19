---
type: added
facade: changed
---
[en]
`nxf list --ref <field>=<id>` lists the items whose ref field holds the id — a person's meetings, stations or relationships — composed with every other `list` filter, sort, limit and page; repeat it for alternatives (at most 8). Archived items are never listed. `flow_list` takes `refs: [{field, id}]`. Library: `ListQuery::referencing`, `read::referrers_of`, `read::ref_filter_keeps`, `read::RefFilter`, `read::MAX_REFS`, `ListReads::refs`, `ListSelect::refs`.
[de]
`nxf list --ref <feld>=<id>` listet die Items, deren Verweisfeld die Id hält — die Meetings, Stationen oder Beziehungen einer Person — kombiniert mit jedem anderen `list`-Filter, der Sortierung, dem Limit und dem Blättern; mehrfach angegeben sind es Alternativen (höchstens 8). Archivierte Items werden nie gelistet. `flow_list` nimmt `refs: [{field, id}]`. Bibliothek: `ListQuery::referencing`, `read::referrers_of`, `read::ref_filter_keeps`, `read::RefFilter`, `read::MAX_REFS`, `ListReads::refs`, `ListSelect::refs`.
