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

## Deployment (NixOS)

The flake provides everything needed to run Bouedig on a server:

    nix build .#bouedig          # backend + migrate binaries, web bundle under share/bouedig/web
    nix build .#web              # just the release web bundle
    nix build .#release-tarball  # tarball published by CI on tag push
    nix build .#prebuilt         # same tarball fetched from GitHub releases (needs prebuiltHash set)

`nixosModules.bouedig` is a `services.bouedig` NixOS module with a hardened
systemd service (dedicated user, `/var/lib/bouedig` state):

```nix
{
  inputs.bouedig.url = "github:MathieuMoalic/bouedig";

  outputs = { self, nixpkgs, bouedig, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        bouedig.nixosModules.bouedig
        {
          services.bouedig = {
            enable = true;
            bindAddr = "127.0.0.1:10025";
            # Recipe browsing stays public; everything else needs the password.
            # Unset -> auth is off entirely.
            passwordFile = config.sops.secrets."bouedig/password".path;
            openrouterKeyFile = config.sops.secrets."bouedig/llm-api-key".path;
          };
          services.caddy.virtualHosts."bouedig.example.com".extraConfig = ''
            reverse_proxy 127.0.0.1:10025
          '';
        }
      ];
    };
  };
}
```

A full example mirroring the blaz deployment (sops-nix secrets + Caddy vhost)
lives in [deploy/nixconfig-example.nix](deploy/nixconfig-example.nix).

### Releases

1. Bump `version` in `Cargo.toml` (`[workspace.package]`) and `flake.nix`.
2. Commit and tag: `git tag v0.1.0 && git push --tags` — CI builds the
   tarball and attaches it to the GitHub release.
3. Paste the `nix hash file` value from the CI job summary into
   `prebuiltHash` in `flake.nix`; `packages.prebuilt` (used by nixconfig)
   becomes buildable.

To try the module locally without touching a server:

    nixos-rebuild build-vm --flake .#bouedig-vm
    ./result/bin/run-bouedig-vm-vm   # then open http://localhost:10025
