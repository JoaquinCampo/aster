# Routing benchmark report v1

Schema version: `1`. Baseline: immutable `fixed-strong` profile (quality 95, cost 2000 micros/M tokens, latency 700 ms, context 128k). Inputs and model profiles are deterministic fixtures, not production measurements.

| Scenario | Required quality | Tokens | Selected | Baseline | Quality | Cost | Latency | UX | Context |
|---|---:|---:|---|---|---|---|---|---|---|
| Brief answer | 50 | 1,000 | fake-luna | fixed-strong | threshold met | lower | lower | direct/no delegation | 4k route |
| Repository refactor | 80 | 20,000 | fake-terra | fixed-strong | threshold met | lower | lower | specialist delegation | 20k route |
| Critical review | 95 | 60,000 | fixed-strong | fixed-strong | equal | equal | equal | escalation visible | 60k route |

The executable fixture is `tests/routing_policy.rs`. Results compare five dimensions: required-quality satisfaction, estimated profile cost, profile latency, delegation/override UX, and allocated context. This report does not claim real provider quality, prices, or end-to-end latency; production profiles require calibrated, versioned evidence.
