{inputs, ...}: {
  perSystem = {system, ...}: {
    # Allow unfree packages (required for CUDA toolkit used by ollama-cuda)
    _module.args.pkgs = import inputs.nixpkgs {
      inherit system;
      config.allowUnfree = true;
      # RTX 5090 / Blackwell (sm_120). Required for nix build .#agentix-daemon
      # to compile llama-cpp-sys-2 with correct GPU kernels.
      # devShell cargo builds auto-detect via cmake so this only affects Nix builds.
      config.cudaCapabilities = ["12.0"];
    };
  };
}
