{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, fenix, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        rust = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "rustc"
          "rust-src"
          "rust-std"
          "rustfmt"
          "clippy"
        ];
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rust
            fenix.packages.${system}.rust-analyzer
            pkgs.pkg-config
            pkgs.openssl
          ];
        };
      }
    );
}
