{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.agentix-daemon;
in {
  options.services.agentix-daemon = {
    enable = lib.mkEnableOption "agentix-daemon inference gateway";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The agentix-daemon package to use.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 11430;
      description = "Port for the OpenAI-compatible HTTP gateway.";
    };

    modelsDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/agentix/models";
      description = "Directory where pulled GGUF blobs and manifests are stored.";
    };

    maxCtx = lib.mkOption {
      type = lib.types.ints.positive;
      default = 32768;
      example = 65536;
      description = ''
        Maximum context window (tokens) allocated per model.
        Larger values consume more VRAM; set based on your GPU.
        Rough guidance: 3090 (24 GB) → 32768–65536 for 7–32B Q4_K_M models.
      '';
    };

    gpuLayers = lib.mkOption {
      type = lib.types.int;
      default = -1;
      description = ''
        Number of model layers to offload to GPU. -1 offloads all layers
        (requires the cuda-enabled build). Set to 0 for CPU-only inference.
      '';
    };

    maxLoadedModels = lib.mkOption {
      type = lib.types.ints.positive;
      default = 2;
      description = "Maximum number of models kept resident in memory simultaneously.";
    };

    vramLimitBytes = lib.mkOption {
      type = lib.types.nullOr lib.types.ints.positive;
      default = null;
      example = 23000000000;
      description = "Hard VRAM cap in bytes. Null means no explicit limit.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to a file containing secret environment variables, one per line:
          ANTHROPIC_API_KEY=sk-ant-...
          OPENAI_API_KEY=sk-...
          OPENROUTER_API_KEY=sk-or-...
        Compatible with sops-nix and agenix EnvironmentFile patterns.
      '';
    };

    ollamaBaseUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://localhost:11434";
      description = "Base URL of an Ollama instance used for Ollama-compat proxy endpoints.";
    };

    extraEnv = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = "Additional environment variables passed verbatim to the daemon.";
    };
  };

  config = lib.mkIf cfg.enable {
    warnings = lib.optional (pkgs.config.cudaCapabilities or [] == []) ''
      agentix-daemon: nixpkgs.config.cudaCapabilities is not set.
      The daemon package was built without GPU-specific CUDA kernels and will
      fall back to CPU-only inference.  Set this in your NixOS configuration:

        nixpkgs.config.cudaCapabilities = [ "8.6" ];  # RTX 3090
        nixpkgs.config.cudaCapabilities = [ "8.9" ];  # RTX 4090
        nixpkgs.config.cudaCapabilities = [ "12.0" ]; # RTX 5090
    '';

    users.users.agentix = {
      isSystemUser = true;
      group = "agentix";
      home = "/var/lib/agentix";
      description = "agentix-daemon service user";
      # GPU access: render covers both NVIDIA (via nvidia-uvm) and AMD/Intel.
      # video is needed for some driver paths that check group membership.
      extraGroups = ["video" "render"];
    };

    users.groups.agentix = {};

    systemd.services.agentix-daemon = {
      description = "agentix inference gateway";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];

      environment =
        {
          AGENTIX_GATEWAY_PORT = toString cfg.port;
          AGENTIX_MODELS_DIR = cfg.modelsDir;
          AGENTIX_MAX_CTX = toString cfg.maxCtx;
          # gpuLayers -1 means "all layers" — u32::MAX in the daemon, but the
          # env var is parsed as u32 so we map -1 → pass nothing (daemon
          # defaults to u32::MAX when CUDA feature is enabled).
          AGENTIX_MAX_LOADED_MODELS = toString cfg.maxLoadedModels;
          OLLAMA_BASE_URL = cfg.ollamaBaseUrl;
        }
        // lib.optionalAttrs (cfg.gpuLayers >= 0) {
          AGENTIX_GPU_LAYERS = toString cfg.gpuLayers;
        }
        // lib.optionalAttrs (cfg.vramLimitBytes != null) {
          AGENTIX_VRAM_LIMIT_BYTES = toString cfg.vramLimitBytes;
        }
        // cfg.extraEnv;

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/agentix-daemon";
        Restart = "on-failure";
        RestartSec = "5s";

        User = "agentix";
        Group = "agentix";

        # agentix manages its own sub-directory layout under StateDirectory.
        StateDirectory = "agentix";
        StateDirectoryMode = "0750";

        # GPU passthrough: PrivateDevices = false leaves the host /dev intact.
        # Do NOT add DeviceAllow — any DeviceAllow entry causes systemd to
        # install a cgroup device BPF filter blocking everything not listed,
        # which breaks CUDA even when the device nodes are world-readable.
        PrivateDevices = false;

        # Load secret env vars (API keys) from a file outside the Nix store.
        EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;
      };
    };

    # Ensure modelsDir exists with correct ownership if it differs from StateDirectory.
    systemd.tmpfiles.rules = lib.optionals (toString cfg.modelsDir != "/var/lib/agentix/models") [
      "d ${cfg.modelsDir} 0750 agentix agentix -"
    ];
  };
}
