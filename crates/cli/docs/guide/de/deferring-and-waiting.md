# Zurückstellen und Warten

Zwei Dinge sehen ähnlich aus — „jetzt nicht" und „nicht, bis etwas anderes passiert" —, aber
nexus-flow modelliert sie sehr unterschiedlich. Wer die Unterscheidung richtig trifft, hält die
Lanes `deferred` und `next` ehrlich, sodass die abgeleitete Arbeitsliste verlässlich bleibt.

## Defer ist nur für echte Kalenderdaten

`defer_until` blendet ein Item aus, bis ein **Datum** erreicht ist. Nutze es nur, wenn es einen
echten Kalendertag gibt, vor dem die Arbeit sinnvollerweise nicht beginnen kann — eine Frist öffnet,
ein Quartal startet, ein Embargo fällt:

```bash
# ein echtes Datum: erst zum Quartalsbeginn auftauchen lassen
nxf create --type chore --title "Quartalssteuer zahlen" --priority P3 --defer 2026-07-01
nxf update <id> --set defer=2026-07-01
```

Vor dem Datum liegt das Item in `nxf deferred` und wird aus `nxf next` herausgehalten; am Datum wird
es automatisch ready. Dieses Verhalten ist nur dann sinnvoll, wenn das Datum echt ist.

**Anti-Pattern: ein Platzhalter-Datum.** Wähle *kein* willkürliches Zukunftsdatum, um „irgendwann,
sobald X liefert" auszudrücken. Du weißt nicht, wann X liefert — jedes erfundene Datum ist also
falsch: zu früh, und das Item springt zurück in `next`, bevor es machbar ist; zu spät, und es bleibt
verborgen, nachdem es längst ready war. Ein Defer-Datum, das du ständig nachschieben müsstest, ist
das Warnsignal — das ist kein Kalender-Ereignis, sondern eine *Abhängigkeit*, und die will das Muster
unten.

## Auf eine Lieferung warten: das WARTE-Chore

nexus-flow hat **keine workspace-übergreifenden Abhängigkeiten** — ein Item in diesem Workspace kann
nicht auf ein Item im Board eines anderen Repos `dep`-en. Wenn deine Arbeit also auf eine externe
**Lieferung** wartet (ein Release in einem anderen Repo, die API eines Partners, ein Upstream-Fix),
modellierst du diese Lieferung als erstklassiges Item *im eigenen Workspace*: ein offenes
**WARTE-Chore** (englisch im Board mit dem Präfix `WAIT:`).

Das Chore steht stellvertretend für das, worauf du wartest. Gib ihm ein `WAIT:`-Präfix, damit es
unverkennbar ist, lass jedes abhängige Item darauf zeigen, und lass seinen Lebenszyklus die Kette
steuern:

```bash
# 1. ein Anker für die Lieferung, auf die du wartest
nxf create --type chore --title "WAIT: acme-api v2 shipped (need the /batch endpoint)" --priority P2 -q
# → 6j6v.w8t2

# 2. die Arbeit, die es braucht, hängt vom Anker ab (erscheint dann als blocked, nicht ready)
nxf dep add 6j6v.k1a9 6j6v.w8t2
nxf dep add 6j6v.p3f0 6j6v.w8t2

# 3. wenn die Lieferung da ist, SCHLIESSE den Anker — die gelieferte Version in die Begründung
nxf close 6j6v.w8t2 --reason "acme-api v2.3.0 shipped with /batch; verified against staging"
```

Das Schließen des Chores ist das **Ereignis**, das die Kette entsperrt: In dem Moment, in dem es
schließt, ist alles, was davon abhing, nicht mehr blockiert und kehrt in `nxf next` zurück. Die
Begründung — mit der konkreten Version — ist der dauerhafte Nachweis, *was* geliefert wurde und
*wann*, genau dort, wo der nächste Agent nachsieht.

Jeder Workspace hält **seinen eigenen** Anker für dieselbe Upstream-Lieferung; es gibt kein geteiltes
repo-übergreifendes Ticket. Das ist bewusst so — jedes Board bleibt in sich geschlossen und
konvergiert für sich.

## Warum ein WARTE-Chore keine „normale Arbeit" ist

Ein WARTE-Chore ist technisch **ready** (es ist offen und unblockiert, kann also in `nxf next`
auftauchen), aber seine Aktion ist nicht „bauen" — sondern „prüfen, ob die Lieferung da ist, und
wenn ja, mit der Version schließen". Lies ein `WAIT:`-Item so: ein wiederkehrender *Check*, keine
Aufgabe zum Hinsetzen-und-Machen. Das konsistente Präfix ist genau das, was dir (und künftigem
Tooling) erlaubt, beide auf einen Blick zu unterscheiden.

## In einem Satz

- **Defer** → ein echtes Kalenderdatum, sonst nichts.
- **Auf eine Lieferung warten** → ein offenes `WAIT:`-Chore, auf das deine Items `dep`-en,
  geschlossen (mit der Version), sobald es geliefert wird.
