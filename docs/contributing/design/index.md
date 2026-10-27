# Design notes

Design notes give the reasons for the main decisions in Saci. Each note
has a status: *historical* notes describe the plan for the first
implementation, and *implemented* notes describe a feature that the user
documentation now covers.

## Notes in this section

| Note | Status | Purpose |
|------|--------|---------|
| [System plan](system-plan.md) | Historical | The goal, the system shape and the initial scope of Saci. |
| [Execution contract](execution-contract.md) | Historical | The worker contract that the first implementation was built against. |
| [Implementation tickets](implementation-tickets.md) | Historical | The feature groups of the first implementation. |
| [Issue index](issue-index.md) | Historical | The GitHub issues created from those tickets. |
| [Failure investigations](investigations.md) | Implemented | Analyze a failed step with a second worker mode, reached over A2A. |
| [Source watch](source-watch.md) | Implemented | Run test plans when watched Git branches change. |
| [Credential and misuse guard](guard.md) | In progress | Find credentials in evidence, and check tool calls and output with local security classifiers. |

## Next steps

- [Saci internals](../internals.md): the library decisions.
- [Development](../development.md): build and check the workspace.
