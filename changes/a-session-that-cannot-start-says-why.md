---
type: fixed
facade: none
---
[en]
On the new engine, a session that cannot be started now says why instead of timing out in silence. When the chat store, the declarations, the sidecar bundle or the session backend refuses a start, the service log names the run, the session and the port that refused, once, and `nxc status` shows the member as could not be started, with the port and since when, until the session starts or the run ends. `nxs sync daemon status` now names the user-level persona folder an instance reads, and on a development instance where that folder does not exist it says so: such an instance knows no persona declared only at user level.
[de]
Auf der neuen Engine sagt eine Sitzung, die nicht gestartet werden kann, jetzt warum, statt stumm in die Zeitüberschreitung zu laufen. Lehnt der Chat-Speicher, die Deklarationen, das Sidecar-Bundle oder das Sitzungs-Backend einen Start ab, nennt das Protokoll des Dienstes einmal den Lauf, die Sitzung und den ablehnenden Port, und `nxc status` zeigt das Mitglied als nicht startbar, mit dem Port und seit wann, bis die Sitzung startet oder der Lauf endet. `nxs sync daemon status` nennt jetzt den Ordner der benutzerweiten Personas, den eine Instanz liest, und sagt auf einer Entwicklungsinstanz, wenn dieser Ordner fehlt: Eine solche Instanz kennt keine Persona, die nur benutzerweit deklariert ist.
