# /qompassai/current/flake.nix
# ---------------------------------------
# Copyright (C) 2025 Qompass AI, All rights reserved

{
  description = "Qompass Custom GitHub Actions Runner (current)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };
      in {
        devShells.default = pkgs.mkShell {
          name = "qompass-current-runner";

          nativeBuildInputs = with pkgs; [
            git
            openssh
            curl
            gnupg
            bash
            coreutils
            systemd
            jq
            dotnet-sdk_8
            dotnet-runtime_8
            nodejs 
          ];

          DOTNET_ROOT = "${pkgs.dotnet-sdk_8}";

          shellHook = ''
            echo "Welcome to Qompass AI 'current' runner!"
            export PATH=$DOTNET_ROOT:$PATH
          '';
        };
      });
}

