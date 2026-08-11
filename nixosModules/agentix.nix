{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.agentix;
in {
  options.services.agentix = {
    enable = lib.mkEnableOption "agentix-daemon AI orchestrator";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The agentix-daemon package to use.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/agentix";
      description = "State directory for agentix (sled, vector index, tantivy).";
    };

    modelWeightsDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/agentix/models";
      description = "Directory containing GGUF model weight files.";
    };

    gatewayPort = lib.mkOption {
      type = lib.types.port;
      default = 11434;
      description = "Port for the OpenAI-compatible HTTP gateway.";
    };

    localModel = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "qwen2.5-coder-32b-instruct-q4_k_m.gguf";
      description = "Filename of the GGUF model to load for local inference. Null disables local inference.";
    };

    defaultRoute = lib.mkOption {
      type = lib.types.enum ["local" "anthropic" "openai"];
      default = "anthropic";
      description = "Default backend for unrecognised model names.";
    };

    cudaDevice = lib.mkOption {
      type = lib.types.int;
      default = 0;
      description = "CUDA device index for local inference.";
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Path to a file containing environment variables (ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.).";
    };

    extraEnv = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = "Extra environment variables passed to the daemon.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.agentix = {
      isSystemUser = true;
      group = "agentix";
      home = cfg.dataDir;
      description = "agentix-daemon service user";
      extraGroups = ["video"]; # GPU device access
    };

    users.groups.agentix = {};

    systemd.services.agentix-daemon = {
      description = "agentix-daemon AI orchestrator gateway";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];

      environment =
        {
          AGENTIX_GATEWAY_PORT = toString cfg.gatewayPort;
          AGENTIX_DATA_DIR = cfg.dataDir;
          AGENTIX_MODEL_WEIGHTS_DIR = cfg.modelWeightsDir;
          AGENTIX_DEFAULT_ROUTE = cfg.defaultRoute;
          CUDA_VISIBLE_DEVICES = toString cfg.cudaDevice;
        }
        // lib.optionalAttrs (cfg.localModel != null) {
          AGENTIX_LOCAL_MODEL = cfg.localModel;
        }
        // cfg.extraEnv;

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/agentix-daemon";
        Restart = "on-failure";
        RestartSec = "5s";

        User = "agentix";
        Group = "agentix";

        StateDirectory = "agentix";
        StateDirectoryMode = "0750";

        # Allow GPU access
        PrivateDevices = false;
        DeviceAllow = [
          "char-drm rw"
          "char-nvidia-frontend rw"
          "char-nvidia-uvm rw"
        ];

        EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;
      };
    };

    # Create model weights dir if it differs from dataDir
    systemd.tmpfiles.rules = lib.optionals (cfg.modelWeightsDir != "${cfg.dataDir}/models") [
      "d ${cfg.modelWeightsDir} 0750 agentix agentix -"
    ];
  };
}
