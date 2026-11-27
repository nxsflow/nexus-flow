# 2026-11-27 · NEX-108 · A free tag is picked by the new member's id, among the free ones

- Decided by: coder on NEX-108
- Why: RFC 0002 says the inviter "picks one that is free" and counts on two inviters picking offline at once rarely giving two members one tag. The first free tag in order would make that collision certain whenever it can happen. So `nxs member add` takes the free tag at the place its new member's id, which is random, gives it among the free tags. The same copy and member always give the same tag, and no random source is added.
- Instead of: the first free tag in order, which two concurrent inviters always share; or a random number generator, which adds a dependency and makes the pick depend on more than the member.
- Topics: members, tickets, store
