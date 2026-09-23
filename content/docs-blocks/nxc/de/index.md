# chat — der Kanal

Der Kanal zwischen Ihnen und den Agenten eines Arbeitsbereichs. `nxc send --to <handle>`
beginnt etwas, `nxc reply --thread <id>` antwortet darauf, und `--escalate` ist die Art, wie ein
Agent *ich brauche Hilfe oder eine Entscheidung* sagt, statt zu raten — die Runde geht damit nach
oben, zu dem, der sie beauftragt hat.

chat ist ein **Substrat, keine Chat-App**, und dieser Unterschied ist der ganze Grund, warum es
existiert. Eine Nachricht ist nicht unterwegs: sie ist eine Zeile in demselben Speicher des
Arbeitsbereichs, in den auch flow und memory schreiben — also durable, mit allem anderen
synchronisiert und wiedervorgelegt statt erinnert. Kein Daemon, den man laufen lassen muss, kein
Server, auf den man zeigt. Das ist auch die Regel, die Agenten zum Sitzungsstart bekommen: stimmt
euch niemals über Kritzeldateien oder Ad-hoc-Notizen ab — die werden nicht zugestellt, nicht
synchronisiert und nie wiedervorgelegt.

Wer angesprochen werden darf und wie eine Gruppe von Agenten vorgeht, wird **deklariert**, nicht
konfiguriert. Eine Persona-Datei je Agent unter `.nxs-personas/` gibt ihm eine Identität, seine
Prompt-Schichten, ein Modellband, seine Werkzeuge und die Liste derer, die ihn ansprechen dürfen;
ein Kanal deklariert eine Gruppe — und ein geordneter (`flow: sequential`) *ist* der Arbeitsablauf,
nicht seine Beschreibung.

```bash
nxc send --to <handle> -   # eine Thread-Id kommt zurück
nxc reply --thread <id> -  # der Rumpf auf STDIN: `… - <<'EOF'`, Ihr Text, dann `EOF`
```

Das `-` ist es, was die Nachricht unversehrt lässt: ein Urteil oder ein Bericht ist voller
Backticks und `$(…)`, und als Shell-Argument übergeben landet er bei der Shell, bevor `nxc` ihn
überhaupt sieht. Eine einzeilige Nachricht darf weiterhin ein Argument sein.

Weil Agenten unbeaufsichtigt laufen, gehören die Grenzen zum Entwurf und sind kein Nachgedanke: eine
Hop-Obergrenze, damit ein Gespräch nicht ewig kreist, ein menschliches Tor vor dem, was ein Agent
nicht entscheidet, und eine Reservierung auf der Arbeitskopie, damit nicht zwei Agenten gleichzeitig
in einem Checkout arbeiten.

## Wohin von hier

- [Erste Schritte](nxc-getting-started) — eine Persona deklarieren und die erste Antwort bekommen.
- [Personas](nxc-personas) und [Kanäle](nxc-channels) — die beiden Deklarationen und der geordnete
  Kanal, der einen Ablauf trägt.
- [Grenzen und Sicherheit](nxc-limits-and-safety) — die Hop-Obergrenze, das menschliche Tor und was
  ein Agent *nicht* entscheidet.
