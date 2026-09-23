# Architektur

Diese Seite ist die Landkarte: was die Teile sind, wohin die Pfeile zeigen und welche Nähte
Verträge sind. Sie ist absichtlich für sich allein lesbar — das Bild darunter illustriert sie, es
trägt sie nicht. Das Terminal kann dir kein Bild zeigen, und ein Agent, der `nxs guide` liest,
verdient dieselbe Darstellung wie du.

![Die nexus-flow-Architektur auf einen Blick: Aufrufer, Oberflächen, Engines, das geteilte op-Log und die Sync-Kante](/nxs/docs/assets/nexus-flow-simplified.webp)

Das gerenderte Diagramm steht auf der Seite zu diesem Thema unter
`nxsflow.com/docs/develop/architecture`; im Terminal liest du die Textfassung, die dasselbe
sagt.

## Fünf Schichten, von oben nach unten

**1 — Wer mit dem System spricht.** Coding-Agenten (jeder aktive Baustein installiert seinen
eigenen SessionStart-Hook), Menschen am Terminal, MCP-Wirte und einbettende Apps wie manufakt.io
und nexflow.it.

**2 — Oberflächen.** EINE Binary, vier Personas, verteilt über `argv[0]`: `nxs` ist die Klammer
(`init`, `prime`, `doctor`, `status`, `sync`, `self-update`, `mcp serve`), und `nxf`, `nxm`, `nxc`
sind die drei Bausteine. Sie sind dünne Aufrufer — keine Oberfläche hält ein eigenes Modell.

**3 — Engines.** flow, memory und chat, jede hinter ihrer eigenen Engine-Naht. Nur die von flow
(`crates/facade`) ist ein SemVer-Vertrag; die anderen beiden sind Nähte ohne dieses Versprechen.
Plugins registrieren sich hier zur Kompilierzeit — so liefert ein Konsument ein Vokabular, das die
Open-Source-Engine selbst nie ausliefert.

**4 — Substrat.** Ein Arbeitsbereich, ein Log. `.nxs/db.sqlite` ist git-ignoriert und
gerätelokal, und eine append-only-Tabelle `ops` darin ist die einzige Quelle der Wahrheit. Jedes
Modul hängt an DASSELBE Log an.

**5 — Die Kante nach außen.** Ein Anti-Entropie-Client, angetrieben von einem Hintergrunddienst,
schiebt lokale Ops zum `nxf-relay` — einem dummen, dauerhaften Op-Träger auf SQLite oder Postgres —
wo andere Replikate über denselben Strom konvergieren. Der Server speichert und gibt Ops zurück; er
faltet, leitet und validiert nie.

Zwei Pfeile lohnen die eigene Erwähnung. Eine einbettende App linkt die **Engine-Naht direkt** und
ruft nie die CLI auf — App und CLI sind Gleiche über einem Kern, weshalb jedes CLI-Verb eine
Entsprechung auf der Naht hat. Und der Substrat-Pfeil läuft nur in eine Richtung:

```
ops (append-only)  →  Fold  →  materialisierte Views  →  Ableitung
```

`nxf next` und `nxf blocked` werden **in SQL über den Views berechnet, nie gespeichert**. Genau
das hält die Arbeitsliste konsistent, egal in welcher Reihenfolge Ops eintrafen — und deshalb kann
das Zusammenführen zweier Replikate kein Brett erzeugen, das sich selbst widerspricht.

## Was das Bild nicht zeigt

Zweierlei, und beides mit Absicht.

Es ist eine **Bausteinsicht**: sie sagt, was die Teile sind und in welche Richtung sie einander
aufrufen — nicht, was über die Zeit geschieht. Alles oben ist die Landkarte; nichts davon ist die
Reise — und die Reise hat eine eigene Seite:
[die Reise einer einzelnen Operation](develop-the-journey-of-one-op) verfolgt ein einzelnes Verb
von der Oberfläche hinunter ins Log und wieder hinauf zu einer Antwort, die niemand gespeichert
hat.

Und sie ist **grob auf Crate-Ebene**: sechs Blöcke für eine Crate-Landkarte mit vierzehn.
`nxs-foundation`, das Innere des Sync-Clients, der Hintergrunddienst, der ihn antreibt, und die
Speicher-Backends des Relays stecken jeweils in dem Band, das sie besitzt, statt gezeichnet zu
sein.

Für die Schicht, die du als Nächstes brauchst: [der Arbeitsbereich](nxs-the-workspace) sagt, was
`nxs init` auf der Platte hinterlässt, und [Module](nxs-modules) sagt, zu welchem Baustein du
greifst.
