# 2026-11-24 · NEX-133 · A tracked GENESIS prints the report on stdout and the error on stderr

- Decided by: coder on NEX-133
- Why: RFC 0004 says the `tracked` case "exits non-zero with this report on standard output" and that the error names the file and says to restore it from git. stdout already carries the report, so `nxs genesis create` writes the error to stderr as `error: <msg>`, in JSON and in Markdown alike, and exits 1. An agent reads the report from stdout as it does on success, and a person sees why it failed.
- Instead of: v1's error object on stdout, which would replace the report the RFC puts there, or the message inside the report, a key the RFC's output does not have.
- Topics: genesis, cli, output, v2
