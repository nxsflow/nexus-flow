---
type: added
facade: breaking
---
[en]
A published skill is a persona. A persona may now be declared as a folder
`.nxs-personas/<name>/SKILL.md` in the form of the Agent Skills specification: `name`,
`description` and `allowed-tools` at the top of the frontmatter, the instructions as the body, and
everything nexus-flow adds under one key, `nxs:` (`title`, `expected_output`, `stage`, `model`,
`addressable`, `prime`, `claude_md`, `base_prompt`, `permissions`, `working_tree`, `machine`, and
the new `requires`). A skill folder found elsewhere, copied in unchanged, is listed, addressed and
answers; without `nxs:` it runs with every default. A relative path in a skill's body means the
persona's own folder, so its `references/` are found from the user-level folder too, and the session
is told where that folder is. `allowed-tools` keeps the three states `tools` has and takes the
space-separated text, a comma-separated text or a YAML list. `license`, `compatibility` and
`metadata` are passed through and shown in `nxc list --json`, which now carries each entry's file,
form and folder under `declaration`. `nxs.requires` names capabilities and is only shown in this
release; nothing binds it yet. Channels may be declared one per file under
`.nxs-personas/channels/<name>.yaml`; `channels` is now a reserved name no persona may take. The
older forms — `<handle>.yaml` and `channels.yaml` — are still read, in the repository's folder and
the user-level folder alike, and a name declared in both forms is refused, naming both files. New:
`nxs personas migrate` rewrites a folder into the new form (`--dry-run` to see the plan, `--user`
for the user-level folder), keeps your comments, reports what it leaves out, proves every rewrite
before it writes anything, refuses a key it cannot place, and finds nothing to do the second time.
`nxs prime` warns about a name that breaks the Agent Skills name rule, the inert keys `session`,
`sub_agents` and `reports_to`, a nexus-flow key at the top of a `SKILL.md`, and `nxs.requires`
that nothing binds yet, a skill without `allowed-tools` (it runs with the full default toolset,
and the shell is approved for its answer), and a persona that skips permission prompts while it
admits callers from other workspaces. A handle may no longer contain whitespace or control
characters, and a declaration file over 1 MiB is refused. `nxs prime` errors name the real file of
a persona or channel, and a broken channel file excludes only its own channels from the roster. For
embedding apps: `DeclarationSource::files` and `file_of`,
`DeclarationFile` / `DeclarationForm`, `declaration` on `PersonaEntry` and `ChannelEntry`,
`declared_form` on `PersonaBrief` (whose `declared_in` builder now takes the form), the modules
`skill` and `persona_migration`, `role::load_declared_personas`, `channel::load_declared_channels`
and `declaration_quality::warn_declarations_in`; `persona_migration::migrate_declarations` is the
library seam of `nxs personas migrate`. An engine older than this one does not see a
skill folder: migrate a repository an app reads only once the app runs this version.
[de]
Ein veröffentlichter Skill ist eine Persona. Eine Persona kann jetzt als Ordner
`.nxs-personas/<name>/SKILL.md` deklariert werden, in der Form der Agent-Skills-Spezifikation:
`name`, `description` und `allowed-tools` oben im Frontmatter, die Anleitung als Rumpf, und alles,
was nexus-flow darüber hinaus braucht, unter einem Schlüssel `nxs:` (`title`, `expected_output`,
`stage`, `model`, `addressable`, `prime`, `claude_md`, `base_prompt`, `permissions`,
`working_tree`, `machine` und das neue `requires`). Ein Skill-Ordner von anderswo, unverändert
hineinkopiert, erscheint in der Liste, lässt sich anschreiben und antwortet; ohne `nxs:` läuft er mit
allen Standardwerten. Ein relativer Pfad im Rumpf eines Skills meint den eigenen Ordner der Persona,
sodass sie ihre `references/` auch aus dem Ordner auf Benutzerebene findet, und die Sitzung erfährt,
wo dieser Ordner liegt. `allowed-tools` behält die drei Zustände von `tools` und nimmt den
Leerzeichen-getrennten Text, einen Komma-getrennten Text oder eine YAML-Liste. `license`,
`compatibility` und `metadata` werden durchgereicht und in `nxc list --json` gezeigt, das jetzt zu
jedem Eintrag Datei, Form und Ordner unter `declaration` trägt. `nxs.requires` nennt benötigte
Fähigkeiten und wird in dieser Version nur gezeigt; noch bindet nichts sie. Kanäle können einzeln
deklariert werden, eine Datei je Kanal unter `.nxs-personas/channels/<name>.yaml`; `channels` ist
jetzt ein reservierter Name, den keine Persona tragen darf. Die älteren Formen — `<handle>.yaml` und
`channels.yaml` — werden weiter gelesen, im Ordner des Repositorys wie im Ordner auf Benutzerebene,
und ein Name in beiden Formen wird abgelehnt, mit Nennung beider Dateien. Neu: `nxs personas migrate`
schreibt einen Ordner in die neue Form um (`--dry-run` zeigt den Plan, `--user` nimmt den Ordner auf
Benutzerebene), behält die Kommentare, meldet, was es weglässt, beweist jede Umschrift, bevor es
etwas schreibt, lehnt einen Schlüssel ab, den es nicht zuordnen kann, und findet beim zweiten Lauf
nichts mehr zu tun. `nxs prime` warnt bei einem Namen, der die Namensregel der Agent-Skills-
Spezifikation verletzt, bei den wirkungslosen Schlüsseln `session`, `sub_agents` und `reports_to`,
bei einem nexus-flow-Schlüssel oben in einer `SKILL.md` und bei `nxs.requires`, das noch nichts
bindet, bei einem Skill ohne `allowed-tools` (er läuft mit dem vollen Standard-Werkzeugsatz, und
für seine Antwort wird die Shell freigegeben) und bei einer Persona, die Rückfragen überspringt und
zugleich Rufer aus anderen Arbeitsbereichen zulässt. Ein Handle darf keine Leer- oder Steuerzeichen
mehr enthalten, und eine Deklarationsdatei über 1 MiB wird abgelehnt. Fehler in `nxs prime` nennen
die echte Datei einer Persona oder eines Kanals, und eine kaputte Kanaldatei schließt nur ihre
eigenen Kanäle aus dem Roster aus. Für einbettende Apps: `DeclarationSource::files` und `file_of`, `DeclarationFile` /
`DeclarationForm`, `declaration` an `PersonaEntry` und `ChannelEntry`, `declared_form` an
`PersonaBrief` (dessen Builder `declared_in` jetzt die Form nimmt), die Module `skill` und
`persona_migration`, `role::load_declared_personas`, `channel::load_declared_channels` und
`declaration_quality::warn_declarations_in`; `persona_migration::migrate_declarations` ist die
Bibliotheks-Naht von `nxs personas migrate`. Ein älterer Engine-Stand sieht keinen Skill-Ordner:
Ein Repository, das eine App liest, erst migrieren, wenn die App auf diesem Stand ist.
