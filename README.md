# Bouedig

A full-stack Dioxus app: an Axum + SQLite backend, a web (WASM) client and an
Android client.

## Usage

Enter the development shell:

    nix develop

All environment knobs (backend port, DB URL, dx dev port, E2E ports) live in
the repo-root `.env` and are loaded automatically by every `just` recipe.

Then use the `just` recipes:

    just dev-web        # backend + web client dev servers (http://localhost:8788)
    just dev-android    # backend + Android dev build (needs ANDROID_HOME/ANDROID_NDK_ROOT)
    just build-web      # release web bundle
    just db-migrate     # apply SQLite migrations
    just test-e2e       # build web bundle + run E2E suite (headless Firefox)
    just check          # type-check the workspace

The web client logs all API calls and panics to the browser console; the
Android client logs via `tracing` (filter with `RUST_LOG`).
