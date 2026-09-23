# Agenten und MCP

memory hat drei Nähte, und sie bedienen denselben Speicher: die `nxm`-CLI (was ein Agent mit einer
Shell tippt), die **MCP-Werkzeuge** (was ein MCP-nativer Host aufruft) und `nxs prime` (was einer
Sitzung übergeben wird, bevor sie irgendetwas fragt). Diese Anleitung handelt von der zweiten und
der dritten — die erste steht in [Befehle](nxm-commands).

Die Regel, die alles zusammenhält: **die Antwort eines MCP-Werkzeugs ist byte-gleich zum
entsprechenden `nxm … --json`** beim selben Zustand. Nicht „gleichwertig" — dieselben Bytes, gehalten
von Paritätstests über die Naht hinweg. Was Sie an der CLI über den Datensatz gelernt haben, gibt
ein Werkzeug also Feld für Feld genauso zurück.

## Einen Host anbinden

Der Server ist `nxs mcp serve`, und er gehört der Suite — ein Server für flow *und* memory über den
einen gemeinsamen `.nxs/`-Speicher. Wie man ihn in Claude Desktop, Cursor oder Windsurf einträgt,
auch wenn noch nichts installiert ist, steht genau einmal in flows Anleitung:
[Einen MCP-Host anbinden](nxf-mcp). Mit `nxs` im `PATH` ist es ein Befehl:

```bash
nxs mcp install
```

**Die memory-Werkzeuge hängen daran, dass das Modul memory aktiv ist** — in dem Arbeitsbereich, mit
dem der Server gestartet wurde. Ein Arbeitsbereich mit nur flow zeigt überhaupt keine
`memory_*`-Werkzeuge: die registrierte Oberfläche ist der Fan-out über die aktiven Bausteine, keine
feste Liste. Fehlen die Werkzeuge, lautet die Antwort fast immer `nxm init` (oder
`nxs init --module memory`) in diesem Arbeitsbereich.

Geben Sie einem angebundenen Host das Vertrauen, das Sie einer Shell in diesem Arbeitsbereich geben
würden: das sind Schreibvorgänge, nicht nur Lesevorgänge.

## Die Werkzeugoberfläche

Acht Werkzeuge — drei lesend, fünf schreibend — jedes das genaue Gegenstück eines CLI-Befehls:

| Werkzeug | entspricht | Anmerkung |
| --- | --- | --- |
| `memory_list` | `nxm memories --json` | alle aktiven Erinnerungen, nach Schlüssel sortiert |
| `memory_search` | `nxm memories <query> --json` | Teilstring über Schlüssel + Text, Groß-/Kleinschreibung egal |
| `memory_show` | `nxm recall <key> --json` | ein Datensatz, oder ein `not_found`-Werkzeugfehler |
| `memory_add` | `nxm remember <text> --introduction <zeile> --json` | der Inhaltshash-Auto-Schlüssel; kein `key`-Parameter |
| `memory_update` | `nxm remember <text> --key <key> --introduction <zeile> --json` | an Ort und Stelle unter einem stabilen Schlüssel |
| `memory_classify` | `nxm classify <key> --json` | einordnen oder die `introduction` neu schreiben; der Text bleibt unberührt |
| `memory_reorder` | `nxm reorder <key>… --json` | das Array der Datensätze in der geschriebenen Reihenfolge |
| `memory_close` | `nxm forget <key> --json` | die `{ok, key}`-Quittung, kein Grabstein-Datensatz |

`memory_add` und `memory_update` nehmen dasselbe optionale Trio entgegen, das die CLI-Flags setzen —
`category`, `scope`, `refs` —, ein Fakt lässt sich also in einem Aufruf festhalten und einordnen.
Wer keines der drei sendet, schreibt die Vorgabewerte.

**`introduction` ist bei beiden Pflicht**, genau wie `--introduction` auf der CLI: eine Zeile,
höchstens 200 Zeichen, das Einzige an der Erinnerung, was ein Sitzungsstart je zu sehen bekommt.
`memory_classify` nimmt sie ebenfalls, optional — so bekommt eine vor diesem Feld geschriebene
Erinnerung ihre Zeile.

**Warum die Schreib-Verben so heißen**: `add` prägt einen Inhaltshash-Schlüssel, `update` verlangt
einen — die Unterscheidung steht also im Namen des Werkzeugs und versteckt sich nicht darin, ob ein
Parameter gesendet wurde. `close` folgt derselben Hauskonvention wie flows Item-Verben und ist wie
`forget` umkehrbar.

### Parameter, die keine Flags sind

Drei Parameter gibt es nur an dieser Naht:

- **`now`** — der Zeitstempel, den ein Schreibvorgang als `updated` setzt. Ausdrücklich statt
  implizit, damit ein Aufrufer, der reproduzierbare Ausgaben braucht, einen hat.
