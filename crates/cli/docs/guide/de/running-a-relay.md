# Einen Relay betreiben

Du hast eine Weile lokal gearbeitet. Jetzt soll dein Board woanders liegen als auf diesem Laptop —
als Sicherung, oder damit eine Kollegin (oder deine zweite Maschine) dieselbe Arbeit sieht.

Dafür braucht es einen **Relay**: einen kleinen Server, der das Op-Log hält und es jeder Replik
gibt, die danach fragt. Einen gehosteten gibt es nicht; du betreibst ihn selbst. Die gute Nachricht:
du hast ihn bereits.

> **Zuerst dies lesen: der Relay hat noch keine Authentifizierung.** Wer seine Adresse erreicht,
> kann den ganzen Stream lesen *und* schreiben. Betreibe ihn in einem privaten Netz, über ein VPN
> oder hinter etwas, das die Authentifizierung übernimmt — nicht auf einer öffentlichen Adresse.
> Das ist eine bekannte Lücke, keine Einstellung, die du von außen ändern kannst. Was er nicht mehr
> kann: **eine Anweisung fälschen**. Jede Op ist von der Maschine unterschrieben, die sie geschrieben
> hat, und ein Agent handelt nur auf Ops, die ein Schlüssel unterschrieben hat, dem du vertraust —
> siehe *Wem eine Maschine glaubt* weiter unten.

## Die Binary hast du schon

Jedes Release-Archiv enthält zwei Programme: `nxs` (die CLI, die du benutzt) und `nxf-relay`. Der
Installer hält den Relay standardmäßig aus deinem `PATH` heraus, weil die meisten Leute nie einen
betreiben:

```bash
NXF_INSTALL_RELAY=1 curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

Ist `nxs` schon installiert, legt ein erneuter Aufruf in dieser Form den Relay daneben.

## Das Kleinste, was funktioniert

```bash
NXF_RELAY_ADDR=127.0.0.1:8787 NXF_RELAY_DB=/var/lib/nxf/relay.sqlite nxf-relay
```

Das ist der ganze Server. Er legt alles in **einer SQLite-Datei** ab — und das ist zugleich die
ganze Sicherungsstrategie: anhalten (oder das Volume schnappschießen), Datei kopieren, fertig. Für
eine Person mit mehreren Geräten oder ein kleines Team auf einer Kiste ist das kein Kompromiss,
sondern die richtige Antwort.

Richte deine Arbeitsbereiche darauf aus und binde einen Stream:

```bash
nxs sync endpoint http://relay.internal:8787   # einmal pro Maschine
nxs sync bind                                  # einmal pro Arbeitsbereich
nxs sync run                                   # push + pull
```

In einem Git-Repository braucht `bind` keine Argumente: die Stream-Id wird aus dem `origin`-Remote
abgeleitet, also landet jeder Klon auf demselben Stream. Die anderen Bindungsarten stehen unter
[Migration](nxf-migration).

Ein Relay hinter einem Gateway, das einen Schlüssel verlangt, bekommt ihn in der URL,
`https://<benutzer>:<passwort>@relay.example`. nxs sendet ihn als Basic-`Authorization`-Header und
wiederholt ihn nie: Ein fehlgeschlagener Durchlauf nennt das Relay nur mit Host, Port und Pfad — im
Log des Dienstes, in seinem Status und in `nxs sync machines` gleichermaßen —, und `nxs sync bind`
und `nxs sync endpoint` zeigen die URL maskiert. Die Dateien, die sie speichern (`.nxs/sync.toml`,
`~/.nexusflow/config.toml`), sind nur für dich lesbar.

## Welche Maschinen einen Arbeitsbereich synchronisieren

Sobald mehr als eine Maschine einen Arbeitsbereich synchronisiert, sagt dir der Relay, welche das
sind und welche davon gerade online sind:

```text
$ nxs sync machines
stream stream-01j8… on http://relay.internal:8787 — 2 machines, 1 online
  online   Mac mini   last seen 42s ago  (this machine)
  offline  MacBook    last seen 3h 5m ago
```

