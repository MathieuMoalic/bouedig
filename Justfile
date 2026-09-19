# Bouedig - every task lives here, run `just <recipe>` inside `nix develop`.

default:
    @just --list

# Apply SQLite migrations (creates ./bouedig.db if needed).
db-migrate:
    cargo run -q -p backend --bin migrate

# Run backend + web client dev servers (web UI on http://localhost:8080).
dev-web:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -q -p backend --bin backend &
    backend_pid=$!
    trap 'kill $backend_pid 2>/dev/null || true' EXIT
    cd frontend-web && dx serve --platform web

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
    just _locate-dist

_locate-dist:
    #!/usr/bin/env bash
    set -euo pipefail
    if   [ -d frontend-web/dist ]; then echo "web bundle: $(pwd)/frontend-web/dist"
    elif [ -d dist ];             then echo "web bundle: $(pwd)/dist"
    else echo "warning: no dist directory found" >&2; fi

# Build a (debug) web bundle and run the E2E suite against it.
test-e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd frontend-web && dx build --platform web)
    if   [ -d frontend-web/dist ]; then export BOUEDIG_DIST_DIR="$PWD/frontend-web/dist"
    elif [ -d dist ];             then export BOUEDIG_DIST_DIR="$PWD/dist"
    else echo "no web bundle produced by dx build" >&2; exit 1; fi
    just db-migrate
    geckodriver --port 4445 &
    gecko_pid=$!
    trap 'kill $gecko_pid 2>/dev/null || true' EXIT
    cargo test -p e2e-tests -- --nocapture
