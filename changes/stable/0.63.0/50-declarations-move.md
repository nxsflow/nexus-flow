---
type: changed
---
[en]
**Declarations live in `.nxs-personas/` now**, not in `roles/`. Who your agents are and where they talk is one folder at the project root: one YAML per persona plus `channels.yaml`. Nothing reads the old folder any more.
[de]
**Deklarationen liegen jetzt in `.nxs-personas/`**, nicht mehr in `roles/`. Wer Ihre Agenten sind und wo sie reden, steht in einem Ordner in der Projektwurzel: eine YAML je Persona plus `channels.yaml`. Den alten Ordner liest nichts mehr.
[migration.en]
Manual, and it is a rename: move `<project-root>/roles/` to `<project-root>/.nxs-personas/`. Nothing converts it for you and nothing warns you — a workspace whose declarations stayed in `roles/` simply has no declared team, so `nxc list` comes up empty and `send --to` refuses every target. The file contents are unchanged.
[migration.de]
Handarbeit, und es ist eine Umbenennung: verschieben Sie `<Projektwurzel>/roles/` nach `<Projektwurzel>/.nxs-personas/`. Nichts wandelt das für Sie um und nichts warnt — ein Workspace, dessen Deklarationen in `roles/` liegen blieben, hat schlicht kein deklariertes Team: `nxc list` bleibt leer und `send --to` weist jedes Ziel ab. Die Dateiinhalte bleiben unverändert.
