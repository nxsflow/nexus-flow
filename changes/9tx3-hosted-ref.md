---
type: added
---
[en]
A hosted board answers `list --ref` from the folded tables: the fold keeps a reverse-reference hint for every custom value shaped like an item id, exactly as the rows stand, and the read checks it against the referring item, so the result equals a replica's. Index revision 4: a hosted stream folded before this version is cleared and refolded once before it is served again — its readers are refused for that time, and every fold run must carry this version at once. No change to the table's indexes; the task fold revision is unchanged, so snapshots from the previous version still import.
[de]
Ein gehostetes Board beantwortet `list --ref` aus den gefalteten Tabellen: die Faltung hält zu jedem Feldwert in Id-Form einen Rückverweis-Hinweis, genau so wie die Zeilen stehen, und das Lesen prüft ihn gegen das verweisende Item, sodass das Ergebnis dem einer Replik gleicht. Index-Revision 4: ein gehosteter Stream, der vor dieser Version gefaltet wurde, wird einmal geleert und neu gefaltet, bevor er wieder bedient wird — seine Leser werden so lange abgewiesen, und jeder Fold-Lauf muss diese Version zugleich tragen. Keine Änderung an den Indizes der Tabelle; die Fold-Revision der Aufgaben bleibt, Snapshots der vorigen Version importieren weiterhin.
