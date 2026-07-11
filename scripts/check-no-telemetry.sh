#!/usr/bin/env bash
set -euo pipefail
root=$(git rev-parse --show-toplevel)
cd "$root"

# Release allowlist: the sole HTTP client dependency and call site implement
# explicitly configured task-provider communication, never product telemetry.
test "$(grep -Ec '^reqwest = ' Cargo.toml)" -eq 1
test "$(grep -RIlE 'reqwest::|Client::new\(|\.send\(\)\.await|TcpStream|UdpSocket|tokio::net|hyper::|tonic::' src --include='*.rs' | sort)" = "src/provider.rs"

# MCP is local stdio only; adding an HTTP/WebSocket transport requires review.
! grep -Eq 'HttpTransport|WebSocket|https?://' src/mcp.rs

# Reject common telemetry/analytics/crash-reporting dependencies, including
# transitive additions visible in the locked release graph.
if grep -Ei '^(name = )?"?(sentry|opentelemetry|telemetry|segment|mixpanel|amplitude|posthog|datadog|newrelic|honeycomb)' Cargo.toml Cargo.lock package.json package-lock.json; then
  echo "product telemetry dependency detected" >&2
  exit 1
fi

cargo test --locked --test no_product_telemetry
echo "Outbound call-site/dependency inventory and deny-network proof passed."
