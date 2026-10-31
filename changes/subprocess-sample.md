---
type: added
facade: changed
---
[en]
`nexus-chat`: the `Worker` trait gains `subprocess_sample` — how many processes have started under a session's open tool call since a given instant, and the CPU they have used so far, as `nxs_service::process::SubprocessSample`. The default is `None`, so an existing host worker compiles unchanged. The bundled sidecar worker reads it from the process table, and only for a process it has proved is the session's own.
[de]
`nexus-chat`: Der Trait `Worker` erhält `subprocess_sample` — wie viele Prozesse seit einem gegebenen Zeitpunkt unter dem offenen Werkzeugaufruf einer Sitzung gestartet sind und wie viel CPU sie bisher verbraucht haben, als `nxs_service::process::SubprocessSample`. Die Vorgabe ist `None`, ein bestehender Host-Worker kompiliert also unverändert. Der mitgelieferte Sidecar-Worker liest das aus der Prozesstabelle, und nur für einen Prozess, von dem er nachgewiesen hat, dass er der Sitzung selbst gehört.
