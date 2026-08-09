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

    # Minimal source for buildDepsOnly: only Cargo.toml + Cargo.lock + stubs.
    # This keeps cargoArtifacts stable across source-only changes so that
    # editing jail or ingest code doesn't force mcp-server's deps to rebuild.
    depsOnlySrc = let
      cargoFiles = lib.fileset.toSource {
        root = ./..;
        fileset = lib.fileset.unions (
          [../Cargo.toml]
          ++ lib.optional (lib.pathExists ../Cargo.lock) ../Cargo.lock
        );
      };
    in
      pkgs.runCommand "crane-deps-src" {} ''
        cp -rT ${cargoFiles} $out
        chmod -R u+w $out
        mkdir -p $out/src/jail $out/src/ingest $out/src/gh_proxy
        cp ${stubLib}  $out/src/lib.rs
        cp ${stubMain} $out/src/main.rs
        cp ${stubMain} $out/src/jail/main.rs
        cp ${stubMain} $out/src/ingest/main.rs
        cp ${stubMain} $out/src/gh_proxy/client.rs
        cp ${stubMain} $out/src/gh_proxy/server.rs
      '';

    commonArgs = {
      src = depsOnlySrc;
      strictDeps = true;
      nativeBuildInputs = [pkgs.pkg-config pkgs.autoPatchelfHook];
      buildInputs = [pkgs.onnxruntime pkgs.openssl];
      ORT_DYLIB_PATH = "${pkgs.onnxruntime}/lib/libonnxruntime.so";
    };

    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

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
            [../Cargo.toml keepFileset]
            ++ lib.optional (lib.pathExists ../Cargo.lock) ../Cargo.lock
          );
        };
      in
        pkgs.runCommand "crane-bin-src" {} ''
          cp -rT ${base} $out
          chmod -R u+w $out
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
            "src/ingest/main.rs"
            "src/jail/main.rs"
            "src/gh_proxy/client.rs"
          ];
          libStubs = ["src/lib.rs"];
        };
        inherit cargoArtifacts;
        cargoExtraArgs = "--bin gh-jail-server";
      });

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
        ingestPkg
        mcpServerPkg
      ];
    };
  in {
    packages.mcp-server = mcpServerPkg;

    packages.ingest = ingestPkg;

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
  };
}
