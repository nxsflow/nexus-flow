---
type: added
---
[en]
`nxs sync snapshot` (both formats, `--based` included) can reach the relay with a bearer token instead of a user and password in the relay URL: `--bearer-stdin` reads the token from stdin, one line, and sends it as `Authorization: Bearer`. The token never appears on the command line, in the environment, in any output or error. A relay URL that also carries a user and password is refused before any request, and a relay that refuses the token (401/403) is a `forbidden` error that names the status.
[de]
`nxs sync snapshot` (beide Formate, auch `--based`) erreicht das Relay jetzt auch mit einem Bearer-Token statt mit Benutzer und Passwort in der Relay-URL: `--bearer-stdin` liest das Token von stdin, eine Zeile, und sendet es als `Authorization: Bearer`. Das Token erscheint nie auf der Kommandozeile, in der Umgebung, in einer Ausgabe oder einem Fehler. Eine Relay-URL, die zusätzlich Benutzer und Passwort trägt, wird vor jeder Anfrage abgewiesen, und ein Relay, das das Token ablehnt (401/403), ergibt einen `forbidden`-Fehler, der den Status nennt.
