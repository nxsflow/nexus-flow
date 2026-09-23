# Die Reise einer einzelnen Operation

[Architektur](develop-architecture) ist die Landkarte. Dies ist eine Fahrt darüber.

**Die Szene.** `bl0k` ist ein Ticket, das du gerade fertig hast; `fr33` wartet darauf. Du schließt
`bl0k` — und `fr33` wird das Nächste, was zu tun ist, ohne dass jemand das hinschreibt.

![Eine Operation vom Verb bis zur geänderten Antwort: nxf close, drei angehängte Zeilen, der Refold, und das Item, das als Nächstes drankommt](/nxs/docs/assets/the-journey-of-one-op.webp)

## Du schließt ein Ticket

```
nxf close bl0k --reason "ausgeliefert in #412"
```

Die CLI hält kein eigenes Modell. Sie ruft `Engine::close(now, actor, id, reason)` auf
`crates/facade` — dieselbe Naht, die eine einbettende App ruft. Es gibt keinen privaten Weg, den
die CLI nehmen kann und eine App nicht; deshalb zählt CLI-Abdeckung nie als Beleg für die Naht.

## Das Log wächst um drei Zeilen

`write::close` hängt an, statt zu schreiben: `set_field(status)`, `set_field(closing_comment)`,
`set_field(closed_at)`. Jedes Anhängen wird zu einer Zeile in der `ops`-Tabelle.

```
12  bl0k  status           in_progress        ← steht weiterhin da
13  bl0k  status           closed
14  bl0k  closing_comment  ausgeliefert in #412
15  bl0k  closed_at        2026-09-02T09:14Z
```

Nichts wurde überschrieben — und das gilt für jedes Verb davor. `nxf create` hängte fünf Zeilen je
Item an, `nxf dep add` eine Kantenzeile, `nxf claim` eine weitere. Das Log *ist* die Historie: jede
Op trägt ihren eigenen `author` und `wall_clock`, wer was geändert hat, ist also daraus ableitbar —
ohne eine zweite Prüftabelle, die man synchron halten müsste. Jede ist außerdem vom Replikat
**unterschrieben**, das sie angehängt hat — Schlüssel-Kennung und Unterschrift reisen mit der Op, und
ein empfangendes Replikat hält fest, ob sie stimmen —, „wer hat was geändert“ ist also beweisbar,
nicht nur lesbar. Der `author` ist der Name für die Anzeige; über eine Agenten-Aktion entscheidet der
Schlüssel.

## Die Sichten werden neu gefaltet

Ein Reducer faltet jede neue Op in die `items`-Sicht, nach keep-if-beats-LWW: eine Op gewinnt nur,
wenn ihr `(lamport, site)` das schlägt, was die Sicht schon hält. Zwei Maschinen, die dieselben Ops
in verschiedener Reihenfolge falten, landen deshalb auf derselben Zeile.

Die Sicht ist ein **Zwischenspeicher** des Logs. Lösche sie, und ein Refold baut sie wieder auf —
darum verschickt der Sync das Log und nie die Sicht.

## Die Antwort ändert sich

```console
$ nxf next
showing 1 of 1
bl0k  P2  in progress  [feature]  Ship the export path

$ nxf blocked
fr33  P2  open         [feature]  Publish the diagram
    ↳ blocked by: bl0k (in progress)
```

```console
$ nxf next
showing 1 of 1
fr33  P2  open         [feature]  Publish the diagram

$ nxf blocked
```

Das zweite `nxf blocked` gibt überhaupt nichts aus — die Lane ist leer, es gibt also keine Zeile zu
zeigen. Zwischen den beiden Läufen lag nichts als jene drei angehängten Zeilen. `blocked_gating`
(`crates/core/src/derive.rs`) hält ein Item nur zurück, solange eine Abhängigkeit noch nicht
geschlossen ist — `fr33` passt also nicht mehr auf das Muster: das Prädikat lief erneut, niemand
hat hineingeschrieben.

**Keine Zeile irgendwo sagt, welches Item als Nächstes kommt.** `nxf next` und `nxf blocked` sind
Fragen, jedes Mal neu gestellt — keine Spalten, die jemand pflegt. Genau das hält das Brett
konsistent, ohne dass es jemand pflegen müsste, und genau dafür zahlt das Schema seinen Preis: was
man fragen will, muss aus dem Log *ableitbar* sein.
