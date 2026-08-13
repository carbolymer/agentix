{inputs, ...}: {
  perSystem = {system, ...}: {
    # Allow unfree packages (required for CUDA toolkit used by ollama-cuda)
    _module.args.pkgs = import inputs.nixpkgs {
      inherit system;
      config.allowUnfree = true;
      # Default targets for local nix build: RTX 3090/Ampere (8.6) + RTX 5090/Blackwell (12.0).
      # Override in your NixOS config via nixpkgs.config.cudaCapabilities to match your GPU.
      # devShell cargo builds auto-detect via cmake so this only affects Nix builds.
      config.cudaCapabilities = ["8.6" "12.0"];
    };
  };
}
