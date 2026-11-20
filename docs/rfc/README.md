# RFCs

`factory/GENESIS.md` describes where nexus-flow is going: the target state of v2. The RFCs here design that target state, one part each, precisely enough to build from. The owner decides every RFC; a coding ticket then builds against a decided RFC, and a pull request that only implements one may be merged without a second review (`factory/CONFIG.md`, merge rule).

Each RFC says at its top whether it is a draft or decided, and it states the design as one complete text. Its "History" section starts with its publication and lists by date what changed since, and in which pull request.

| RFC | Title |
| --- | --- |
| [0001](0001-nexus-flow-on-nocentraldb.md) | nexus-flow on NoCentralDB |
| [0002](0002-tickets.md) | Tickets in NoCentralDB |
| [0003](0003-decisions.md) | decisions in NoCentralDB |
| [0004](0004-genesis.md) | Genesis storage |
| [0005](0005-goals.md) | Goals in NoCentralDB |
| [0006](0006-factory.md) | Factory storage |
| [0007](0007-status.md) | status in NoCentralDB |
| [0009](0009-work-across-repositories.md) | Work across repositories |
| [0010](0010-merge-gate.md) | The merge gate |

Each area of GENESIS.md gets its own RFC: the six modules (genesis, goals, tickets, decisions, factory, status), the work across repositories, the merge gate, and the move from v1 to NoCentralDB.

## Writing one

Copy [`0000-template.md`](0000-template.md) to `NNNN-<slug>.md`, with the next free number, and open it as a draft pull request. Its title starts with the RFC number. A draft names, in "Open questions", every choice it leaves to the owner, with the options and a recommendation, so that each can be decided on its own.

An RFC states its design directly. Each choice sits in the section it belongs to: what was chosen, a sentence on why, and the options not taken where a reader would ask "why not X?". There is no list of decisions, and there are no references such as "decision S13".

## Deciding one

The owner decides an RFC by approving its pull request once "Open questions" is empty: every question there is either answered in the text or moved to a later RFC. The RFC's status line then reads "Decided", with the date, and the pull request is merged. Decisions the owner makes along the way go into the text, and the ones a reader would look for go into the decision log as well (`factory/decisions/`).

## How a decided RFC changes

A decided RFC changes by rewriting its text, never by adding to it. A pull request that changes a decided RFC does three things:

1. The change is written into the text where it belongs, so that the RFC reads as if it had been written that way from the start. The pull request adds no amendment section and no mark such as "(Amendment 3)": an RFC has no amendment sections.
2. The decision goes under "Decisions", in short form: what changed, who decided it and when, why, and the options not taken. This section appears only once a decision is made after the RFC was decided, on new findings.
3. A dated line in "History" names the pull request.

"History" starts at publication with one line, and each later change adds one. The git log keeps how the drafts got there. A change to a decided RFC waits for the owner, as every RFC decision does. A withdrawn decision stays under "Decisions", marked as withdrawn.
