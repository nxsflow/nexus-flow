# Module

Die Suite besteht aus drei Bausteinen über einem Speicher. Diese Seite ist die Landkarte: wofür jeder
da ist und zu welchem du greifst. Sie gehört vor die drei Detailanleitungen — die setzen jeweils
voraus, dass du schon weißt, in welchem Baustein du bist.

Aktiviert wird **pro Arbeitsbereich**, mit `nxs init`. Einen zu aktivieren verpflichtet dich nicht
zu den anderen: ein Arbeitsbereich kann flow allein tragen oder alle drei.

## flow (`nxf`) — was zu tun ist

Der Issue-Tracker: Items, die Abhängigkeiten zwischen ihnen, Fälligkeits- und Aufschubdaten, ein
append-only Notizstrom und ein Abschlussgrund. Seine kennzeichnende Eigenschaft: **die Arbeitsliste
wird abgeleitet, nicht gespeichert** — `nxf next` und `nxf blocked` werden bei jeder Frage aus dem
Abhängigkeitsgraphen berechnet und können deshalb weder veralten noch ihm widersprechen.

Was ein Item *ist*, kommt aus einem **Plugin**: die Typen, die du anlegen darfst, das Vokabular, das
du liest, und die Rangfolge hinter `next`. `nxs init` fragt danach, weil die Wahl alles prägt, was du
danach tippst. Zu flow greifst du, wenn die Frage lautet: *Was ist als Nächstes dran, und was
blockiert es?*

→ [Anleitung zu flow](nxf-getting-started)

## memory (`nxm`) — was das Projekt weiß

Durable Tatsachen, die eine Sitzung überleben: Konventionen, Fallen, Entscheidungen und die Gründe
dahinter. `remember` schreibt eine, `recall` liest sie vollständig zurück, und ein stabiler Schlüssel
lässt eine Tatsache **an Ort und Stelle korrigieren**, statt widersprüchliche Kopien anzuhäufen.

Der Zweck ist nicht Ablage, sondern Wiedervorlage: Erinnerungen werden zu Beginn jeder Sitzung
zurückgegeben, damit ein neuer Agent mit dem beginnt, was der letzte gelernt hat. Sie lassen sich
zusätzlich in ein generiertes `NEXUS_MEMORY.md` projizieren, sodass auch jemand ohne installierte
Suite denselben Kontext lesen kann. Zu memory greifst du, wenn etwas mühsam Gelerntes nicht zweimal
gelernt werden soll.

→ [Anleitung zu memory](nxm-getting-started)

## chat (`nxc`) — der Kanal zwischen dir und deinen Agenten

Der Kanal zwischen dir und den Agenten eines Arbeitsbereichs: `send --to` beginnt etwas,
`reply --thread` antwortet darauf, und jede Nachricht ist durable und wiederholbar statt eine
Nachricht im Flug. Wer angesprochen werden darf und wie eine Gruppe von Agenten vorgeht, wird **deklariert** — in Dateien
unter `.nxs-personas/`: eine Persona je Agent und Kanäle, die einen Ablauf tragen.

Zu chat greifst du, wenn mehr als ein Agent arbeitet und eine Übergabe das Ende einer Sitzung
überstehen muss. Notizzettel im Dateisystem tun das nicht: sie werden nicht zugestellt, nicht
synchronisiert und nie wiedergegeben.

→ [Anleitung zu chat](nxc-getting-started)

## Was sie teilen

Einen `.nxs/`-Arbeitsbereich und einen Speicher darunter — siehe
[Der Arbeitsbereich](nxs-the-workspace). Jeder Baustein faltet seine eigene Sicht über denselben
append-only Log. Deshalb fügt ein zweiter Baustein eine Sicht hinzu und keine zweite Datenbank, und
deshalb trägt ein Sync-Strom sie alle.

Die Klammer selbst besitzt keine Domäne. `nxs` richtet Arbeitsbereiche ein (`init`), holt Kontext
zurück (`prime`), diagnostiziert (`doctor`), migriert das Schema, synchronisiert, aktualisiert sich
selbst und bedient die Suite über MCP. Alles Übrige gehört einem Baustein.
