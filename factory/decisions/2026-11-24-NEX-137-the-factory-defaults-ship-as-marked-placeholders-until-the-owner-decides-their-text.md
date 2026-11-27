# 2026-11-24 · NEX-137 · The factory's defaults ship as marked placeholders until the owner decides their text

- Decided by: coder on NEX-137
- Why: RFC 0006 makes each default's text a template the owner decides in its own ticket (NEX-140, and NEX-174 for the merge gate's files), but the install needs files to embed and hash. `templates/factory/` holds `FACTORY.md`, `CONFIG.md`, `roles/pm.md`, `roles/coder.md` and `status-note.md`, each a heading and one line under the first line `<!-- nexus-flow: placeholder. The owner decides this default's text (NEX-140); until then nexus-flow ships this placeholder. -->`, and `shipped.toml` lists one hash each. No `merge-gate/` is shipped until NEX-174 decides it. When the owner's text lands, its hash is added after the placeholder's, so a repository that installed a placeholder sees it as `older_default` and `--update` replaces it. A test holds the embedded table equal to the folder.
- Instead of: shipping this repository's own factory files as the defaults, which would decide the owner's texts in a coding PR, or waiting with the install until the texts are decided.
- Topics: factory, templates, v2