Der **Hintergrunddienst** jeder Maschine meldet dem Relay nach jedem Durchlauf, der synchronisiert
hat, „ich bin da“. Ein von Hand gestartetes `nxs sync run` tut das nicht: „online“ verspricht, dass
auf dieser Maschine etwas den Arbeitsbereich betreut, und dieses Versprechen hält nur der Dienst.
Eine Maschine taucht also in der Liste auf, sobald ihr Dienst (`nxs sync daemon install`) den
Arbeitsbereich synchronisiert hat.

**Online** heißt: Die letzte Meldung der Maschine ist höchstens doppelt so alt wie der Takt, den ihr
Dienst zugesagt hat, plus eine Minute. Der Takt ist das Intervall des Dienstes, aber nie kürzer als
seine Wiederholungspause von 5 Minuten — also **11 Minuten** beim voreingestellten Intervall und bei
jedem kürzeren. Der zweite Takt fängt einen gescheiterten Durchlauf ab, damit eine laufende Maschine
nicht flackert; der Preis ist, dass eine ausgeschaltete Maschine noch bis zu 11 Minuten als online
gilt. `--json` liefert je Maschine `last_seen_at`, `age_secs` und die Spanne, nach der geurteilt
wurde (`online_within_secs`).

**Eine Maschine ist ein Dienst-Heim.** Auf einem gewöhnlichen Rechner ist das der Rechner selbst.
Eine Entwicklungsinstanz (`~/.nexusflow-dev`) ist eine eigene Maschine — sie ist ein eigener
Dienst —, und ihr voreingestellter Name sagt das: `studio (dev)`.

**Der Name** ist zunächst der Hostname des Rechners, und `nxs sync bind` sagt dir, welcher das ist,
bevor der Dienst ihn zum ersten Mal meldet. Anzeigen und ändern:

```bash
nxs sync machine               # Name und Id dieser Maschine
nxs sync machine "Mac mini"    # umbenennen; die Id bleibt
```

Ein Name muss sich lesen, wie er ist: bis zu 64 Zeichen, keine unsichtbaren oder
richtungsändernden Zeichen, kein Leerraum außer Leerzeichen.

Der Relay hat keine Authentifizierung, und das prägt drei Dinge:

- **Jeder, der den Stream lesen kann, kann auch die Maschinennamen lesen**, samt dem Zeitpunkt, zu
  dem jede zuletzt gesehen wurde. Steckt dein eigener Name im Hostnamen, benenne die Maschine um.
  Mehr wird nicht übertragen: eine auf der Maschine erzeugte Id (eine ULID — ihre ersten Zeichen
  verraten, wann sie erzeugt wurde), der Name und der Takt des Dienstes. Keine Pfade, kein Benutzer,
  keine Hardware-Kennung.
- **Ein Eintrag ist eine Behauptung, kein Beweis.** Wer den Relay erreicht, kann eine Maschine
  anmelden oder eine gestoppte online aussehen lassen — bis zu etwa zwei Stunden, dem längsten Takt,
  den ein Relay annimmt. Nimm die Liste als Hinweis, welche Maschinen es gibt und welche wach sind,
  nie als Erlaubnis.
- **Sie ist begrenzt.** Eine Maschine, von der 30 Tage lang niemand gehört hat, wird vergessen und
  taucht wieder auf, sobald ihr Dienst sich erneut meldet. Eine Antwort nennt höchstens 100
  Maschinen; hält der Relay mehr (typischerweise eine Flut erfundener), sagt `nxs sync machines`,
  dass die Liste nicht vollständig ist.

Präsenz ist **nicht Teil des Op-Logs**. Der Relay hält einen Eintrag je Maschine und Stream und
überschreibt ihn bei jeder Meldung: SQLite und Postgres in einer Tabelle `machine_presence`, die der
Relay beim Start anlegt, DynamoDB als dritte Art von Eintrag (`machine#<id>`) in der Registry-Tabelle,
die du ohnehin hast — keine neue Tabelle. Bei DynamoDB braucht die Rolle des Relays jetzt zusätzlich
**`dynamodb:Query`** auf dieser Tabelle, und du kannst TTL auf ihrem Attribut `expires_at`
einschalten, damit vergessene Maschinen entfernt werden (ohne TTL werden sie nur ausgeblendet). Ein
Relay, der älter ist als diese Funktion, zeichnet schlicht keine Präsenz auf: `nxs sync machines`
sagt das, und die Synchronisation läuft unverändert weiter. Ein Relay hinter einem Gateway muss
zusätzlich `GET` und `POST` auf `/streams/{id}/machines` durchleiten.

