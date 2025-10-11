{
  description = "A Rust-based CLI application for managing restic backups with S3-compatible storage";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage rec {
          pname = "restic-backup-service";
          version = "0.9.0";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
          ];

          buildInputs = with pkgs; [
            openssl
          ];

          # Runtime dependencies
          propagatedBuildInputs = with pkgs; [
            restic
            awscli2
          ];

          # Ensure runtime dependencies are available in PATH
          postInstall = ''
            wrapProgram $out/bin/restic-backup-service \
              --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.restic pkgs.awscli2]}
          '';

          meta = with pkgs.lib; {
            description = "A Rust-based CLI application for managing restic backups with S3-compatible storage";
            homepage = "https://github.com/timlisemer/restic-backup-service";
            license = licenses.mit;
            maintainers = [];
            platforms = platforms.linux;
          };
        };

        # Expose the package as 'restic-backup-service' as well
        packages.restic-backup-service = self.packages.${system}.default;
      }
    )
    // {
      # NixOS module (system-independent)
      nixosModules.default = import ./nixos-module.nix;
      nixosModules.restic-backup-service = self.nixosModules.default;
    };
}
