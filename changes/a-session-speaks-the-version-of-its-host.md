---
type: fixed
facade: changed
---
[en]
A session on the new engine now runs the `nxc`, `nxf` and `nxm` of the very binary its background service runs: their directory comes first on the session's PATH. A development service used to hand its sessions the installed `nxc` — another version — and a named development instance now reaches its sessions as `NXS_SERVICE_INSTANCE`, so they read that instance's personas and registry. The installed service is unchanged: its `nxc` is the one it already found. `nexus-chat` gains `v2_bridge::service_layout_for` and `v2_bridge::session_tools_dir`.
[de]
Eine Sitzung auf der neuen Engine ruft jetzt `nxc`, `nxf` und `nxm` genau des Programms auf, das ihr Hintergrunddienst ausführt: deren Verzeichnis steht vorn im PATH der Sitzung. Ein Entwicklungsdienst gab seinen Sitzungen bisher das installierte `nxc` — eine andere Version —, und eine benannte Entwicklungsinstanz erreicht ihre Sitzungen jetzt als `NXS_SERVICE_INSTANCE`, damit sie deren Personas und Register lesen. Der installierte Dienst bleibt, wie er ist: sein `nxc` ist das, das er schon fand. `nexus-chat` erhält `v2_bridge::service_layout_for` und `v2_bridge::session_tools_dir`.
