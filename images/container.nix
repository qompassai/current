# /qompassai/current/images/container.nix
# --------------------------------------------------
# Copyright (C) 2025 Qompass AI, All rights reserved

{ pkgs, runnerVersion, dockerVersion, buildxVersion, runnerHooksVersion }:

let
  runnerArch = if pkgs.stdenv.hostPlatform.system == "x86_64-linux" then "x64"
               else if pkgs.stdenv.hostPlatform.system == "aarch64-linux" then "arm64"
               else throw "Unsupported arch";

  dockerArch = if runnerArch == "x64" then "x86_64" else "aarch64";

  runnerUser = "runner";
  runnerUid = 1001;
  runnerGid = 1001;

in pkgs.dockerTools.buildImage {
  name = "qompassai/gh-runner-rootless";
  tag = runnerVersion;

  contents = [
    pkgs.git
    pkgs.gnutar
    pkgs.unzip
    pkgs.curl
    pkgs.jq
    pkgs.lsb-release
    pkgs.coreutils
    pkgs.sudo
  ];

  config = {
    User = "${runnerUid}:${runnerGid}";
    WorkingDir = "/home/runner";
    Env = [
      "DEBIAN_FRONTEND=noninteractive"
      "RUNNER_MANUALLY_TRAP_SIG=1"
      "ACTIONS_RUNNER_PRINT_LOG_TO_STDOUT=1"
      "ImageOS=ubuntu22"
    ];
    Cmd = [ "./run.sh" ];
  };

  extraCommands = ''
    mkdir -p ./home/runner

    echo "📦 Downloading GitHub Actions runner"
    curl -L -o runner.tar.gz https://github.com/actions/runner/releases/download/v${runnerVersion}/actions-runner-linux-${runnerArch}-${runnerVersion}.tar.gz
    tar -C ./home/runner -xzf runner.tar.gz
    rm runner.tar.gz

    echo "📦 Installing container hooks"
    curl -L -o hooks.zip https://github.com/actions/runner-container-hooks/releases/download/v${runnerHooksVersion}/actions-runner-hooks-k8s-${runnerHooksVersion}.zip
    unzip -d ./home/runner/k8s hooks.zip
    rm hooks.zip

    echo "📦 Installing Docker CLI and Buildx"
    curl -L -o docker.tgz https://download.docker.com/linux/static/stable/${dockerArch}/docker-${dockerVersion}.tgz
    tar -C ./home/runner -xzf docker.tgz
    rm docker.tgz

    curl -L -o ./home/runner/docker-buildx \
      https://github.com/docker/buildx/releases/download/v${buildxVersion}/buildx-v${buildxVersion}.linux-${runnerArch}
    chmod +x ./home/runner/docker-buildx
    mkdir -p ./usr/local/lib/docker/cli-plugins
    mv ./home/runner/docker-buildx ./usr/local/lib/docker/cli-plugins/docker-buildx
  '';

  extraDirectories = {
    "etc".text = ''
      root:x:0:0:root:/root:/bin/sh
      ${runnerUser}:x:${toString runnerUid}:${toString runnerGid}:Runner:/home/runner:/bin/bash
    '';
  };
}

