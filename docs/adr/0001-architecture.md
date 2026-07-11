# ADR 0001: Provisional architecture and license

**Status:** Accepted for the first slice

Aster uses a Rust control plane with a provider-neutral `PiAdapter`, SQLite for local durable state, and Ratatui/Crossterm for the terminal interface. The adapter boundary prevents UI, routing, and persistence from depending on an unverified Pi transport contract. The deterministic fake validates the process boundary without claiming live Pi compatibility.

MIT is provisionally selected for original code because it is permissive and simple. Distribution compatibility and notices must be revisited after Pi source and license inspection; no Pi code is currently vendored or distributed.

Effectful operations will ultimately pass through a capability broker or an enforced isolation boundary. The current slice performs no workspace mutation, network access, credential access, or external publication and must not be described as providing full sandbox enforcement.