## Wem eine Maschine glaubt

Jede Op, die ein Arbeitsbereich schreibt, ist von ihm **unterschrieben**, und jede Op, die er
empfängt, wird **geprüft**. Die Prüfung entscheidet nur eines: ob hier eine **Agenten-Aktion** auf
die Op folgen darf. Das Brett selbst fragt nie danach — jede Op wird behalten und gezeigt, wer auch
immer sie unterschrieben hat, damit alle Maschinen weiterhin beim selben Brett ankommen.

Eine Aktion folgt einer Op, wenn dieser Arbeitsbereich sie geschrieben hat oder wenn ihre
Unterschrift zu einem Schlüssel auf der **Vertrauensliste** dieses Arbeitsbereichs passt. Alles
andere — eine Op von einem Schlüssel, dem du nicht vertraust, eine, deren Unterschrift nicht passt,
eine ganz ohne Unterschrift — wird gezeigt und bewirkt nichts: Eine Antwort darin beantwortet keine
Runde, nennt keine Sitzung, die geweckt wird, und wird als niemandes Antwort eingesammelt.

Der Schlüssel gehört zum Arbeitsbereich auf dieser Maschine: Seine private Hälfte liegt in
`.nxs/signing.key`, nur für dich lesbar, angelegt beim ersten Öffnen des Arbeitsbereichs. Damit dein
Schreibtisch-Rechner auf das handeln darf, was dein Laptop anstößt, jeweils im selben Arbeitsbereich:

```bash
nxs sync key                                        # auf dem Laptop: gibt ed25519:… aus
nxs sync trust add ed25519:… --name laptop          # auf dem Schreibtisch-Rechner
```

**Vergleiche diese Zeichenkette über einen Weg, dem du traust** — vorlesen, in einen Chat
einfügen, von dem du weißt, dass er deiner ist —, nie über den Relay: Der Relay ist genau die
Partei, der die Prüfung misstraut. `nxs sync trust list` hilft beim Finden des richtigen Schlüssels:
Es zeigt die Schlüssel, denen du vertraust, und die, die hier Ops unterschrieben haben, ohne dass du
ihnen vertraust — samt den Namen, die ihre Ops behaupten. Die Namen sind Behauptungen; vergleichen
musst du den Schlüssel.

```bash
nxs sync trust list                                 # wem dieser Arbeitsbereich glaubt
nxs sync verify <op-oder-nachrichten-id>            # wer eine Op unterschrieben hat, ob sie wirken darf
nxs sync trust remove ed25519:…                     # einem Schlüssel sofort nicht mehr glauben
```

Die Liste ist **lokal**: Sie wird nie synchronisiert, ein Schnappschuss trägt sie nicht, und jede
Maschine entscheidet selbst, wem sie glaubt. Das Austragen eines Schlüssels wirkt sofort, auch für
seine früheren Ops. Dafür gibt es absichtlich kein MCP-Werkzeug: Die Anweisung eines Agenten darf
nicht erweitern können, wem deine Maschine glaubt.

Was du siehst, wenn etwas nicht vertraut ist:

- **Eine Nachricht** trägt in `nxc threads show` den Zusatz `(unvouched — no agent action follows
  it)` und in `--json` `"unvouched": true`.
- **Ein Thread**, dessen Eröffnung oder dessen „wer schuldet bis wann eine Antwort“ aus einer Op
  stammt, der du nicht vertraust, ist **gehalten**: `nxc threads show` sagt es, und `nxc tick`
  antwortet `held`, statt zu handeln.
- **Ein Sync-Durchlauf warnt**, wenn unterwegs Unterschriften verloren gingen — Ops, die diese
  Maschine unterschrieben hält und die ohne Unterschrift zurückkamen, oder unsignierte Ops von
  Maschinen, die sonst unterschreiben. Ein Relay älter als nxs 0.58 wirft sie weg: aktualisiere ihn.
  Die Ops bleiben so oder so erhalten; sie bewirken nur nichts. Und `nxs sync trust list` zeigt,
  wie viele Ops jeder vertraute Schlüssel hier unterschrieben hat: Eine Maschine, der du vertraust,
  deren Änderungen ankommen, während diese Zahl bei null bleibt, sitzt hinter einem solchen Relay.

