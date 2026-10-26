---
type: fixed
---
[en]
`nxf-relay` with the DynamoDB backend can be installed from source again: `cargo install --locked --git https://github.com/nxsflow/nexus-flow.git nxs-server --features dynamodb --bin nxf-relay` failed since 0.217.0 with "package `nxs-server` does not have a dependency named `nxs-fold-ddb`", because the `dynamodb` feature named a crate the server only uses in its own tests. CI now runs exactly that install on every change.
[de]
`nxf-relay` mit dem DynamoDB-Backend lässt sich wieder aus dem Quellcode installieren: `cargo install --locked --git https://github.com/nxsflow/nexus-flow.git nxs-server --features dynamodb --bin nxf-relay` schlug seit 0.217.0 mit „package `nxs-server` does not have a dependency named `nxs-fold-ddb`" fehl, weil das Feature `dynamodb` ein Crate nannte, das der Server nur in seinen eigenen Tests verwendet. CI führt genau diese Installation jetzt bei jeder Änderung aus.
