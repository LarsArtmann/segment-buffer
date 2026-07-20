{
  description = "Durable bounded queue with zstd+CBOR segment files, ack-based deletion, and crash recovery";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    systems.url = "github:nix-systems/default";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
    # rust-overlay gives us pinned MSRV (1.85) and nightly toolchains
    # alongside whatever nixpkgs' default stable happens to be.
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = import inputs.systems;
      imports = [ inputs.treefmt-nix.flakeModule ];

      perSystem =
        {
          config,
          pkgs,
          lib,
          ...
        }:
        let
          # Extend pkgs with rust-bin from rust-overlay so we can pull pinned
          # MSRV and nightly toolchains without changing default behavior.
          # `inputs` is available via closure (flake-parts injects it).
          pkgsRust = pkgs.extend (import inputs.rust-overlay);

          craneLib = inputs.crane.mkLib pkgs;

          # MSRV-pinned crane: proves `packages.default` builds on the crate's
          # declared floor (1.86), not just on whatever nixpkgs stable ships.
          # The checks (test/clippy/doc) keep using `craneLib` (stable) for
          # broader coverage; only the shipped package is MSRV-locked.
          craneLibMsrv = (inputs.crane.mkLib pkgsRust).overrideToolchain (
            p: p.rust-bin.stable."1.86.0".minimal
          );

          # Source filtered to Cargo-relevant files for build caching.
          src = craneLib.cleanCargoSource ./.;

          # zstd-sys compiles bundled libzstd from C (needs a C compiler, which
          # stdenv provides in builds); system zstd + pkg-config cover the
          # optional system-lib path used by some zstd-sys configurations.
          commonArgs = {
            inherit src;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.zstd ];
            strictDeps = true;
            # cleanCargoSource strips README.md, but lib.rs uses
            # include_str!("../README.md") to embed it in rustdoc.
            # Copy it back in after unpacking.
            postUnpack = ''
              cp ${./README.md} "$sourceRoot/README.md"
            '';
          };

          # Build dependencies once (with the encryption feature so aes-gcm and
          # rand are vendored), then reuse the artifacts across every check.
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              cargoExtraArgs = "--features encryption";
            }
          );

          # Dependency artifacts built under the MSRV toolchain, reused by
          # `packages.default` so it does not rebuild deps from scratch.
          cargoArtifactsMsrv = craneLibMsrv.buildDepsOnly (
            commonArgs
            // {
              cargoExtraArgs = "--features encryption";
            }
          );
        in
        {
          devShells = {
            # Reproducible dev shell. mkShell (not mkShellNoCC) because zstd-sys
            # compiles bundled C and needs a C compiler from stdenv.
            default = pkgs.mkShell {
              packages = with pkgs; [
                rustc
                cargo
                rustfmt
                clippy
                rust-analyzer
                zstd
                pkg-config
              ];
            };

            # Minimal CI shell: just the toolchain.
            ci = pkgs.mkShell {
              packages = with pkgs; [
                rustc
                cargo
                rustfmt
                clippy
              ];
            };

            # MSRV verification shell — pinned Rust 1.86 (the floor declared in
            # Cargo.toml's `rust-version`). Use this to validate that the crate
            # actually compiles on its declared MSRV:
            #
            #   nix develop .#msrv -c cargo check --all-targets --features encryption
            #
            # (Note: `cargo +1.86.0` syntax is rustup-only and does NOT work
            # inside a Nix shell — use the shell's cargo directly.)
            msrv = pkgs.mkShell {
              packages = [
                pkgsRust.rust-bin.stable."1.86.0".default
                pkgs.pkg-config
                pkgs.zstd
              ];
            };

            # Fuzz shell — nightly Rust for libfuzzer-sys / `cargo +nightly fuzz`.
            # Use this to actually run the fuzz targets:
            #
            #   nix develop .#fuzz -c cargo fuzz run fuzz_corrupted_read -- -max_total_time=60
            fuzz = pkgs.mkShell {
              packages = [
                (pkgsRust.rust-bin.nightly.latest.minimal.override {
                  extensions = [
                    "rust-src"
                    "rustc-codegen-cranelift"
                    "rustc-dev"
                  ];
                })
                pkgs.pkg-config
                pkgs.zstd
              ];
            };
          };

          packages.default = craneLibMsrv.buildPackage (
            commonArgs
            // {
              cargoArtifacts = cargoArtifactsMsrv;
              cargoExtraArgs = "--features encryption";
              doCheck = false;
              meta = with lib; {
                description = "Durable bounded queue with zstd+CBOR segment files";
                license = licenses.asl20;
              };
            }
          );

          checks = {
            build = config.packages.default;
            test = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoExtraArgs = "--features encryption";
                cargoTestExtraArgs = "--no-fail-fast";
              }
            );
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoExtraArgs = "--all-targets --features encryption";
                cargoClippyExtraArgs = "-- -D warnings";
              }
            );
            fmt = craneLib.cargoFmt { src = ./.; };
            doc = craneLib.cargoDoc (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoExtraArgs = "--no-deps --features encryption";
              }
            );
          };

          treefmt = {
            projectRootFile = "Cargo.toml";
            programs = {
              nixfmt.enable = true;
              rustfmt = {
                enable = true;
                # Match the crate's edition so `nix fmt` agrees with `cargo fmt`.
                edition = "2021";
              };
            };
          };

          # Reproducible fuzz runners. Each app runs one cargo-fuzz target
          # under the pinned nightly toolchain for `seconds` (default 60).
          # Override via:
          #   nix run .#fuzz-corrupted-read -- 300
          #   nix run .#fuzz-recovery -- --max-len=4096
          apps =
            let
              mkFuzzApp = target: {
                type = "app";
                program = pkgs.writeShellScriptBin "fuzz-${target}" ''
                  set -euo pipefail
                  export PATH="${
                    pkgs.lib.makeBinPath [
                      (pkgsRust.rust-bin.nightly.latest.minimal.override {
                        extensions = [ "rust-src" ];
                      })
                      pkgs.cargo
                    ]
                  }:$PATH"
                  export LD_LIBRARY_PATH="${
                    pkgs.lib.makeLibraryPath [ pkgs.zstd ]
                  }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
                  # cargo-fuzz needs to be on PATH; expect the caller to have
                  # installed it once via `cargo install cargo-fuzz`.
                  export PATH="$HOME/.cargo/bin:$PATH"
                  cd "$PWD"
                  exec cargo-fuzz run ${target} -- -max_total_time="''${1:-60}" "''${@:2}"
                '';
              };
            in
            {
              fuzz-corrupted-read = mkFuzzApp "fuzz_corrupted_read";
              fuzz-recovery = mkFuzzApp "fuzz_recovery";
            };
        };
    };
}
