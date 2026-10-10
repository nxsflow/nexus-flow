---
type: added
facade: changed
---
[en]
`nxf next` can now be narrowed and paged. `--type <type>` keeps only items of that type and can be
given more than once. `--in <id>` keeps only the items in a container: its children and the items
that contribute to it, so an item that belongs to one epic and contributes to another shows up under
both. The filters remove rows but never re-rank the rest. `--limit N` still cuts the list and states
the total. `--limit N --paginate` also gives you a token for the next page, and
`--paginate --token <token>` shows that page. Under `--json`, the token is in `next_token` and the
envelope also carries `restarted`. The list you are paging through is cached on this machine for an
hour and never synced. If an item in that list changes, or the token is stale or invalid, you get
the first page of a fresh list, with `restarted: true` and a new token. Items that qualify only
after the first page appear on your next fresh query. For apps, `Engine::next_query` and
`Engine::next_query_value` take a `read::NextQuery` with these filters and the paging settings.
`read::next_active_filtered` applies the same filters on a server, and `nxs_fold_ddb` now reads each
active ticket's contributes-to edges so that a server can use the container filter.
[de]
`nxf next` lässt sich jetzt eingrenzen und seitenweise lesen. `--type <typ>` behält nur Einträge
dieses Typs und kann mehrfach angegeben werden. `--in <id>` behält nur die Einträge in einem
Container: seine Kinder und die Einträge, die zu ihm beitragen. Ein Eintrag, der zu einem Epic gehört
und zu einem anderen beiträgt, erscheint also unter beiden. Die Filter entfernen Zeilen, ordnen den
Rest aber nie neu. `--limit N` kürzt die Liste weiterhin und nennt die Gesamtzahl.
`--limit N --paginate` liefert zusätzlich ein Token für die nächste Seite, und
`--paginate --token <token>` zeigt diese Seite. Mit `--json` steht das Token in `next_token`, und die
Antwort enthält außerdem `restarted`. Die Liste, durch die Sie blättern, wird eine Stunde lang auf
diesem Rechner zwischengespeichert und nie synchronisiert. Ändert sich ein Eintrag dieser Liste oder
ist das Token veraltet oder ungültig, erhalten Sie die erste Seite einer frischen Liste, mit
`restarted: true` und einem neuen Token. Einträge, die erst nach der ersten Seite dazukommen,
erscheinen bei der nächsten frischen Abfrage. Für Apps nehmen `Engine::next_query` und
`Engine::next_query_value` eine `read::NextQuery` mit diesen Filtern und den Seiteneinstellungen.
`read::next_active_filtered` wendet dieselben Filter auf einem Server an, und `nxs_fold_ddb` liest
jetzt die Beitragskanten jedes aktiven Tickets, damit ein Server den Container-Filter nutzen kann.