Arbeitsbereiche aus der Zeit vor den Unterschriften funktionieren weiter: Was diese Maschine vorher
geschrieben hat, gilt als ihr eigenes, und Ops von Maschinen, die noch nicht unterschreiben, werden
gezeigt und bewirken nichts.

## Auf welcher Maschine ein Chat läuft

Ein Chat mit einer Persona läuft auf genau **einer** Maschine; jede andere Maschine, die den
Arbeitsbereich synchronisiert, zeigt ihn und startet nichts. Welche, wird festgelegt, wenn der Chat
beginnt — `nxc send --to <persona> --machine <m>`, sonst `machine:` der Persona, sonst die Maschine,
die ihn beginnt — und in den Chat geschrieben. So kann ein Chat, den du auf dem Handy beginnst, auf
dem Mac mini laufen, und eine Antwort von überall erreicht die Persona dort. Die Einzelheiten stehen
im Chat-Guide ([Personas](nxc-personas)).

Was das über Maschinen hinweg trägt, richtet diese Seite schon ein:

- **Der Dienst der bestimmten Maschine nimmt den Chat auf.** Er fragt den Relay alle 15 Sekunden, ob
  es etwas Neues gibt — eine kleine Anfrage je Arbeitsbereich, ein voller Durchgang nur bei Ja —, so
  dass ein anderswo geschriebener Chat dort binnen etwa einer halben Minute startet.
- **Nur von einer Maschine, der er vertraut.** Auftrag und Festlegung müssen beide von einem
  Schlüssel auf seiner Vertrauensliste kommen (siehe oben). Ein Auftrag von anderswo steht im Faden
  und startet nichts.
- **Einmal.** Jede Nachricht wird je Maschine einmal aufgenommen, auch wenn diese Maschine zwei Klone
  desselben Repositorys hält.
- **„Online" entscheidet, ob nachgefragt wird.** Ein Chat für eine Maschine, die nicht online ist,
  wird nicht gesendet: Du bekommst die Maschinen, die es sind, und wählst. Die Liste ist die aus
  `nxs sync machines` — ein Hinweis, nie eine Erlaubnis.

## Eine neue Maschine vom Schnappschuss

Eine neue Maschine faltet beim ersten Sync die ganze Historie: jede Op, die das Brett je hatte, eine
nach der anderen. Bei einem Brett mit einigen tausend Ops dauert das ein, zwei Sekunden, und es wächst
mit dem Brett. Stattdessen kann eine Maschine, die das Brett schon synchronisiert, einen
**Schnappschuss** übergeben — ihr Op-Log, das daraus gefaltete Brett und die Relay-Position, bis zu
der beide reichen. Die neue Maschine startet davon und zieht nur, was danach kam:

```bash
nxs sync run                              # auf einer Maschine, die das Brett synchronisiert: erst pushen
nxs sync snapshot board.snapshot          # den Schnappschuss schreiben

nxs init                                  # auf der neuen Maschine, im neuen Klon
nxs sync bind --snapshot board.snapshot --endpoint http://relay.internal:8787
```

`bind --snapshot` tritt dem Stream bei, von dem der Schnappschuss stammt, übernimmt den Schnappschuss
und meldet den Arbeitsbereich erst danach beim Hintergrunddienst an — dessen erster Durchlauf zieht
nur noch den Rest. Die neue Maschine zeigt danach genau das Brett der Quelle. Das ist die Zusage: Ein
Schnappschuss plus der Rest faltet zum selben Stand wie die ganze Historie, auch mit Änderungen, die
in die Vergangenheit geschrieben wurden, und mit Ops, die der Relay doppelt hält.

Was geprüft und namentlich abgewiesen wird:

- **Die Quelle hat alles gepusht.** `nxs sync snapshot` verweigert, solange diese Maschine Ops hält,
  die der Relay noch nicht gesehen hat: Ein Schnappschuss verspricht, dass alles darin vom Relay aus
  erreichbar ist.
- **Der neue Arbeitsbereich ist leer.** Ein Schnappschuss startet ein frisches Replikat; ein
  Arbeitsbereich, der schon Ops hält, bindet ohne `--snapshot` und synchronisiert auf dem gewohnten
  Weg. Ein abgewiesenes `bind` lässt ihn ungebunden und leer zurück; eines, das mittendrin
  unterbrochen wurde, kann das Log geladen, aber ungebunden hinterlassen — dann ohne `--snapshot`
  erneut binden, und er synchronisiert von dort weiter.
