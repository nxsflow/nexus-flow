---
type: fixed
facade: none
---
[en]
A paged read (`next`, `list` or `mention by` with `--limit --paginate`) that is asked for first pages several times within one second now always evicts its oldest cached result first. Before, results from the same second were evicted in an arbitrary order, so a token handed out a moment ago could restart at the first page while an older one kept working.
[de]
Eine geblätterte Abfrage (`next`, `list` oder `mention by` mit `--limit --paginate`), die innerhalb einer Sekunde mehrmals nach der ersten Seite gefragt wird, verdrängt jetzt immer zuerst ihr ältestes zwischengespeichertes Ergebnis. Bisher wurden Ergebnisse aus derselben Sekunde in beliebiger Reihenfolge verdrängt, sodass ein gerade ausgegebenes Token wieder bei der ersten Seite beginnen konnte, während ein älteres weiter funktionierte.
