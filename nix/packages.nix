# Builds the whole Cargo workspace once with crane, then exposes each tool as a thin wrapper
# that adds its runtime PATH/libraries. Every tool shares one dependency build (`cargoArtifacts`)
# and one workspace compile, so Cargo feature unification never forces per-tool rebuilds.
{
  lib,
  pkgs,
  craneLib,
}: let
  root = ../.;

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (craneLib.fileset.commonCargoSources root)
      (lib.fileset.fileFilter (file: file.hasExt "svg") ../crates)
    ];
  };

  # transcribe.cpp (tyspeak) builds ggml with no SIMD on Linux by default. Target x86-64-v3
  # (AVX2/FMA, any x86 CPU from ~2015 on) so CPU inference is fast but builds stay reproducible.
  transcribeCmakeArgs =
    lib.optionalString pkgs.stdenv.hostPlatform.isx86_64
    "-DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON -DGGML_BMI2=ON";

  # Libraries every tool links or dlopens.
  guiLibs = with pkgs; [gtk3 libappindicator-gtk3];

  commonArgs = {
    inherit src;
    pname = "tys-tools";
    version = (lib.importTOML ../Cargo.toml).workspace.package.version;
    strictDeps = true;
    nativeBuildInputs = with pkgs; [cmake pkg-config];
    buildInputs =
      guiLibs
      ++ (with pkgs; [
        dbus
        libx11
        libxi
        libxtst
        xdotool
      ]);
    TRANSCRIBE_CMAKE_ARGS = transcribeCmakeArgs;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  workspace = craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;
      # Tests run in `checks` (they need no display), not on every package build.
      doCheck = false;
      meta = {
        description = "All ty* desktop tools (unwrapped binaries)";
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    });

  # Wrap `bins` from the workspace build with GTK env (icons, GSettings schemas),
  # LD_LIBRARY_PATH for the dlopen'd appindicator, and `runtimeDeps` on PATH.
  mkTool = {
    name,
    description,
    bins ? [name],
    runtimeDeps ? [],
  }:
    pkgs.stdenv.mkDerivation {
      pname = name;
      inherit (workspace) version;
      dontUnpack = true;
      strictDeps = true;
      nativeBuildInputs = with pkgs; [makeWrapper wrapGAppsHook3];
      buildInputs = guiLibs ++ [pkgs.gsettings-desktop-schemas];
      dontWrapGApps = true;
      installPhase = "mkdir -p $out/bin";
      postFixup = lib.concatMapStrings (bin: ''
        makeWrapper ${workspace}/bin/${bin} $out/bin/${bin} \
          "''${gappsWrapperArgs[@]}" \
          --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath guiLibs} \
          --suffix PATH : ${lib.makeBinPath ([pkgs.libnotify pkgs.xdg-utils] ++ runtimeDeps)}
      '')
      bins;
      passthru = {inherit workspace;};
      meta = {
        inherit description;
        mainProgram = name;
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    };
in {
  inherit workspace cargoArtifacts commonArgs craneLib src;

  tyboard = mkTool {
    name = "tyboard";
    description = "Clipboard history tray daemon";
    bins = ["tyboard" "tyboard-rofi"];
  };

  tykey = mkTool {
    name = "tykey";
    description = "Text expander tray daemon";
  };

  tycapture = mkTool {
    name = "tycapture";
    description = "X11 screen recording tray applet";
    runtimeDeps = with pkgs; [ffmpeg-full pulseaudio dunst i3];
  };

  tyshot = mkTool {
    name = "tyshot";
    description = "X11 screenshot and annotation tool";
  };

  tyspeak = mkTool {
    name = "tyspeak";
    description = "Speech-to-text dictation tray daemon";
    runtimeDeps = with pkgs; [pulseaudio xdotool];
  };
}
