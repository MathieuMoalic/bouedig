{
  description = "Bouedig - full-stack Dioxus app (Axum backend, Web + Android clients)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , rust-overlay
    , flake-utils
    ,
    }:
    let
      # Keep in sync with [workspace.package] version in Cargo.toml; bump
      # together with the `vX.Y.Z` release tag.
      version = "0.1.1";

      systems = [ "x86_64-linux" "aarch64-linux" ];

      pkgsFor = system:
        import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
          # Mirror blaz: the Android SDK components are unfree and need the
          # licenses accepted for `androidenv.composeAndroidPackages` to work.
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
        };

      # The web bundle needs a big filter anyway, and cargo wants the whole
      # workspace (root Cargo.toml/lock + every member) to resolve.
      bouedigSrc = pkgs: pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: _type:
          let base = baseNameOf path;
          in !builtins.elem base [
            "target" "result" "dist" ".direnv" ".github" ".zcode"
            "deploy" "docs" "Justfile" "README.md" "flake.nix" "flake.lock"
          ];
      };

      # Release web bundle (dx handles wasm-bindgen itself; only wasm-opt
      # comes from PATH via binaryen).
      mkWeb = pkgs:
        let toolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
        };
        in
        pkgs.stdenv.mkDerivation {
          pname = "bouedig-web";
          inherit version;
          src = bouedigSrc pkgs;
          nativeBuildInputs = [ toolchain pkgs.dioxus-cli pkgs.binaryen ];

          cargoDeps = pkgs.rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
          };

          env = {
            CARGO_NET_OFFLINE = "true";
            # Keep dx's caches out of $HOME: the sandbox has no network and
            # no writable home.
            XDG_CACHE_HOME = "/build/.cache";
            XDG_CONFIG_HOME = "/build/.config";
            XDG_DATA_HOME = "/build/.data";
          };

          configurePhase = ''
            runHook preConfigure
            # cargoDeps is a cargo-vendor directory (importCargoLock), so
            # point cargo's source replacement at it and give cargo a
            # writable CARGO_HOME for its caches.
            export CARGO_HOME="$PWD/.cargo-home"
            mkdir -p "$CARGO_HOME" .cargo
            cat > .cargo/config.toml <<EOF
            [source.crates-io]
            replace-with = "vendored-sources"

            [source.vendored-sources]
            directory = "$cargoDeps"
            EOF
            runHook postConfigure
          '';

          buildPhase = ''
            runHook preBuild
            (cd frontend-web && dx build --platform web --release)
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp -r target/dx/frontend-web/release/web/public/. $out/
            runHook postInstall
          '';

          meta = with pkgs.lib; {
            description = "Bouedig web client (release bundle)";
            license = licenses.mit;
          };
        };

      # Backend + migrate binaries, with the web bundle embedded under
      # share/bouedig/web (served via BOUEDIG_STATIC_DIR).
      mkBouedig = pkgs: pkgs.rustPlatform.buildRustPackage {
        pname = "bouedig";
        inherit version;
        src = bouedigSrc pkgs;
        cargoLock.lockFile = ./Cargo.lock;
        buildAndTestSubdir = "backend";
        doCheck = false;
        postInstall = ''
          mv $out/bin/backend $out/bin/bouedig
          mkdir -p $out/share/bouedig/web
          cp -r ${mkWeb pkgs}/. $out/share/bouedig/web/
        '';
        meta = with pkgs.lib; {
          description = "Bouedig recipe manager (backend + web client)";
          homepage = "https://github.com/MathieuMoalic/bouedig";
          license = licenses.mit;
          platforms = systems;
          mainProgram = "bouedig";
        };
      };

      # Single tarball published by CI on tag push; the `prebuilt` package
      # below fetches exactly this artifact.
      mkReleaseTarball = pkgs: bouedig: pkgs.stdenvNoCC.mkDerivation {
        pname = "bouedig-release-tarball";
        inherit version;
        src = bouedig;
        nativeBuildInputs = [ pkgs.gnutar pkgs.gzip ];
        installPhase = ''
          runHook preInstall
          mkdir -p $out staging
          cp -r $src/bin staging/bin
          cp -r $src/share staging/share
          tar -czf $out/bouedig-v${version}-${pkgs.stdenv.hostPlatform.system}.tar.gz -C staging .
          runHook postInstall
        '';
        meta = with pkgs.lib; {
          description = "Bouedig release tarball (binaries + web bundle)";
          license = licenses.mit;
        };
      };

      # sri hash of the latest release tarball; CI prints `nix hash file` for
      # every release — paste it here to make `packages.prebuilt` buildable.
      prebuiltHash = "sha256-iAUAZP22smxaSJ6khB9cYnNwFJaQYwJQlacpyRJIFVo=";

      # Same trick blaz uses: the release tarball carries stale /nix/store
      # references from the build machine, so the fetchurl's references are
      # discarded and the binaries get repointed at this system's loader and
      # libraries.
      mkPrebuilt = pkgs: pkgs.stdenvNoCC.mkDerivation {
        pname = "bouedig";
        inherit version;
        src = (pkgs.fetchurl {
          url = "https://github.com/MathieuMoalic/bouedig/releases/download/v${version}/bouedig-v${version}-x86_64-linux.tar.gz";
          hash = prebuiltHash;
        }).overrideAttrs {
          unsafeDiscardReferences = { out = true; };
        };
        dontUnpack = true;
        dontBuild = true;
        nativeBuildInputs = [ pkgs.patchelf ];
        installPhase = ''
          runHook preInstall
          # The tarball has two top-level entries (bin/ + share/), which the
          # stdenv unpacker rejects, so unpack manually.
          mkdir work && cd work
          tar xzf $src
          mkdir -p $out/share/bouedig
          cp -r bin $out/bin
          cp -r share/bouedig/web $out/share/bouedig/web
          for b in $out/bin/*; do
            patchelf \
              --set-interpreter ${pkgs.stdenv.cc.bintools.dynamicLinker} \
              --set-rpath ${pkgs.lib.makeLibraryPath [ pkgs.stdenv.cc.cc.lib pkgs.glibc ]} \
              $b
          done
          runHook postInstall
        '';
        meta = with pkgs.lib; {
          description = "Bouedig recipe manager (prebuilt release)";
          homepage = "https://github.com/MathieuMoalic/bouedig";
          license = licenses.mit;
          platforms = [ "x86_64-linux" ];
          mainProgram = "bouedig";
        };
      };

      serviceModule = { lib, config, pkgs, ... }:
        let
          cfg = config.services.bouedig;
          defaultPackage =
            self.packages.${pkgs.stdenv.hostPlatform.system}.bouedig
            or (throw "bouedig: no package for ${pkgs.stdenv.hostPlatform.system}");
        in
        {
          options.services.bouedig = {
            enable = lib.mkEnableOption "Bouedig recipe manager";

            package = lib.mkOption {
              type = lib.types.package;
              default = defaultPackage;
              description = "The bouedig package to use.";
            };

            bindAddr = lib.mkOption {
              type = lib.types.str;
              default = "127.0.0.1:10025";
              description = "Address to bind the HTTP server to.";
            };

            databasePath = lib.mkOption {
              type = lib.types.str;
              default = "/var/lib/bouedig/bouedig.sqlite";
              description = "Path to the SQLite database file.";
            };

            dataDir = lib.mkOption {
              type = lib.types.str;
              default = "/var/lib/bouedig";
              description = "Data directory (recipe images live under images/).";
            };

            staticDir = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Directory with the web client bundle. Defaults to the copy
                embedded in the package.
              '';
            };

            basePath = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/bouedig";
              description = "Serve the app under a sub-path instead of /.";
            };

            password = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = ''
                Household password: recipe browsing stays public, everything
                else needs it. Leave unset to disable auth entirely.
              '';
            };

            passwordFile = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "Path to a file with the password (for sops-nix).";
            };

            secureCookies = lib.mkOption {
              type = lib.types.bool;
              default = true;
              description = ''
                Add the `Secure` attribute to session cookies. Keep this on
                when the app is served over TLS (Caddy); plain-HTTP deploys
                must turn it off or login breaks.
              '';
            };

            openrouterKey = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "OpenRouter API key for JEV grocery categorization.";
            };

            openrouterKeyFile = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
              description = "Path to a file with the OpenRouter API key (for sops-nix).";
            };

            classifierModel = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "OpenRouter model for the classifier (default typesafe/jev-router).";
            };

            classifierEndpoint = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              description = "Classifier API endpoint override.";
            };

            importAllowPrivate = lib.mkOption {
              type = lib.types.bool;
              default = false;
              description = "Allow recipe import from private/loopback URLs.";
            };
          };

          config = lib.mkIf cfg.enable {
            assertions = [
              {
                assertion = !(cfg.password != null && cfg.passwordFile != null);
                message = "services.bouedig.password and services.bouedig.passwordFile are mutually exclusive";
              }
              {
                assertion = !(cfg.openrouterKey != null && cfg.openrouterKeyFile != null);
                message = "services.bouedig.openrouterKey and services.bouedig.openrouterKeyFile are mutually exclusive";
              }
            ];

            users.users.bouedig = {
              isSystemUser = true;
              group = "bouedig";
              home = cfg.dataDir;
              createHome = true;
            };
            users.groups.bouedig = { };

            systemd.tmpfiles.rules = [
              "d ${cfg.dataDir} 0750 bouedig bouedig - -"
              "d ${lib.dirOf cfg.databasePath} 0750 bouedig bouedig - -"
            ];

            systemd.services.bouedig = {
              description = "Bouedig recipe manager";
              after = [ "network.target" ];
              wantedBy = [ "multi-user.target" ];

              environment = {
                BOUEDIG_ADDR = cfg.bindAddr;
                BOUEDIG_DB_URL = "sqlite://${cfg.databasePath}?mode=rwc";
                BOUEDIG_DATA_DIR = cfg.dataDir;
                BOUEDIG_STATIC_DIR =
                  if cfg.staticDir == null
                  then "${cfg.package}/share/bouedig/web"
                  else cfg.staticDir;
              }
              // lib.optionalAttrs (cfg.basePath != null) { BOUEDIG_BASE_PATH = cfg.basePath; }
              // lib.optionalAttrs (cfg.password != null) { BOUEDIG_PASSWORD = cfg.password; }
              // lib.optionalAttrs cfg.secureCookies { BOUEDIG_SECURE_COOKIES = "1"; }
              // lib.optionalAttrs (cfg.openrouterKey != null) { BOUEDIG_OPENROUTER_KEY = cfg.openrouterKey; }
              // lib.optionalAttrs (cfg.classifierModel != null) { BOUEDIG_CLASSIFIER_MODEL = cfg.classifierModel; }
              // lib.optionalAttrs (cfg.classifierEndpoint != null) { BOUEDIG_CLASSIFIER_ENDPOINT = cfg.classifierEndpoint; }
              // lib.optionalAttrs cfg.importAllowPrivate { BOUEDIG_IMPORT_ALLOW_PRIVATE = "1"; };

              script = ''
                ${lib.optionalString (cfg.passwordFile != null) ''
                  export BOUEDIG_PASSWORD="$(cat ${cfg.passwordFile})"
                ''}
                ${lib.optionalString (cfg.openrouterKeyFile != null) ''
                  export BOUEDIG_OPENROUTER_KEY="$(cat ${cfg.openrouterKeyFile})"
                ''}
                exec ${cfg.package}/bin/bouedig
              '';

              serviceConfig = {
                WorkingDirectory = cfg.dataDir;
                User = "bouedig";
                Group = "bouedig";
                StateDirectory = "bouedig";
                Restart = "always";
                RestartSec = "5s";
                NoNewPrivileges = "yes";
                PrivateTmp = "yes";
                ProtectSystem = "strict";
                ReadWritePaths = [
                  cfg.dataDir
                  (lib.dirOf cfg.databasePath)
                ];
                SocketBindAllow = let
                  port = lib.last (lib.splitString ":" cfg.bindAddr);
                in [ "tcp:${port}" ];
                SocketBindDeny = "any";
              };
            };
          };
        };
    in
    {
      nixosModules.bouedig-service = serviceModule;

      # Local smoke-test VM: `nixos-rebuild build-vm --flake .#bouedig-vm`
      # then run ./result/bin/run-bouedig-vm-vm and open
      # http://localhost:10025 (forwarded from the guest).
      nixosConfigurations.bouedig-vm = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          serviceModule
          ({ lib, ... }: {
            services.bouedig = {
              enable = true;
              # 0.0.0.0 so the host reaches it through QEMU user networking.
              bindAddr = "0.0.0.0:10025";
            };
            networking.firewall.allowedTCPPorts = [ 10025 ];
            system.stateVersion = "25.05";
            virtualisation.vmVariant = {
              virtualisation.memorySize = 1024;
              virtualisation.forwardPorts = [
                { from = "host"; host.port = 10025; guest.port = 10025; }
              ];
            };
          })
        ];
      };
    }
    // flake-utils.lib.eachSystem systems (system:
      let
        pkgs = pkgsFor system;

        # Same provisioning as blaz: the SDK (incl. NDK + build-tools) comes
        # from nixpkgs' androidenv so APK builds work with no local setup.
        androidSdk = (pkgs.androidenv.composeAndroidPackages {
          platformVersions = [ "36" "35" "34" ];
          buildToolsVersions = [ "36.0.0" "35.0.0" "34.0.0" ];
          ndkVersions = [ "27.0.12077973" ];
          includeNDK = true;
          cmakeVersions = [ "3.22.1" ];
          includeCmake = true;
          includeEmulator = false;
        }).androidsdk;
        sdkRoot = "${androidSdk}/libexec/android-sdk";
        ndkRoot = "${sdkRoot}/ndk/27.0.12077973";
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            # One toolchain for every build target of the workspace:
            #  - host (Linux x86): backend server
            #  - wasm32: web client
            #  - aarch64/armv7/x86_64/i686: Android client
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "rustfmt" "clippy" ];
              targets = [
                "wasm32-unknown-unknown"
                "aarch64-linux-android"
                "armv7-linux-androideabi"
                "i686-linux-android"
                "x86_64-linux-android"
              ];
            })
            pkgs.just
            pkgs.dioxus-cli
            pkgs.cargo-binstall
            pkgs.sqlite
            pkgs.geckodriver
            pkgs.firefox
            pkgs.android-tools
            pkgs.binaryen # wasm-opt, required by `dx build --release`
            pkgs.python3 # scripts/release.py
            pkgs.gh # GitHub release publishing (`just release`)
            androidSdk # Android SDK + NDK for `dx build --platform android`
            pkgs.jdk17 # gradle, driven by the Android build
          ];

          # Let geckodriver locate the Nix firefox and write to its profile dir.
          env = {
            ANDROID_SDK_ROOT = sdkRoot;
            ANDROID_HOME = sdkRoot;
            ANDROID_NDK_HOME = ndkRoot;
            ANDROID_NDK_ROOT = ndkRoot;
            JAVA_HOME = "${pkgs.jdk17}/lib/openjdk";
          };

          shellHook = ''
            export MOZ_HEADLESS=1
            export DIOXUS_ACTIVE_OVERLAY="nix"
          '';
        };

        packages = {
          default = self.packages.${system}.bouedig;
          bouedig = mkBouedig pkgs;
          web = mkWeb pkgs;
          release-tarball = mkReleaseTarball pkgs (mkBouedig pkgs);
        } // (pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          prebuilt = mkPrebuilt pkgs;
        });
      });
}
