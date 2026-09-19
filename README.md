# Bouedig

A full-stack Dioxus app: an Axum + SQLite backend, a web (WASM) client and an
Android client.

## Usage

Enter the development shell:

    nix develop

Then use the `just` recipes:

    just dev-web        # backend + web client dev servers
    just dev-android    # Android dev build (needs ANDROID_HOME/ANDROID_NDK_ROOT)
    just build-web      # release web bundle
    just db-migrate     # apply SQLite migrations
    just test-e2e       # build web bundle + run E2E suite (headless Firefox)
    just check          # type-check the workspace
