# Created implementation issues

Repository: https://github.com/orpiske/midnight-saci/issues

Each child issue has a delivery boundary, acceptance criteria, exclusions, and linked dependencies.

## 1. Establish the Rust project and quality gates

Parent: [#1](https://github.com/orpiske/midnight-saci/issues/1)

- [1.1 Create the Rust project scaffold](https://github.com/orpiske/midnight-saci/issues/13)
- [1.2 Adopt Wanaku quality gates](https://github.com/orpiske/midnight-saci/issues/14)

## 2. Define run, environment, policy, event, and report contracts

Parent: [#2](https://github.com/orpiske/midnight-saci/issues/2)

- [2.1 Define execution types and lifecycle contracts](https://github.com/orpiske/midnight-saci/issues/15)
- [2.2 Load and validate simple YAML configuration](https://github.com/orpiske/midnight-saci/issues/17)
- [2.3 Specify REST APIs and the worker reporting protocol](https://github.com/orpiske/midnight-saci/issues/19)

## 3. Parse, validate, and scaffold Markdown test plans

Parent: [#3](https://github.com/orpiske/midnight-saci/issues/3)

- [3.1 Parse and validate Markdown test plans](https://github.com/orpiske/midnight-saci/issues/18)
- [3.2 Resolve and package reusable procedures](https://github.com/orpiske/midnight-saci/issues/20)
- [3.3 Add offline test-plan scaffolding](https://github.com/orpiske/midnight-saci/issues/21)

## 4. Integrate a Rust agent library and OpenAI-compatible LLM connection

Parent: [#4](https://github.com/orpiske/midnight-saci/issues/4)

- [4.1 Validate and select a Rust agent framework](https://github.com/orpiske/midnight-saci/issues/16)
- [4.2 Integrate the configured LLM connection](https://github.com/orpiske/midnight-saci/issues/22)

## 5. Implement worker shell and HTTP tools

Parent: [#5](https://github.com/orpiske/midnight-saci/issues/5)

- [5.1 Capture evidence and enforce credential boundaries](https://github.com/orpiske/midnight-saci/issues/23)
- [5.2 Implement a persistent bounded shell tool](https://github.com/orpiske/midnight-saci/issues/24)
- [5.3 Implement a bounded HTTP tool](https://github.com/orpiske/midnight-saci/issues/25)
- [5.4 Package and bootstrap the worker image](https://github.com/orpiske/midnight-saci/issues/34)

## 6. Implement sequential agent execution and evidence-based assessment

Parent: [#6](https://github.com/orpiske/midnight-saci/issues/6)

- [6.1 Integrate tools with controlled agent orchestration](https://github.com/orpiske/midnight-saci/issues/29)
- [6.2 Validate assessments and aggregate outcomes](https://github.com/orpiske/midnight-saci/issues/26)
- [6.3 Sequence plan execution and basic policy](https://github.com/orpiske/midnight-saci/issues/32)

## 7. Implement the backend API and durable run records

Parent: [#7](https://github.com/orpiske/midnight-saci/issues/7)

- [7.1 Persist runs, events, evidence and reports](https://github.com/orpiske/midnight-saci/issues/27)
- [7.2 Implement authenticated client-facing REST APIs](https://github.com/orpiske/midnight-saci/issues/30)
- [7.3 Implement run-scoped worker reporting](https://github.com/orpiske/midnight-saci/issues/31)

## 8. Launch and supervise local container workers

Parent: [#8](https://github.com/orpiske/midnight-saci/issues/8)

- [8.1 Launch workers through Podman and Docker](https://github.com/orpiske/midnight-saci/issues/36)
- [8.2 Supervise runs, cancellation and restart recovery](https://github.com/orpiske/midnight-saci/issues/37)

## 9. Deliver the CLI for complete local and remote runs

Parent: [#9](https://github.com/orpiske/midnight-saci/issues/9)

- [9.1 Add backend startup and run submission CLI commands](https://github.com/orpiske/midnight-saci/issues/33)
- [9.2 Add CLI progress, reports and cancellation](https://github.com/orpiske/midnight-saci/issues/35)
- [9.3 Deliver a reproducible local walkthrough](https://github.com/orpiske/midnight-saci/issues/38)

## 10. Add namespace-scoped Kubernetes/OpenShift execution and packaging

Parent: [#10](https://github.com/orpiske/midnight-saci/issues/10)

- [10.1 Implement the Kubernetes Job worker launcher](https://github.com/orpiske/midnight-saci/issues/39)
- [10.2 Package a minimal Kubernetes and OpenShift install](https://github.com/orpiske/midnight-saci/issues/40)
- [10.3 Verify and document namespace deployment workflows](https://github.com/orpiske/midnight-saci/issues/42)

## 11. Extend worker tools through configured MCP servers

Parent: [#11](https://github.com/orpiske/midnight-saci/issues/11)

- [11.1 Configure MCP connections with rmcp](https://github.com/orpiske/midnight-saci/issues/28)
- [11.2 Bridge enabled MCP tools into agent execution](https://github.com/orpiske/midnight-saci/issues/41)

## 12. Add configurable outcome actions and bounded extension hooks

Parent: [#12](https://github.com/orpiske/midnight-saci/issues/12)

- [12.1 Configure and dispatch MCP outcome actions](https://github.com/orpiske/midnight-saci/issues/43)
- [12.2 Persist action approvals and expose API and CLI decisions](https://github.com/orpiske/midnight-saci/issues/44)
- [12.3 Make outcome delivery resilient to retries and restarts](https://github.com/orpiske/midnight-saci/issues/45)
