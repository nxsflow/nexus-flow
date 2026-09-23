---
type: added
---
[en]
**Agent sessions leave a transcript, and it is bounded.** Every role session's normalized stream — assistant text, thinking, tool calls and their results, and any subagent activity — is readable with `nxc transcript show <session>`. Transcripts are device-local and never synced, and they are kept for 30 days after a session's last write, so they stop growing forever.
[de]
**Agenten-Sitzungen hinterlassen einen Verlauf, und der ist begrenzt.** Der normalisierte Strom jeder Rollensitzung — Text, Denkschritte, Werkzeugaufrufe samt Ergebnissen und alles, was Subagenten dabei taten — ist mit `nxc transcript show <Sitzung>` lesbar. Verläufe sind gerätelokal und werden nie synchronisiert; sie werden 30 Tage nach dem letzten Schreibvorgang aufbewahrt und wachsen damit nicht mehr unbegrenzt.
