---
type: added
facade: breaking
---
[en]
One persona definition for every repository on a machine. Beside a workspace's own
`.nxs-personas/`, every workspace now reads the user-level folder `~/.nexusflow/personas/` —
personas and a `channels.yaml`, laid out the same way — merged per name: a persona or channel the
repository declares itself hides the user-level one of the same name, everything else is added. One
`pm` written once runs in every repository as that repository's own participant, with its board,
memory and working copy. `nxc list` marks each entry that comes from the user-level folder and says
which ones the repository hides, and so does `nxs prime`; a persona from it is told at session start
where its declaration lives, because a path in its instructions relative to "this repository" means
the repository it runs in. The per-operation freeze holds for it as for the repository's own folder.
A user-level `external` admits callers only from workspaces the running repository itself trusts
by name. A development build reads a user-level folder only under a named service instance
(`~/.nexusflow-<name>/personas/`), never the production one. A machine without the folder behaves
as before. For embedding apps: `Engine::definitions`,
`Engine::directory` and `Engine::prime_as` read the same folder (an app cannot choose another);
new `user_path`, `from_user` and `shadowed` on `DeclarationSource`, `origin` on `PersonaEntry` and
`ChannelEntry`, `declared_in` on `PersonaBrief`, `user_declarations_dir`, `merged_channels`, and
`Definitions::resolve_with_user_dir` / `Definitions::from_source` — a struct literal of those four
types needs the new fields.
[de]
Eine Persona-Definition für jedes Repository auf einer Maschine. Neben dem eigenen `.nxs-personas/`
liest jeder Arbeitsbereich jetzt den Ordner auf Benutzerebene `~/.nexusflow/personas/` — Personas
und eine `channels.yaml`, genauso aufgebaut — und führt beide je Name zusammen: Eine Persona oder ein
Kanal, den das Repository selbst deklariert, verdeckt den gleichnamigen Eintrag auf Benutzerebene,
alles andere kommt hinzu. Ein einmal geschriebener `pm` läuft in jedem Repository als dessen eigener
Teilnehmer, mit dessen Board, Gedächtnis und Arbeitskopie. `nxc list` markiert jeden Eintrag aus dem
Ordner auf Benutzerebene und sagt, welche das Repository verdeckt, `nxs prime` ebenso; eine Persona
von dort erfährt beim Sitzungsstart, wo ihre Deklaration liegt, denn ein Pfad in ihren Anweisungen
relativ zu „diesem Repository" meint das Repository, in dem sie läuft. Das Einfrieren je Vorgang gilt
dort wie für den eigenen Ordner. Ein `external` auf Benutzerebene lässt Aufrufer nur aus Arbeitsbereichen
zu, denen das laufende Repository selbst beim Namen vertraut. Ein Entwicklungs-Build liest einen
Ordner auf Benutzerebene nur unter einer benannten Dienst-Instanz (`~/.nexusflow-<name>/personas/`),
nie den produktiven. Eine Maschine ohne den Ordner verhält sich wie bisher. Für einbettende
Apps: `Engine::definitions`, `Engine::directory` und `Engine::prime_as` lesen denselben Ordner (eine
App kann keinen anderen wählen); neu sind `user_path`, `from_user` und `shadowed` an
`DeclarationSource`, `origin` an `PersonaEntry` und `ChannelEntry`, `declared_in` an `PersonaBrief`,
`user_declarations_dir`, `merged_channels` sowie `Definitions::resolve_with_user_dir` / `Definitions::from_source` — ein Struct-Literal dieser
vier Typen braucht die neuen Felder.
