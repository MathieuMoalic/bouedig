# Bouedig - every task lives here, run `just <recipe>` inside `nix develop`.

default:
    @just --list

# Apply SQLite migrations (creates ./bouedig.db if needed).
db-migrate:
    cargo run -q -p backend --bin migrate

# Run backend + web client dev servers (web UI on http://localhost:8788).
dev-web:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -q -p backend --bin backend &
    backend_pid=$!
    trap 'kill $backend_pid 2>/dev/null || true' EXIT
    cd frontend-web && dx serve --platform web --port 8788 --open false --interactive false

# Run the Android dev build on a connected device/emulator.
# Requires ANDROID_HOME + ANDROID_NDK_ROOT (and adb from the Nix shell).
dev-android:
    #!/usr/bin/env bash
    set -euo pipefail
    cd frontend-android && dx serve --platform android

# Type-check everything that can be checked on the host.
check:
    cargo check -p shared -p backend -p e2e-tests
    cargo check -p frontend-web --target wasm32-unknown-unknown

# Build a release web bundle (used for production / reverse-proxy deploys).
build-web:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd frontend-web && dx build --platform web --release)
    echo "web bundle: $(pwd)/target/dx/frontend-web/release/web/public"

# Build a (debug) web bundle and run the E2E suite against it.
test-e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd frontend-web && dx build --platform web)
    export BOUEDIG_DIST_DIR="$PWD/target/dx/frontend-web/debug/web/public"
    [ -d "$BOUEDIG_DIST_DIR" ] || { echo "no web bundle at $BOUEDIG_DIST_DIR" >&2; exit 1; }
    just db-migrate
    geckodriver --port 4445 &
    gecko_pid=$!
    trap 'kill $gecko_pid 2>/dev/null || true' EXIT
    cargo test -p e2e-tests -- --nocapture
