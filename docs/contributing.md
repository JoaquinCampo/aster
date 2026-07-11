# Contributing

## Setup

Use stable Rust. Work from a clean branch and never commit `.aster/`, provider transcripts, credentials, or generated keys.

```sh
cargo build --locked
cargo test --locked --all-targets --all-features
```

Install hooks with `pre-commit install` if `pre-commit` is available. The hook configuration runs formatting, clippy, tests, and the tracked-file secret scan. The commands remain directly runnable without Python tooling.

## Change standard

* Keep role/model/effort and isolation dimensions independent.
* Route effects through the broker; add both allow and deny tests.
* Preserve durable intent/start/outcome ordering and unknown-outcome recovery.
* Use deterministic providers in routine tests; live/paid tests must be explicit.
* Update `docs/acceptance-matrix.md` whenever a BRIEF requirement changes status or evidence.
* Label fixture-only and manual evidence accurately.
* Update operations, configuration, security, recovery, and extension guidance when their contracts change.

Before commit:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
./scripts/scan-secrets.sh
```

After the candidate commit, run the clean-checkout validator. A release additionally requires successful hosted CI, preserved macOS arm64 evidence, and review of known limitations. Conventional commit subjects are preferred; keep commits focused and do not bypass hooks to conceal failures.
