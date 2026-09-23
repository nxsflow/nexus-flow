# Erste Schritte

`nxs` ist die Klammer der nexus-flow-Suite. Es ist **eine Binary**, die drei Bausteine trägt —
**flow** (`nxf`: Projekte, Aufgaben, Abhängigkeiten), **memory** (`nxm`: durables Projektwissen) und
**chat** (`nxc`: der Kanal zwischen dir und deinen Agenten) — über einem gemeinsamen Speicher.
Installiert wird einmal; welche der drei aktiv sind, entscheidest du **pro Arbeitsbereich**.

Diese Anleitung führt von „nichts installiert" zu einem Arbeitsbereich, dessen Agent sich zu Beginn
jeder Sitzung seinen eigenen Kontext zurückholt. Sie ist der eine Ort, an dem die Ersteinrichtung
steht; die Anleitungen der einzelnen Bausteine setzen hier auf.

## Installieren

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

Das Skript erkennt die Plattform, prüft den Download — eine sha256 **und** eine Signatur — und
installiert **ein** Programm: `nxs`. Die übrigen Namen sind Symlinks darauf: `nxf`, `nxm` und `nxc`
sind dieselbe Binary, die aus dem getippten Namen entscheidet, welche Persona sie ist. Genau deshalb
können die vier in der Version nie auseinanderlaufen — es gibt nur ein Programm zu aktualisieren.

Die Signaturprüfung braucht entweder `minisign` oder ein OpenSSL, das Ed25519 beherrscht. Ein
unverändertes macOS hat weder noch (sein `openssl` ist LibreSSL): also entweder minisign
installieren — `brew install minisign` — oder mit `NXF_INSECURE=1` starten, was allein die sha256
prüft und damit Integrität belegt, aber nicht Herkunft. `nxs self-update` prüft Signaturen danach
bedingungslos.

```bash
nxs --version
```

## Einen Arbeitsbereich einrichten

Ein Arbeitsbereich ist ein `.nxs/`-Verzeichnis in deinem Projekt: **ein** Speicher, in den jeder
aktivierte Baustein schreibt. Angelegt wird er so:

```bash
nxs init
```

Auf einem echten Terminal fragt das mit den Pfeiltasten, welche Bausteine du willst. Die eigene
Unterkonfiguration eines Bausteins läuft sichtbar in diesem Rahmen — flow fragt nach dem Plugin,
denn diese Wahl entscheidet über das Vokabular, das du für die Lebensdauer des Arbeitsbereichs
liest und schreibst.

Auf einem Agenten oder im Skript benennst du die Bausteine, statt gefragt zu werden:

```bash
nxs init --module flow --module memory --json
```

Beide Wege landen im selben Arbeitsbereich. Einen weiteren Baustein aktivierst du später mit einem
erneuten `nxs init` — der Befehl ist idempotent und ergänzt, statt zu ersetzen.

## Was `nxs init` hinterlässt

Vier Dinge, und alle vier sind es wert, gekannt zu werden:

- **`.nxs/`** — der Arbeitsbereich selbst: ein SQLite-Speicher, über den jeder aktive Baustein seine
  eigenen Sichten faltet. Er ist git-ignoriert und liegt lokal auf dieser Maschine.
- **Die geteilte Agenten-Datei** — der `AGENTS.md`-Abschnitt, den jeder aktive
  Baustein beisteuert, zu einem Dokument zusammengesetzt statt zu dreien, die sich widersprechen.
- **Ein SessionStart-Hook je aktivem Baustein** — `nxf prime`, `nxm prime`, `nxc prime`.
- **Ein Eintrag beim Hintergrunddienst** — der Arbeitsbereich kommt auf die Liste, die dieser Dienst
  betreut, damit ein hier aufgeschobenes Datum oder ein deklariertes Fenster überhaupt jemanden hat,
  der hinsieht. Es ist ein lokaler Verzeichniseintrag und nichts weiter: er synchronisiert nichts,
  und es verlässt nichts diese Maschine. `nxs sync unregister` nimmt ihn wieder von der Liste.

[Der Arbeitsbereich](nxs-the-workspace) geht jedes davon im Einzelnen durch.

## Der Hintergrunddienst, und die Frage, die `init` dazu stellt

Ein Prozess je Maschine hält die Fristen jedes Arbeitsbereichs, den er betreut, und synchronisiert
die, die an einen Strom gebunden sind. Ohne ihn feuert ein hier fällig werdendes Aufschubdatum
schlicht nie — deshalb bietet `nxs init` auf einem echten Terminal an, ihn einzurichten:

```
Set up the nexus-flow background service on this machine?
```

Sagst du nein, wird die Antwort in diesem Arbeitsbereich festgehalten, und kein späteres `nxs init`
fragt erneut; der Rahmen sagt weiterhin, wie der Dienst steht, und `nxs sync daemon install` richtet
ihn ein, sobald du es anders willst. Im Skript oder unter `--json` wird die Frage nie gestellt und
nichts installiert: benenne es ausdrücklich mit `nxs init --service`, oder erledige die Frage mit
`nxs init --no-service`.

Der Installer ist macOS-only (es ist ein launchd-Agent). Überall sonst startest du `nxs sync daemon`
im Vordergrund unter deinem eigenen Supervisor — systemd, runit, was immer du schon hast — und er
hält dort genau dieselben Fristen.

## Was den Kontext zu Sitzungsbeginn zurückholt

Die Hooks sind **einer je aktivem Baustein**, nicht einer für die Klammer — und das ist eine bewusste
Umkehr der früheren Form. Der Wirt kürzt jede Hook-Ausgabe für sich, bei 10.240 Byte; drei Hooks
tragen also drei Budgets statt eines. Das ist gemessen, nicht angenommen: die Hooks dreier Bausteine
von je rund 8 KB kamen vollständig an, zusammen 24 KB, wo ein einzelner kombinierter Hook bei 10 KB
abgeschnitten worden wäre.

`nxs prime` gibt es weiterhin, und es ist weiterhin der Fächer:

```bash
nxs prime
```

Es führt das `prime`-Verb jedes **aktiven** Bausteins aus und gibt eine Antwort zurück — flows
Brett, memorys Index, chats Kanal — in fester Reihenfolge und mit einem gemeinsamen `now`, damit ein
Aufschubdatum in einem Lauf für jeden Baustein dasselbe bedeutet. Das ist der Befehl für das ganze
Bild an einer Stelle. Die Sitzungs-Hooks gehen nur nicht mehr darüber.

So oder so gilt die Wirkung, auf die es ankommt: Ein Agent beginnt eine Sitzung und weiß bereits, was
dieses Projekt gerade tut, was es gelernt hat und wer sonst daran arbeitet. Ein reiner flow-Bereich
liefert genau flows prime; memory erscheint an dem Tag, an dem du memory aktivierst. Konfiguriert
wird dafür nichts — die Hook-Menge folgt aus den aktiven Bausteinen.

## Welcher Baustein wofür?

Kurz: **flow** für das, was zu tun ist, **memory** für das, was das Projekt weiß, **chat** für die
Nachrichten zwischen dir und deinen Agenten. [Module](nxs-modules) ist die längere Antwort und lohnt
sich vor den drei Detailanleitungen.

## Wie es weitergeht

- [Module](nxs-modules) — wofür jeder Baustein da ist
- [Der Arbeitsbereich](nxs-the-workspace) — was auf der Platte liegt und wie du es gesund hältst
- Dann der Baustein, den du wirklich brauchst: [flow](nxf-getting-started),
  [memory](nxm-getting-started), [chat](nxc-getting-started)
