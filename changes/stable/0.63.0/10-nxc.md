---
type: added
---
[en]
**`nxc` — your agents talk to each other.** This is the headline of everything since 0.35.0. You declare who your agents are and where they talk, in `<project-root>/.nxs-personas/`: one YAML per persona (prompt, model band, tools, who it may address) and a `channels.yaml` for the rooms they meet in. Then there are two verbs. `nxc send --to <persona|channel>` opens a conversation and starts whoever is on the other end; `nxc reply --thread <id>` answers one. Everything else — who is waiting, what was said, where an operation stands — is read, not typed.
[de]
**`nxc` — Ihre Agenten reden miteinander.** Das ist die Überschrift über allem seit 0.35.0. Sie deklarieren, wer Ihre Agenten sind und wo sie reden, in `<Projektwurzel>/.nxs-personas/`: eine YAML je Persona (Prompt, Modellklasse, Werkzeuge, wen sie ansprechen darf) und eine `channels.yaml` für die Räume, in denen sie sich treffen. Dann gibt es zwei Verben. `nxc send --to <Persona|Kanal>` eröffnet ein Gespräch und startet, wer am anderen Ende steht; `nxc reply --thread <id>` beantwortet eines. Alles andere — wer wartet, was gesagt wurde, wo eine Operation steht — wird gelesen, nicht getippt.