- **`actor`** — der Autor, der eingetragen wird. Ohne ihn schreibt der Server der Klammer unter
  seiner einen `nxs`-Identität (`--actor` / `NXS_ACTOR` → `NXF_ACTOR`/`USER` → `nxs`), *nicht* unter
  der `NXM_ACTOR`-Vorgabe der CLI. Diese Abweichung ist Absicht: ein stdio-Server schreibt für jeden
  Baustein, den er bedient, unter einer Identität statt unter einer Persona pro Modul.
- **`workspace`** — überschreibt pro Aufruf, welchen Arbeitsbereich dieser Aufruf öffnet. Ohne
  Angabe ist es der, mit dem der Server gestartet wurde.

### Fehler

Eine Ablehnung kommt als **Werkzeugfehler** zurück, nicht als Protokollfehler, und trägt dieselbe
Art und dieselbe Meldung, die auch die CLI ausgeben würde: ein unbekannter Schlüssel ist
`not_found`, ein leerer Text oder eine unbekannte Reichweite ist `validation`. Der Host bleibt
verbunden, und das Modell kann lesen, was schiefging, und es erneut versuchen.

## Was `nxs prime` beiträgt

Es gibt **kein `memory_prime`-Werkzeug**, und das ist eine Entscheidung, keine Lücke: der
Sitzungsstart wird einmal beim Verbinden geliefert, als `initialize.instructions` des Servers,
zusammengesetzt aus demselben `nxs prime`-Fan-out, den Sie von Hand bekommen — die Sitzungs-Hooks
liefern dieselben Blöcke, einen je aktivem Modul. memorys
Anteil daran trägt:

1. **Die Gedächtnisregel** — dauerhaftes Projektwissen gehört in `nxm remember`, nie in eine
   selbstgebaute `MEMORY.md`, weil nichts eine solche Datei zurückliest.
2. **Die Korrekturregel** — eine Erinnerung, die dem Projekt widerspricht, wird unter ihrem
   Schlüssel an Ort und Stelle korrigiert oder vergessen. Die Sitzung, die sie liest, ist der am
   besten informierte Korrektor, den sie je bekommt.
3. **Wie eine Einleitung geschrieben wird**, samt der Auflage, dass eine `rules`-Zeile ihre Regel
   ausspricht statt sie anzukündigen.
4. **Warum nichts `NEXUS_MEMORY.md` aus `CLAUDE.md` einbindet** — der Index ist bereits im Kontext,
   und die Projektion wiederholte ihn und legte jeden Rumpf darunter.
5. **Die Befehle** und **je eine Zeile für jede Erinnerung, die die Abrufregel hier wiedergibt**, in
   Lesereihenfolge — so viele, wie das Byte-Budget des Blocks trägt, und wenn es mehr sind, sagen
   die Überschrift und ein Hinweis unter dem Verzeichnis es. Nicht die Rümpfe: `memory_show` holt
   einen davon, und `memory_list` antwortet über den ganzen Satz, was der Block auch gezeigt hat.

Diese Instruktionen sind **statisch**: sie werden beim Verbinden berechnet und während der Sitzung
nicht aufgefrischt. Der lebende Stand ist also das, was die Werkzeuge antworten — `memory_list` und
die Lesewerkzeuge des Boards —, und die Instruktionen sagen das selbst, statt eine Momentaufnahme
vorzutäuschen.

## Wohin ein Agent schreiben soll

Die Naht ändert an dem Rat nichts, deshalb steht er hier einmal:

- **Ein dauerhafter Fakt über das Projekt** — eine Konvention, eine Falle, eine Entscheidung samt
  Grund — gehört in memory, unter einen Schlüssel, zu dem Sie zurückkehren können. Nicht in eine
  Notizdatei, nicht in eine Chatnachricht.
- **Ein Fakt über ein einzelnes Board-Item** kommt mit `scope: "item"` und dem Item in `refs`
  herein. Er liest sich dann an diesem Item statt in jedem Sitzungsstart — genau das hält den
  Sitzungsstart lesbar.
- **Arbeitsstand — was Sie gerade tun** — ist keine Erinnerung. flow führt die Arbeit
  ([nxf-Kernkonzepte](nxf-core-concepts)); chat trägt das Gespräch darüber
  ([nxc-Kernkonzepte](nxc-core-concepts)). memory ist für das, was danach noch stimmt.

## Weiter

- [Befehle](nxm-commands) — die CLI-Verben, die jedes Werkzeug spiegelt.
- [Kernkonzepte](nxm-core-concepts) — die Register, die ein Werkzeugaufruf setzt, und die Abrufregel.
- [Einen MCP-Host anbinden](nxf-mcp) — den Server eintragen, für jeden Host.
