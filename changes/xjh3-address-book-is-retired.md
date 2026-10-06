---
type: removed
facade: breaking
---
[en]
`address_book` is retired. Who a persona may address is derived from what every other persona and
channel admits — each described in its own words — and no persona keeps a list of its own any more.
A declaration that still carries the key loads unchanged; the key is ignored, `nxs prime` warns
about it, and `nxs personas migrate` leaves it out. A persona that declared `address_book: []` now
sees the derived directory like every other. For embedding apps: `RoleDecl::address_book` and
`AddressBookEntry` are removed, and so are `why` on `PersonaEntry` and `ChannelEntry` and
`unresolved` on `Directory`. The derived `address_book` of `nxc prime --json` is unchanged.
[de]
`address_book` ist abgeschafft. Wen eine Persona ansprechen kann, ergibt sich daraus, was jede andere
Persona und jeder Kanal zulässt — jeweils in deren eigenen Worten beschrieben —, und keine Persona
führt mehr eine eigene Liste. Eine Deklaration, die den Schlüssel noch trägt, lädt unverändert; der
Schlüssel wird ignoriert, `nxs prime` warnt, und `nxs personas migrate` lässt ihn weg. Eine Persona,
die `address_book: []` deklariert hatte, sieht jetzt das abgeleitete Verzeichnis wie jede andere.
Für einbettende Apps: `RoleDecl::address_book` und `AddressBookEntry` entfallen, ebenso `why` an
`PersonaEntry` und `ChannelEntry` und `unresolved` an `Directory`. Das abgeleitete `address_book`
von `nxc prime --json` bleibt unverändert.
