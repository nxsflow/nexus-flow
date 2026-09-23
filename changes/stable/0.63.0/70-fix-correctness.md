---
type: fixed
---
[en]
**Two processes on one workspace no longer lose each other's writes.** The logical clock two `nxs` invocations advance in parallel is now correct under concurrency, so a write made while another process was writing is not silently dropped. Ops can also no longer be written with a blank author: an unset actor is refused rather than recorded as nobody.
[de]
**Zwei Prozesse auf einem Workspace verlieren einander die Schreibvorgänge nicht mehr.** Die logische Uhr, die zwei parallel laufende `nxs`-Aufrufe fortschreiben, ist unter Nebenläufigkeit jetzt korrekt — ein Schreibvorgang während eines anderen geht nicht mehr stillschweigend verloren. Außerdem können Ops nicht mehr ohne Autor geschrieben werden: ein nicht gesetzter Akteur wird abgelehnt, statt als niemand festgehalten zu werden.
