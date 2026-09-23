# memory — was das Projekt weiß

Durable Tatsachen, die die Sitzung überleben, in der sie gelernt wurden: eine Konvention, eine
Falle, der Grund, aus dem eine Entscheidung so ausging. `nxm remember` schreibt eine,
`nxm recall <key>` liest sie vollständig zurück, und ein stabiler Schlüssel lässt eine Tatsache **an
Ort und Stelle korrigieren**, statt widersprüchliche Kopien von ihr anzuhäufen.

Der Zweck ist nicht Ablage, sondern **Wiedervorlage**. Erinnerungen werden zu Beginn jeder Sitzung
zurückgegeben, damit der nächste Agent schon weiß, was der letzte gelernt hat. Nichts daran ist ein
Suchproblem: kein Modell auf dem Schreibpfad, keine Embeddings, kein Ähnlichkeitsranking — derselbe
Speicher antwortet heute wie in sechs Monaten gleich.

Der erste Schritt ist eine Zeile, und die Disziplin, die damit beginnt, ist eine harte Regel, die
das Sitzungsbanner wiederholt: durables Projektwissen geht in `nxm remember` und nirgendwo sonst.
Ein `MEMORY.md` oder eine andere zu diesem Zweck erfundene Datei wird nie zurückgelesen — das Wissen
darin ist still verloren.

```bash
nxm remember "Die vier cargo-Tore laufen in CI, nicht lokal" --key gates
nxm memories                  # das Verzeichnis — eine Zeile je Erinnerung
nxm recall gates              # eine Erinnerung, vollständig
```

Was dabei ankommt, ist dieses **Verzeichnis**, nicht die Erinnerungen selbst: je eine Zeile, die
sagt, was die Erinnerung *sagt*. Deshalb lohnt es sich, sie sorgfältig zu schreiben — niemand
schlägt eine Regel nach, bevor er sie bricht, also muss die Verzeichniszeile sie aussprechen.

## Wohin von hier

- [Erste Schritte](nxm-getting-started) — was memory braucht, bevor es laufen kann, und deine erste
  gemerkte Tatsache.
- [Kernkonzepte](nxm-core-concepts) — Schlüssel und Auto-Schlüssel, die drei Register, die
  Abrufregel und der Grabstein.
- [Agenten und MCP](nxm-agents-and-mcp) — die `memory_*`-Werkzeuge, die ein MCP-Host bekommt, und
  wohin ein Agent schreibt.
