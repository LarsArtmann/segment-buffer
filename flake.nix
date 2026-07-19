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
          craneLib = inputs.crane.mkLib pkgs;

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
          };

          # Build dependencies once (with the encryption feature so aes-gcm and
          # rand are vendored), then reuse the artifacts across every check.
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              cargoExtraArgs = "--features encryption";
            }
          );
        in
        {
          # Reproducible dev shell. mkShell (not mkShellNoCC) because zstd-sys
          # compiles bundled C and needs a C compiler from stdenv.
          devShells.default = pkgs.mkShell {
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
          devShells.ci = pkgs.mkShell {
            packages = with pkgs; [
              rustc
              cargo
              rustfmt
              clippy
            ];
          };

          packages.default = craneLib.buildPackage (
            commonArgs
            // {
              inherit cargoArtifacts;
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
        };
    };
}
