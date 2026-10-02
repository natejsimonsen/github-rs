{
  description = "A fast desktop app for GitHub pull requests, styled like github.com";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: rec {
        github-prs = pkgs.callPackage ./nix/package.nix { };
        default = github-prs;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/github-prs";
        };
      });

      # `nix develop`: everything needed to build and run from source.
      devShells = forAll (pkgs: {
        default = pkgs.callPackage ./nix/shell.nix { };
      });

      formatter = forAll (pkgs: pkgs.nixfmt-rfc-style);
    };
}
