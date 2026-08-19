# AutoPCB Routing Integration Boundary

Date: 2026-08-19

Outboard should become the executable-plugin boundary for AutoPCB external routers, native DRC workers, and proprietary tool adapters.

## Intended AutoPCB uses

- Altium-native DRC worker;
- KiCad/PNS or external route-repair worker;
- FreeRouting adapter;
- learned-policy worker;
- GPU/ML candidate scorer;
- long-running native-tool process with health checks;
- route oracle for dataset generation.

## Boundary rule

External workers produce candidates and evidence.  AutoPCB's route kernel remains authoritative for route-state commit and final verification.

```text
AutoPCB RouteTask
    -> outboard plugin request
    -> external candidate or native report
    -> AutoPCB verifier and evaluator
```

No external plugin should receive authority to mutate committed AutoPCB route state directly.

## Suggested interface crates

Future AutoPCB plugin interfaces should be narrow:

- `autopcb-solver-api`: route-task request and candidate response;
- `autopcb-native-drc-api`: native DRC request and report;
- `autopcb-policy-api`: learned action ranking over semantic route actions;
- `autopcb-geometry-api`: batch broad-phase or scoring requests, never final commit.

## Outboard requirements for this integration

1. deterministic manifest capture in route-event logs;
2. stable protocol-version negotiation;
3. timeout and cancellation per route stage;
4. persistent workers for expensive native tools;
5. explicit stdout/stderr capture into reproducibility artifacts;
6. doctor commands capable of proving native tool availability before a routing run.

## First implementation slice

Once AutoPCB's `RouteTransaction` API stabilizes, add a minimal demo plugin:

```text
autopcb-solver-echo
```

It should read a task, emit no route changes, and prove the manifest/provenance path end to end.  Real adapters can then replace the echo worker without changing AutoPCB core semantics.
