# Erste Schritte

Der erste Schritt ist derselbe, ob du beitragen oder deine eigene Anwendung auf der Engine bauen
willst: die Quellen holen, den Bau beweisen, und dann etwas Echtes laufen lassen.

## Die Quellen

```bash
git clone https://github.com/nxsflow/nexus-flow.git
cd nexus-flow
cargo test -p nexus-flow-core
```

Mehr als eine Rust-Toolchain braucht es nicht; nichts sonst ist zu installieren, kein Dienst zu
starten. Das Repository ist ein Cargo-Workspace, und die Aufteilung lohnt zwei Minuten, bevor du
eine Datei suchst:

| Wo | Was |
|---|---|
| `crates/foundation`, `crates/core` | Substrat und Engine — das Op-Log, die Reducer, die Ableitung |
| `crates/facade` | die öffentliche API, die eine Anwendung einbindet. Der eine SemVer-Vertrag im Repo |
| `crates/cli`, `crates/memory`, `crates/chat`, `crates/nxs` | die vier Kommandoflächen, dünne Aufrufer über den Engines |
| `crates/*/docs/guide/{en,de}` | die Anleitungen — derselbe Text, den `nxs guide` druckt und diese Website ausliefert |
| `content/` | der Assembler, der aus diesen Anleitungen den veröffentlichten Doku-Feed macht |
| `examples/tauri-board/` | die Beispiel-Anwendung, mit Absicht außerhalb des Workspace |

## Die vier Tore

Vier Befehle sind die Tore, an die sich dieses Projekt hält:

```bash
cargo test                                  # Unit + Integration (inkl. Differential-Orakel)
cargo test --release                        # findet Effekte, die im Debug wegkompiliert sind
cargo clippy --all-targets -- -D warnings   # Lint, Warnungen sind Fehler
cargo fmt --check                           # Formatierungstor
```

Ein voller Lauf ist lang — lang genug, dass er nicht zwischen zwei Änderungen gehört. Teste die
Crate, in der du arbeitest (`cargo test -p nexus-flow-core`), solange die Änderung in deiner Hand
ist, und lass CI das Tor sein: den Debug-Durchgang fährt es bei jedem Pull Request, den
Release-Durchgang und die macOS-Bahn auf `main`. Verhalten wird hier test-first geschrieben, der
rote Test kommt also vor dem Code, der ihn beantwortet; und eine Änderung, die Nutzer bemerken
würden, bringt ein Changelog-Fragment in `changes/` mit, in beiden Sprachen.

## Die Beispiel-Anwendung

`examples/tauri-board/` ist die, von der man abschaut. Sie bettet die Engine **im selben Prozess**
ein — sie bindet `nexus-flow-facade` ein, hält eine langlebige `Engine` in Tauris verwaltetem
Zustand und beantwortet ihr Fenster aus diesem Griff. Nirgends steckt ein `nxf`-Unterprozess darin,
und genau das ist der Punkt: die Naht, die sie ruft, ist dieselbe, die auch die CLI ruft — nichts
daran ist ein privater Weg, den eine Anwendung nicht nehmen könnte.

Sie liegt **außerhalb** des Cargo-Workspace, damit ein `cargo test` an der Wurzel nie Tauri und
WebKit auf deiner Maschine übersetzt. Das ist Absicht, und es kostet das Beispiel seine Abdeckung —
deshalb prüft ein eigener CI-Job seinen Typbau, sobald sich das Beispiel oder facade, core oder
foundation darunter ändert.

Zum Laufen braucht es die WebView der Plattform (auf macOS vorinstalliert) und einen
Arbeitsbereich, auf den es zeigt; die genauen Befehle stehen im `README.md` des Beispiels, samt dem
kleinen Graphen, den es anlegt, damit das Fenster etwas zu zeigen hat. Sieh ihm einmal zu, während
`nxf` aus einem zweiten Terminal in denselben Arbeitsbereich schreibt: das Fenster zeichnet sich von
selbst neu, weil es den Änderungsstrom der Engine abonniert, statt ihn abzufragen.

## Danach die Landkarte

[Architektur](develop-architecture) ist die Landkarte dessen, was du gerade geklont hast — fünf
Schichten, wohin die Pfeile zeigen, und welche Nähte Verträge sind. Danach verfolgt
[die Reise einer Operation](develop-the-journey-of-one-op) ein einzelnes `nxf close` vom Verb bis zu
einer abgeleiteten Antwort, die niemand geschrieben hat, und
[eine Antwort über zwei Maschinen](develop-a-reply-across-two-machines) macht dieselbe Fahrt mit
einer zweiten Replik darin.
