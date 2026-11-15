# 2026-11-15 · no ticket · A test's NAME is a claim that can be false while all stays green — only mutation finds it, and a mutation that reds nothing is itself a finding.

- Decided by: recorded in the project memory (`nxm`, key `a-test-name-is-a-claim`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, verification

## The record

The NAME of a test is a claim about its body — and it can be false without anything going red.

Twice during the nexus-chat surface work (August 2026):

- `stream_is_refused_to_a_persona_and_says_what_to_do_instead` promised the second half in its own
  name and checked NONE of it. That is exactly why a verb the same commit had removed from the
  surface survived in the message.
- `each_member_thread_names_its_own_assignee_with_no_tie_break_left_to_lose` claimed no tie-break
  was left. In the scenario as built, even a BROKEN outer guard produced byte-identical output,
  because the inner one already excluded that session. Both guards were redundant there.

THE ONLY CHECK THAT FINDS THIS IS MUTATION: remove the mechanism the test claims to check and see
whether the test goes red. If it stays green, it is checking something other than what its name
says.

THIS APPLIES TO MUTATIONS THEMSELVES. Twice a mutation made nothing red, and both times the reason
was in the setup rather than in the subject:
- The release path called a `pub(crate)` twin function while the mutation hit the `pub` shell that
  no production code used any more.
- A prescribed assertion pinned the STATE a predicate reads instead of the predicate FIRING —
  closing the door left the state untouched and the test green.

A mutation that makes nothing red is not proof of correctness; it is a finding about the test.
