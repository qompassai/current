# /qompassai/current/images/default.nix
# ------------------------------------------------
# Copyright (C) 2025 Qompass AI, All rights reserved

{ pkgs ? import <nixpkgs> {} }:

import ./container.nix {
  inherit pkgs;
  runnerVersion = "2.316.0";
  dockerVersion = "28.0.1";
  buildxVersion = "0.21.2";
  runnerHooksVersion = "0.6.1";
}
