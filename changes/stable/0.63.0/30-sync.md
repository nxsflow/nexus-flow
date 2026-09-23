---
type: added
---
[en]
**Sync runs by itself, and it can run on your own Postgres.** `nxs sync daemon` keeps every bound workspace in sync continuously instead of on demand, and `nxs sync bind` no longer needs `--create` or `--join` — run it with no flags and it derives the stream id deterministically. The `nxf-relay` in every release archive can now use a Postgres, Supabase included, so you can host the bus yourself.
[de]
**Sync läuft von selbst — und auf Ihrem eigenen Postgres.** `nxs sync daemon` hält jeden gebundenen Workspace fortlaufend synchron statt nur auf Zuruf, und `nxs sync bind` braucht kein `--create` oder `--join` mehr: ohne Flags aufgerufen leitet es die Stream-Id deterministisch ab. Der `nxf-relay` in jedem Release-Archiv kann jetzt ein Postgres benutzen, Supabase eingeschlossen — Sie können den Bus also selbst betreiben.
