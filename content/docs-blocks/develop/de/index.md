# Darauf bauen, oder mitbauen

Alles bis hierher beantwortet *wie benutze ich das*. Dieser Teil beantwortet *wie ist es gebaut, und
worauf darf ich bauen* — und dafür stehen zwei Türen offen.

**Darauf bauen.** nexus-flow ist zuerst eine Bibliothek. Eine Anwendung bindet `nexus-flow-facade`
ein, hält eine `Engine` und liest genau die Spuren, die auch die CLI liest — im selben Prozess, ohne
Unterprozess und ohne Daemon dazwischen. Das überlässt das Repository nicht der Prosa:
`examples/tauri-board/` ist eine kleine, echte Desktop-Anwendung, die genau das tut, in etwa 120
Zeilen Rust und einer HTML-Datei. Ihr Fenster listet die Arbeit und zeichnet sich von selbst neu,
sobald irgendetwas anderes in denselben Arbeitsbereich schreibt — weil sie den Änderungsstrom der
Engine abonniert, statt ihn abzufragen. Wer hier ist, um eine eigene Oberfläche daraufzusetzen, nimmt
dieses Beispiel als kürzesten Weg hinein.

**Mitbauen.** Das Projekt nimmt Beiträge an. Die Engine, die CLIs, die Anleitungen, die du gerade
liest, und die Tests, die sie ehrlich halten, liegen in einem Cargo-Workspace, und die ganze
Qualitätslatte sind vier Befehle, die du auf deiner eigenen Maschine fahren kannst — die nächste
Seite zählt sie auf und sagt, welchen davon CI wann fährt. Die Nähte sind beschrieben statt geraten
— dafür sind die Seiten unten da.

Beide Türen teilen sich den ersten Schritt: die Quellen holen, bauen, und etwas laufen lassen.
[Erste Schritte](develop-getting-started) ist dieser Schritt, für beide Fälle.

## Wohin von hier

- [Erste Schritte](develop-getting-started) — die Quellen auf deiner Maschine, die Tore und die
  Beispiel-Anwendung.
- [Architektur](develop-architecture) — die Landkarte: fünf Schichten, wohin die Pfeile zeigen, und
  welche Nähte Verträge sind.
- [Die Reise einer Operation](develop-the-journey-of-one-op) — ein `nxf close`, verfolgt vom Verb
  bis zu einer abgeleiteten Antwort, die niemand geschrieben hat — und danach
  [dieselbe Fahrt über zwei Maschinen](develop-a-reply-across-two-machines).
