# ADR 0001: Provisional architecture and license

**Status:** Accepted for the first slice

Aster uses a Rust control plane with a provider-neutral `PiAdapter`, SQLite for local durable state, and Ratatui/Crossterm for the terminal interface. The adapter boundary prevents UI, routing, and persistence from depending on an unverified Pi transport contract. The deterministic fake validates the process boundary without claiming live Pi compatibility.

MIT is selected for Aster's original code. Upstream `badlogic/pi-mono` at inspected commit `8479bd84743e8889f728acb21a62794102db0529` is also MIT-licensed (copyright 2025 Mario Zechner), as are the locally installed `@mariozechner/pi-agent-core` and `@mariozechner/pi-ai` version 0.73.0 package manifests. The licenses are compatible. Aster does not currently vendor or distribute Pi source or binaries; if that changes, Pi's copyright and permission notice must accompany substantial copied or bundled portions. `LICENSE` covers Aster and `THIRD_PARTY_NOTICES.md` records the current Pi obligation.

Effectful operations will ultimately pass through a capability broker or an enforced isolation boundary. The current slice performs no workspace mutation, network access, credential access, or external publication and must not be described as providing full sandbox enforcement.
