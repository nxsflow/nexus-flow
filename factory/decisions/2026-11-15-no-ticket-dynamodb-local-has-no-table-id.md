# 2026-11-15 · no ticket · DynamoDB Local returned no TableId: test a re-created table via CreationDateTime more than 1 s apart, and TableId rules on constructed descriptions only.

- Decided by: recorded in the project memory (`nxm`, key `dynamodb-local-has-no-table-id`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: dynamodb, testing

## The record

A DynamoDB Local (the one serving localhost:8000 on the development machine, version not checked; whether CI's amazon/dynamodb-local:latest behaves the same is UNVERIFIED) answered DescribeTable on 2026-10-23 (6j6v.dbmd MF-3) with a TableArn of the form arn:aws:dynamodb:ddblocal:000000000000:table/<name> and a CreationDateTime with millisecond precision, but with NO TableId (null). So a test that a re-created table of the same name has a new identity should see it through CreationDateTime: delete, wait MORE than one second, re-create (f6yb MF-4 / dbmd MF-4) — and must not rely on TableId. Rules on TableId itself (a new TableId means another table) are tested on constructed TableDescriptions only. ItemCount was current there (AWS refreshes it about every six hours), so EmptyStore after a purge behaves as on a fresh real table.
