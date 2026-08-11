{inputs, ...}: {
  perSystem = {
    inputs',
    lib,
    pkgs,
    ...
  }: let
    toolchain = inputs'.fenix.packages.stable.toolchain;
    craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

    # Stub source files used to satisfy cargo's path requirements without
    # including real source content in derivations that don't need it.
    stubMain = pkgs.writeText "stub-main.rs" "fn main() {}";
    stubLib = pkgs.writeText "stub-lib.rs" "";

    # Minimal source for buildDepsOnly: Cargo manifests for all workspace members
    # plus stubs at every [[bin]]/[lib] entry point so cargo can resolve and
    # compile all dependencies without touching real source.
    # This keeps cargoArtifacts stable across source-only changes.
    depsOnlySrc = let
      cargoFiles = lib.fileset.toSource {
        root = ./..;
        fileset = lib.fileset.unions (
          [
            ../Cargo.toml
            # Workspace member manifests — cargo must be able to read the full
            # workspace even when building a single package.
            ../agentix-api/Cargo.toml
            ../agentix-router/Cargo.toml
            ../agentix-daemon/Cargo.toml
            ../agentix-harness/Cargo.toml
            ../agentix-ax/Cargo.toml
            ../agentix-infer/Cargo.toml
          ]
          ++ lib.optional (lib.pathExists ../Cargo.lock) ../Cargo.lock
        );
      };
    in
      pkgs.runCommand "crane-deps-src" {} ''
        cp -rT ${cargoFiles} $out
        chmod -R u+w $out
        # Root crate stubs
        mkdir -p $out/src/jail $out/src/ax_jail $out/src/ingest $out/src/gh_proxy
        cp ${stubLib}  $out/src/lib.rs
        cp ${stubMain} $out/src/main.rs
        cp ${stubMain} $out/src/jail/main.rs
        cp ${stubMain} $out/src/ax_jail/main.rs
        cp ${stubMain} $out/src/ingest/main.rs
        cp ${stubMain} $out/src/gh_proxy/client.rs
        cp ${stubMain} $out/src/gh_proxy/server.rs
        # Workspace member stubs
        mkdir -p $out/agentix-api/src
        cp ${stubLib} $out/agentix-api/src/lib.rs
        mkdir -p $out/agentix-router/src
        cp ${stubLib} $out/agentix-router/src/lib.rs
        mkdir -p $out/agentix-daemon/src
        cp ${stubMain} $out/agentix-daemon/src/main.rs
        mkdir -p $out/agentix-harness/src
        cp ${stubLib} $out/agentix-harness/src/lib.rs
        mkdir -p $out/agentix-ax/src
        cp ${stubMain} $out/agentix-ax/src/main.rs
        mkdir -p $out/agentix-infer/src
        cp ${stubLib} $out/agentix-infer/src/lib.rs
      '';

    # agentix-infer depends on llama-cpp-2 which drives a C++ build via cmake.
    # All packages share these build tools so that a single cargoArtifacts
    # covers the full workspace dependency graph.
    commonArgs = {
      src = depsOnlySrc;
      strictDeps = true;
      nativeBuildInputs = [pkgs.pkg-config pkgs.autoPatchelfHook pkgs.clang pkgs.cmake pkgs.ninja];
      buildInputs = [pkgs.onnxruntime pkgs.openssl pkgs.libclang.lib pkgs.stdenv.cc.cc.lib];
      ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime.so";
      LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
      CMAKE_GENERATOR = "Ninja";
      CMAKE_MAKE_PROGRAM = "${pkgs.ninja}/bin/ninja";
      doCheck = false;
    };

    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

    # CUDA packages for agentix-daemon llama-cpp build
    cudaPackages = pkgs.cudaPackages_12;
    libcublasStatic = pkgs.lib.getOutput "static" cudaPackages.libcublas;

    cudaArgs = {
      # cuda_nvcc provides the nvcc binary and a setup hook that adds CUDA
      # component include paths to NVCC_PREPEND_FLAGS, making cuda_runtime.h
      # findable in the Nix sandbox (cudatoolkit's nvcc wrapper does not do this).
      # cuda_cudart is needed both for the cuda_runtime.h header at build time
      # and for libcudart at link/runtime.
      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [
        cudaPackages.cuda_nvcc
        cudaPackages.cuda_cudart
      ];
      buildInputs = commonArgs.buildInputs ++ [
        cudaPackages.cuda_cudart
        cudaPackages.libcublas
        libcublasStatic
      ];
      CMAKE_CUDA_COMPILER = "${cudaPackages.cuda_nvcc}/bin/nvcc";
      # sm_120 = RTX 5090 Laptop (Blackwell GB20x). Without this cmake
      # tries to auto-detect via a JIT compilation that fails in the sandbox.
      CMAKE_CUDA_ARCHITECTURES = "120";
      CUDA_HOME = "${cudaPackages.cudatoolkit}";
      CUDA_PATH = "${cudaPackages.cudatoolkit}";
      CUDA_TOOLKIT_ROOT_DIR = "${cudaPackages.cudatoolkit}";
      RUSTFLAGS = "-L ${cudaPackages.cudatoolkit}/lib -L ${cudaPackages.cudatoolkit}/lib/stubs -L ${cudaPackages.cuda_cudart}/lib -L ${libcublasStatic}/lib";
    };

    # Separate cargoArtifacts for the CUDA-enabled agentix-daemon build.
    # Feature flags change how llama-cpp-sys-2 is compiled so CUDA and
    # non-CUDA artifacts cannot be shared.
    cudaCargoArtifacts = craneLib.buildDepsOnly (commonArgs // cudaArgs);

    # Full source tree for agentix-* workspace packages.
    agentixSrc = lib.fileset.toSource {
      root = ./..;
      fileset = lib.fileset.unions (
        [
          ../Cargo.toml
          ../src
          ../agentix-api
          ../agentix-router
          ../agentix-daemon
          ../agentix-harness
          ../agentix-ax
          ../agentix-infer
        ]
        ++ lib.optional (lib.pathExists ../Cargo.lock) ../Cargo.lock
      );
    };

    agentixDaemonPkg = craneLib.buildPackage (commonArgs
      // cudaArgs
      // {
        src = agentixSrc;
        cargoArtifacts = cudaCargoArtifacts;
        cargoExtraArgs = "--package agentix-daemon --features cuda";
        # libcuda.so.1 is the NVIDIA driver API — present on the host at
        # runtime but never in the Nix store at build time.
        autoPatchelfIgnoreMissingDeps = ["libcuda.so.1"];
      });

    axPkg = craneLib.buildPackage (commonArgs
      // {
        src = agentixSrc;
        inherit cargoArtifacts;
        cargoExtraArgs = "--package agentix-ax";
      });

    # Build a per-binary source tree: real files from keepFileset, plus
    # stubs at the listed paths so cargo finds all [[bin]]/[lib] entries.
    # binStubs get "fn main() {}", libStubs get an empty file.
    mkBinSrc = keepFileset: {
      binStubs ? [],
      libStubs ? [],
    }:
      let
        base = lib.fileset.toSource {
          root = ./..;
          fileset = lib.fileset.unions (
            [
              ../Cargo.toml
              # Workspace member manifests must be present so cargo can
              # resolve the full workspace even when building a single bin.
              ../agentix-api/Cargo.toml
              ../agentix-router/Cargo.toml
              ../agentix-daemon/Cargo.toml
              ../agentix-harness/Cargo.toml
              ../agentix-ax/Cargo.toml
              ../agentix-infer/Cargo.toml
              keepFileset
            ]
            ++ lib.optional (lib.pathExists ../Cargo.lock) ../Cargo.lock
          );
        };
      in
        pkgs.runCommand "crane-bin-src" {} ''
          cp -rT ${base} $out
          chmod -R u+w $out
          # Workspace member stubs (cargo needs entry points even for
          # members not being compiled)
          mkdir -p $out/agentix-api/src
          cp ${stubLib} $out/agentix-api/src/lib.rs
          mkdir -p $out/agentix-router/src
          cp ${stubLib} $out/agentix-router/src/lib.rs
          mkdir -p $out/agentix-daemon/src
          cp ${stubMain} $out/agentix-daemon/src/main.rs
          mkdir -p $out/agentix-harness/src
          cp ${stubLib} $out/agentix-harness/src/lib.rs
          mkdir -p $out/agentix-ax/src
          cp ${stubMain} $out/agentix-ax/src/main.rs
          mkdir -p $out/agentix-infer/src
          cp ${stubLib} $out/agentix-infer/src/lib.rs
          ${lib.concatMapStringsSep "\n" (p: ''
            mkdir -p "$out/$(dirname "${p}")"
            cp ${stubMain} "$out/${p}"
          '')
          binStubs}
          ${lib.concatMapStringsSep "\n" (p: ''
            mkdir -p "$out/$(dirname "${p}")"
            cp ${stubLib} "$out/${p}"
          '')
          libStubs}
        '';

    # Library source shared by mcp-server and ingest.
    # src/lib.rs re-exports src/ingest/ submodules so they must be included.
    libFileset = lib.fileset.unions [
      ../src/lib.rs
      ../src/db.rs
      ../src/embed.rs
      ../src/fmt.rs
      ../src/rerank.rs
      ../src/tools.rs
      ../src/ingest
    ];

    mcpServerPkg = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc (lib.fileset.union libFileset ../src/main.rs) {
          binStubs = [
            "src/jail/main.rs"
            "src/ax_jail/main.rs"
            "src/ingest/main.rs"
            "src/gh_proxy/client.rs"
            "src/gh_proxy/server.rs"
          ];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin mcp-server";
      });

    ingestPkg = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc (lib.fileset.union libFileset ../src/ingest) {
          binStubs = [
            "src/main.rs"
            "src/jail/main.rs"
            "src/ax_jail/main.rs"
            "src/gh_proxy/client.rs"
            "src/gh_proxy/server.rs"
          ];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin ingest";
      });

    claudeJailUnwrapped = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc ../src/jail/main.rs {
          binStubs = [
            "src/main.rs"
            "src/ax_jail/main.rs"
            "src/ingest/main.rs"
            "src/gh_proxy/client.rs"
            "src/gh_proxy/server.rs"
          ];
          libStubs = ["src/lib.rs"];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin claude-jail";
      });

    ghJailClientPkg = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc ../src/gh_proxy/client.rs {
          binStubs = [
            "src/main.rs"
            "src/ax_jail/main.rs"
            "src/ingest/main.rs"
            "src/jail/main.rs"
            "src/gh_proxy/server.rs"
          ];
          libStubs = ["src/lib.rs"];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin gh-jail-client";
        # Deploy as 'gh' so it shadows the real gh inside the jail.
        postInstall = ''
          mv $out/bin/gh-jail-client $out/bin/gh
        '';
      });

    ghJailServerPkg = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc ../src/gh_proxy/server.rs {
          binStubs = [
            "src/main.rs"
            "src/ax_jail/main.rs"
            "src/ingest/main.rs"
            "src/jail/main.rs"
            "src/gh_proxy/client.rs"
          ];
          libStubs = ["src/lib.rs"];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin gh-jail-server";
      });

    axJailUnwrapped = craneLib.buildPackage (commonArgs
      // {
        src = mkBinSrc ../src/ax_jail/main.rs {
          binStubs = [
            "src/main.rs"
            "src/jail/main.rs"
            "src/ingest/main.rs"
            "src/gh_proxy/client.rs"
            "src/gh_proxy/server.rs"
          ];
          libStubs = ["src/lib.rs"];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin ax-jail";
      });

    axJailSanityCheck = pkgs.writeShellScriptBin "ax-jail-check" ''
      #!/usr/bin/env bash
      set -euo pipefail

      PASS=0
      FAIL=0

      check() {
        local label="$1"
        local result="$2"
        local ok="$3"
        if [ "$ok" = "1" ]; then
          echo "[OK]   $label: $result"
          PASS=$((PASS + 1))
        else
          echo "[FAIL] $label: $result"
          FAIL=$((FAIL + 1))
        fi
      }

      WT=$(git rev-parse --show-toplevel 2>&1) && check "git rev-parse --show-toplevel" "$WT" 1 || check "git rev-parse --show-toplevel" "$WT" 0
      STATUS=$(git status 2>&1) && check "git status" "ok" 1 || check "git status" "$STATUS" 0
      LOG=$(git log -1 --oneline 2>&1) && check "git log -1" "$LOG" 1 || check "git log -1" "$LOG" 0

      COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "")
      if [ -z "$COMMON" ]; then
        check "hooks mask" "cannot determine git-common-dir" 0
      else
        HOOKS_CONTENT=$(ls "$COMMON/hooks" 2>&1)
        if [ -z "$HOOKS_CONTENT" ]; then
          check "hooks mask" "empty" 1
        else
          check "hooks mask" "NOT empty: $HOOKS_CONTENT" 0
        fi
      fi

      if [ -z "$COMMON" ]; then
        check "config mask" "cannot determine git-common-dir" 0
      else
        CONFIG_OUT=$(git config --file "$COMMON/config" core.fsmonitor testvalue 2>&1 || true)
        if echo "$CONFIG_OUT" | grep -qi 'read.only\|permission\|cannot open'; then
          check "config mask" "write correctly denied" 1
        else
          check "config mask" "write was NOT denied (config not masked!)" 0
        fi
      fi

      KEYS=$(env | grep -E 'ANTHROPIC_API_KEY|OPENAI_API_KEY|OPENROUTER_API_KEY' || true)
      if [ -z "$KEYS" ]; then
        check "no API keys in environment" "clean" 1
      else
        check "no API keys in environment" "FOUND: $KEYS" 0
      fi

      echo ""
      if [ "$FAIL" -eq 0 ]; then
        echo "All $PASS check(s) passed."
        exit 0
      else
        echo "$FAIL check(s) FAILED."
        exit 1
      fi
    '';

    axJailBinDir = pkgs.buildEnv {
      name = "ax-jail-tools";
      pathsToLink = ["/bin"];
      paths = [
        pkgs.nix
        pkgs.git
        pkgs.gh
        pkgs.curl
        pkgs.bash
        pkgs.python3
        pkgs.direnv
        pkgs.coreutils
        pkgs.findutils
        pkgs.jq
        pkgs.gnused
        ingestPkg
        mcpServerPkg
        axPkg
        axJailSanityCheck
      ];
    };

    # Merged ~/bin for the jail. buildEnv with pathsToLink=["/bin"] handles
    # multi-binary packages (coreutils, findutils) correctly; all symlinks
    # point into /nix/store which is bind-mounted read-only inside the jail.
    claudeJailBinDir = pkgs.buildEnv {
      name = "claude-jail-tools";
      pathsToLink = ["/bin"];
      paths = [
        inputs'.llm-agents.packages.claude-code
        pkgs.nix
        pkgs.git
        ghJailClientPkg
        pkgs.curl
        pkgs.bash
        pkgs.python3
        pkgs.direnv
        pkgs.coreutils
        pkgs.findutils
        pkgs.jq
        pkgs.gnugrep
        pkgs.ripgrep
        pkgs.gnused
        pkgs.openssh
        ingestPkg
        mcpServerPkg
      ];
    };
  in {
    packages.mcp-server = mcpServerPkg;

    packages.ingest = ingestPkg;

    # Wrapper that sets LD_LIBRARY_PATH so libcuda.so.1 (NVIDIA driver API,
    # not in the Nix store) is found at runtime. Also fixes the nix run binary
    # name: crane names derivations after the root crate (mcp-server-0.1.0),
    # so nix run .#agentix-daemon would try to exec 'mcp-server' without this.
    packages.agentix-daemon = pkgs.writeShellScriptBin "agentix-daemon" ''
      export LD_LIBRARY_PATH="/run/opengl-driver/lib:${cudaPackages.cuda_cudart}/lib:${cudaPackages.libcublas}/lib:''${LD_LIBRARY_PATH:-}"
      exec ${agentixDaemonPkg}/bin/agentix-daemon "$@"
    '';

    packages.ax = axPkg;

    # Wrapper script sets env vars the Rust binary reads, then execs it.
    packages.claude-jail = pkgs.writeShellScriptBin "claude-jail" ''
      # buildEnv puts merged symlinks under bin/
      export CLAUDE_JAIL_BIN_DIR="${claudeJailBinDir}/bin"
      export CLAUDE_JAIL_BWRAP="${pkgs.bubblewrap}/bin/bwrap"
      export CLAUDE_JAIL_GH_SERVER="${ghJailServerPkg}/bin/gh-jail-server"
      # Nix store cacert bundle — works even if /etc/ssl is absent on the host
      export NIX_SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
      # Enable nix-command and flakes inside the jail
      export NIX_CONFIG="extra-experimental-features = nix-command flakes"
      exec ${claudeJailUnwrapped}/bin/claude-jail "$@"
    '';

    packages.gh-jail-server = ghJailServerPkg;

    packages.ax-jail = pkgs.writeShellScriptBin "ax-jail" ''
      export AX_JAIL_BIN_DIR="${axJailBinDir}/bin"
      export AX_JAIL_BWRAP="${pkgs.bubblewrap}/bin/bwrap"
      export NIX_SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
      export NIX_CONFIG="extra-experimental-features = nix-command flakes"
      exec ${axJailUnwrapped}/bin/ax-jail "$@"
    '';
  };
}
