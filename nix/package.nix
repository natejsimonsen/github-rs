# The app as a Nix package. `nix build` puts the binary in result/bin.
{
  lib,
  stdenv,
  rustPlatform,
  libxkbcommon,
  wayland,
  libGL,
  vulkan-loader,
  xorg,
}:
let
  manifest = (lib.importTOML ../Cargo.toml).package;
  # The window system and GPU libraries are opened at run time, so they go
  # on the binary's search path rather than being linked in.
  runtimeLibs = [
    libxkbcommon
    wayland
    libGL
    vulkan-loader
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libxcb
  ];
in
rustPlatform.buildRustPackage {
  pname = manifest.name;
  version = manifest.version;

  # Only what the build reads, so a README edit doesn't rebuild everything.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../examples
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  postFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    patchelf --add-rpath ${lib.makeLibraryPath runtimeLibs} $out/bin/github-prs
  '';

  meta = {
    description = manifest.description;
    homepage = "https://github.com/natejsimonsen/github-rs";
    license = lib.licenses.mit;
    mainProgram = "github-prs";
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
