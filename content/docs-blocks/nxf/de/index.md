# flow — was zu tun ist

Der Issue-Tracker der Suite: Items, die Abhängigkeiten zwischen ihnen, Fälligkeits- und
Aufschubdaten, ein append-only Notizstrom und der Grund, aus dem ein Item geschlossen wurde. Das ist
die Aufzeichnung dessen, was ein Projekt tut — und warum.

Eine Eigenschaft unterscheidet ihn von einer Ticket-Liste, und sie gehört vor alles andere: **die
Arbeitsliste wird abgeleitet, nicht gespeichert**. `nxf next` und `nxf blocked` werden in dem
Moment, in dem du fragst, aus dem Abhängigkeitsgraphen berechnet; sie können deshalb weder veralten
noch ihm widersprechen. Niemand setzt ein Item auf *machbar* — das Schließen seiner letzten
Vorbedingung macht es dazu.

Das prägt den ersten Schritt. Er lautet nicht „schreib eine Aufgabe", sondern *halte fest, was es
gibt und worauf es wartet*:

```bash
nxf create --type feature --title "Release 1.0 ausliefern" --priority P1
nxf dep add <item> <vorbedingung>
nxf next
```

Was ein Item *ist*, kommt aus einem **Plugin**: die Typen, die du anlegen darfst, das Vokabular, das
du liest und schreibst, und die Rangfolge hinter `next`. Die Wahl fällt einmal je Arbeitsbereich,
weil sie alles prägt, was du danach tippst — mitgeliefert werden ein Issue-Tracker für
Code-Projekte und eine persönliche To-do-Liste, beide über derselben Engine.

Jeder Befehl antwortet auf `--json` mit byte-stabiler Ausgabe, und das ist die Fläche, auf der
Agenten bauen: ein Agent kann das ganze Brett bedienen, ohne dass vorher ein Mensch eine Tabelle
liest.

## Wohin von hier

- [Erste Schritte](nxf-getting-started) — vom leeren Verzeichnis zu einem geplanten, verfolgten
  Stück Arbeit.
- [Kernkonzepte](nxf-core-concepts) — Items, Abhängigkeiten, und wie die beiden Spuren abgeleitet
  statt gespeichert werden.
- [Aufschieben und warten](nxf-deferring-and-waiting) — die eine Unterscheidung, die leicht
  schiefgeht: ein echtes Kalenderdatum gegenüber dem Warten auf etwas, das erst geliefert werden
  muss.
