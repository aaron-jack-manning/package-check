{
  description = "Tool to check a Typst package.";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils }: utils.lib.eachDefaultSystem
    (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages = rec {
          default = typst-package-check;
          typst-package-check = pkgs.rustPlatform.buildRustPackage
            {
              pname = "typst-package-check";
              version = "0.1.0";
              src = ./.;
              nativeBuildInputs = [ pkgs.pkg-config ];
              buildInputs = [ pkgs.openssl.dev pkgs.git ];
              cargoHash = "sha256-J7M5bAc11tB6m1i/yz0M49g1oskD0HFVtg7j4B7rBjU=";
              # Don't run `cargo test`, as there are no tests to run.
              doCheck = false;
            };
          docker-image = pkgs.dockerTools.buildImage {
            name = "ghcr.io/typst/package-check";
            tag = typst-package-check.version;
            copyToRoot = with pkgs.dockerTools; [
              caCertificates
              pkgs.git
              typst-package-check
            ];
            config = {
              Entrypoint = [ "/bin/typst-package-check" ];
              WorkingDir = "/data";
            };
          };
        };
      });
}