- **Das ID-Präfix der Maschine gehört ihr.** Hält ein anderes Replikat im Stream schon das Präfix,
  das der neue Arbeitsbereich bekommen hat, gibt der Relay ihm vor dem Import ein neues, damit die
  ankommenden IDs bleiben, wie sie sind.
- **Die Position gehört zu diesem Relay.** Beim Schreiben des Schnappschusses fragt nxs den Relay,
  welche Op er an der Position hält, bis zu der die Maschine synchronisiert ist, verweigert, wenn die
  Maschine diese Op nicht hat, und schreibt ihre ID in den Schnappschuss. Bevor irgendetwas
  geschrieben wird, fragt die neue Maschine den Relay nach der Op an derselben Position und verlangt
  dieselbe. Ein Schnappschuss von einem anderen Relay, oder von vor einem Neuaufbau oder Umordnen des
  Relay-Logs, besteht diese Prüfung nicht — die Maschine zieht dann stattdessen von vorn und
  überspringt jede Op, die sie schon hat. Langsamer, aber nie lückenhaft. (Geprüft wird die
  Nummerierung an einer Position, die ein anderes Log so gut wie nie zufällig trifft; ein Beweis,
  dass die ganze Historie davor dieselbe Liste ist, ist es nicht.)
- **Die Version.** Ein Schnappschuss einer anderen nxs-Version wird aus dem mitgelieferten Log neu
  gefaltet, und `bind` sagt das; einer von einem nxs, dessen Datenbank dieses nicht lesen kann, wird
  abgewiesen. `--json` meldet, was geschah (`snapshot.views`: `taken` oder `refolded`, mit Grund).

**Wo ein Schnappschuss liegt, entscheidest du** — eine Datei, die du kopierst, ein Bucket, den eine
App führt. Der Relay hält keine Schnappschüsse: Er kann nicht falten, und solange er niemanden
authentifiziert, wäre ein dort abgelegter Schnappschuss ein gefaltetes Brett, das jeder unterschieben
und niemand gegen sein Log prüfen könnte. Ein Schnappschuss trägt das Op-Log und das daraus gefaltete
Brett. Er verrät also, was der Stream selbst jedem verrät, der ihn lesen kann — und nichts von dem,
was nxs nur für diese Maschine hält (Agenten-Transkripte, Sitzungen, Leases). nxs schreibt sie nur
für dich lesbar. Und das **Brett eines Schnappschusses wird genommen, wie es ist**: Es ist so
vertrauenswürdig wie die Maschine, von der es stammt — gib also nur Dateien von Maschinen weiter, die
du kontrollierst. Sein Log wird geprüft wie jede ankommende Op: Jede Unterschrift wird auf der neuen
Maschine erneut geprüft, und die beginnt mit einer eigenen Vertrauensliste — ein Schnappschuss kann
dort also nichts zum Wirken bringen. Einer, den der Import nicht zurücklesen könnte — ein Log mit einem Wert
vom falschen Typ, eine View-Zelle, die nicht zu ihrer Spalte passt —, wird ganz abgewiesen, bevor
etwas geschrieben wird.

Eine App, die nxs einbettet — ein gehostetes Brett, das kalt startet —, macht dasselbe über
`nxs_sync::snapshot::{export, import}`: von einem Replikat exportieren, das auf Stand ist (der Export
bestätigt die Position beim Relay), die Bytes dort ablegen, wo die App ihnen traut, und beim nächsten
Kaltstart in einen leeren Store importieren und ab der Position weitersynchronisieren, die `import`
zurückgibt. Ein Replikat, das schreiben wird, registriert vorher sein ID-Präfix, wie vor jedem ersten
Sync.

## Mit einem Postgres synchronisieren, zum Beispiel Supabase

Wenn lieber eine verwaltete Datenbank die Daten halten soll — fremde Sicherungen, fremde Platten —,
richte den Relay auf Postgres aus. Jedes Postgres geht: Supabase, Neon, RDS oder dein eigenes.

```bash
export NXF_RELAY_BACKEND=postgres
export NXF_RELAY_PG_URL='postgres://user:pass@db.example.com:5432/nxf?sslmode=require'
nxf-relay
```

