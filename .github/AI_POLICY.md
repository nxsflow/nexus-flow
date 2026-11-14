# AI policy

AI tools and agents are welcome in nexus-flow. Much of this project is built with them, and we expect contributors to use them too.

This policy applies to everyone: outside contributors, the maintainers, and the project's own agents. There is no exemption for maintainers, on purpose. The rules that make an outside pull request trustworthy make ours trustworthy too.

## The rule

An agent may write the change. A named human must understand it, be able to explain it, and answer review on it, either personally or by reading every agent-drafted reply before it is posted.

## Pull requests

- **Disclose how the change was written.** The pull request template's Authorship section asks for it: written by a human, by an agent under close human direction, by an agent with human review, by an agent checked against a decided design, as an agent's beta release notes, by an agent that carries out a decision, or as an agent's tooling change. Check the one that is true.
- **The first exception: a decided design.** The project's own pm agent may merge a pull request that no person has reviewed line by line, when the pull request only implements a decided RFC. A decided RFC is one whose decisions the owner has made and that is merged on `main`. The pull request names the RFC sections it implements and departs from none of them. It may build a safe subset of them: a case it does not build yet is refused or parked safely (data is kept, and nothing the RFC would refuse is accepted), named in the pull request, and covered by a follow-up ticket. A choice the RFC leaves open inside the core, below the SDK, may be made and recorded in the decision log. A departure from the RFC is never decided in the pull request: it goes to the owner as its own question. It brings in no public name, format or runtime dependency that the RFC does not name, and it changes no gate. pm checks it against the RFC and the gates, ticks "Agent, checked against a decided design", and lists it in the daily note, so that the owner can read it after the merge and ask for a revert or a follow-up. The owner is still the named human who answers review on it.
- **The second exception: a beta's release notes.** The pm agent may merge the pull request that carries a beta's release notes, curated by an agent from the change fragments, without a person's review. The pull request changes nothing but the beta entry of `release-notes.json`. pm checks that every entry comes from a fragment, ticks "Agent, beta release notes", and lists it in the daily note. The notes of a stable release still go to the owner, who curates them.
- **The third exception: carrying out a decision.** The pm agent may merge a pull request that changes documentation only and decides nothing new. Each change either records a decision the owner made, with its date and where it was made, or makes a text follow RFC text that is already merged on `main`, and says which. It asks the owner nothing: it holds no open decision for the owner. It does not touch `factory/GENESIS.md`, `README.md`, `factory/` or `.github/`. pm checks every change against its source, ticks "Agent, carries out a decision", and lists it in the daily note.
- **The fourth exception: tooling.** The pm agent may merge a pull request that changes only the tools developers and agents work with, such as session hooks, the development build profile or comments. It changes no gate, no CI or release workflow beyond comments, no product code, no public API, no runtime dependency, no factory rule and no policy. pm checks that, ticks "Agent, tooling", and lists it in the daily note.
- **Everything else waits for a person's review**, outside contributions included.
- **Name who follows up.** The template's `Follow-up:` line names the person who answers review. A pull request that no human has committed to answer is closed.
- A fix comes with a test that fails without it, and "How it was tested" lists only what was actually run.

## Issues, discussions and comments

They may be drafted with AI, but a human reviews and edits each one before it is posted, and stands behind it. Long, generic text that does not engage with this project costs the reader time; say what you observed, in your own words where it matters.

## Fabricated reports

A bug report, security report or reproduction that is made up (a crash that never happened, an output that was never produced, a vulnerability in code that does not exist) gets the reporter blocked from the project. An honest mistake is not a fabrication: a report that turns out to be wrong is fine.

## What we do not do

We keep no public list of people or accounts who broke this policy. Enforcement happens on the pull request, issue or account concerned, and nowhere else.
