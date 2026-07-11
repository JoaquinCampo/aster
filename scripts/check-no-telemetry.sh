#!/usr/bin/env bash
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"

# Release allowlist: HTTP call sites implement explicitly configured task-provider
# or capability-mediated MCP communication, never product telemetry.
test "$(grep -Ec '^reqwest = ' Cargo.toml)" -eq 1
test "$(grep -RIlE 'reqwest::|Client::new\(|\.send\(\)\.await|TcpStream|UdpSocket|tokio::net|hyper::|tonic::' src --include='*.rs' | sort)" = $'src/mcp.rs\nsrc/provider.rs'

# MCP HTTP must retain explicit disclosure and runtime broker mediation.
grep -q 'NetworkDisclosure' src/mcp.rs
grep -q 'EffectBrokerMediator' src/mcp.rs
grep -q 'authorize_network' src/effects.rs

# Reject common telemetry/analytics/crash-reporting dependencies, including
# transitive additions visible in the locked release graph.
if grep -Ei '^(name = )?"?(sentry|opentelemetry|telemetry|segment|mixpanel|amplitude|posthog|datadog|newrelic|honeycomb)' Cargo.toml Cargo.lock package.json package-lock.json; then
  echo "product telemetry dependency detected" >&2
  exit 1
fi

cargo test --locked --test no_product_telemetry
echo "Outbound call-site/dependency inventory and deny-network proof passed."