Der Relay legt seine Tabellen beim ersten Start selbst an, es gibt also keinen Migrationsschritt.

Drei Dinge sind es wert, gewusst zu werden:

- **`sslmode=require`** willst du für alles, was übers Internet erreichbar ist; jeder verwaltete
  Anbieter verlangt es. Die Vorgabe (`prefer`) handelt TLS aus, wenn der Server sie anbietet, und
  fällt sonst auf Klartext zurück — richtig für eine Datenbank im eigenen privaten Netz und falsch
  für eine im offenen Internet. Also `require` schreiben und es auch so meinen. Zeigt der Relay auf
  einen nicht-lokalen Rechner und bleibt auf `prefer`, sagt er das beim Start — statt dein Op-Log
  still im Klartext zu verschicken.
- **Einer eigenen oder selbst signierten CA** wird vertraut, indem `NXF_RELAY_PG_CA_FILE` auf ihre
  PEM-Datei zeigt. Öffentliche Zertifizierungsstellen sind eingebaut und brauchen keine Einrichtung
  — nimm aber nicht an, dass ein verwalteter Anbieter eine öffentliche benutzt. **Supabase signiert
  seine Postgres- und Pooler-Zertifikate mit einer eigenen Wurzel**, ein darauf gerichteter Relay
  scheitert also im Handschlag mit `invalid peer certificate: UnknownIssuer`, bis du dieses
  Zertifikat herunterlädst (Project Settings → Database → SSL Configuration) und
  `NXF_RELAY_PG_CA_FILE=/pfad/zu/prod-ca-2021.crt` setzt. Das ist der eine zusätzliche Schritt, den
  Supabase braucht — und er gilt für jeden Anbieter mit eigener CA.
- **Der Verbindungspool** ist `NXF_RELAY_PG_POOL_MAX_SIZE` (Vorgabe 16) und
  `NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS` (Vorgabe 30).

Ein Backend-Wechsel ändert nichts, was ein Client sehen kann: dieselbe HTTP-Oberfläche, dieselbe
Konvergenz, dieselben Stream-Ids. Von SQLite auf Postgres kommst du, indem du einen zweiten Relay
aufsetzt und neu bindest.

## DynamoDB: selbst bauen

Der Relay hat auch einen DynamoDB-Backend, für serverlose Installationen ohne eigene Platte. Er ist
**nicht** in der ausgelieferten Binary, und zwar mit Absicht: er zieht eine native
Krypto-Werkzeugkette (`cmake`, einen C-Compiler) in jeden Build und macht jeden Download um ~19 MB
schwerer — für etwas, das fast niemand betreibt. Unterstützt ist er trotzdem, eben als Quellbau:

```bash
git clone https://github.com/nxsflow/nexus-flow.git && cd nexus-flow
git checkout v0.55.0                                     # ein genaues Release festnageln
cargo build --release -p nxs-server --bin nxf-relay --features dynamodb
```

Du brauchst `cmake` und eine C-Toolchain auf der Baumaschine, und du musst die beiden Tabellen
selbst anlegen — der Relay legt oder wandert sie nie. Ihre nötige Form steht oben in
`crates/server/src/store_ddb.rs`.

Zeigst du eine **ausgelieferte** Binary auf `NXF_RELAY_BACKEND=dynamodb`, startet sie nicht und sagt
dir genau das: der Backend ist eine Bau-Zeit-Fähigkeit, und dieser Bau trägt sie nicht.

## Was es noch nicht gibt

- **Authentifizierung.** Siehe die Warnung ganz oben. Bis sie da ist, ist die Erreichbarkeit des
  Relays die Zugangskontrolle — weil sie es tatsächlich ist. Unterschriften hindern einen Relay
  daran, eine Anweisung zu fälschen; sie hindern niemanden, der ihn erreicht, daran, den Stream zu
  lesen, Ops zu schreiben, die nichts bewirken, oder Ops zurückzuhalten.
- **Ein Container-Image.** `docker run …/nxf-relay` ist der naheliegende Weg, das hier zu betreiben,
  und es ist auch spezifiziert — aber einen Ein-Befehl-Weg zu veröffentlichen, ein
  *unauthentifiziertes* gemeinsames Board aufzustellen, ist die falsche Reihenfolge. Es kommt nach
  der Authentifizierung.
