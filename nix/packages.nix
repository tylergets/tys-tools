# Package builds use one derivation per Rust crate, shared between tools.
# Crane supplies the Cargo-based development shell and workspace-wide checks.
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
        gtk-layer-shell
        libxkbcommon
        libx11
        libxi
        libinput
        libxtst
        linux-pam
        udev
        xdotool
      ]);
    TRANSCRIBE_CMAKE_ARGS = transcribeCmakeArgs;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  # Proc macros read Cargo.toml even though rustc builds each crate directly.
  # Resolve workspace inheritance so those manifests work in isolated sources.
  workspaceManifest = (lib.importTOML ../Cargo.toml).workspace;
  resolveDependencies = lib.mapAttrs (name: dep:
    if builtins.isAttrs dep && (dep.workspace or false)
    then let
      inherited = workspaceManifest.dependencies.${name};
      base =
        if builtins.isString inherited
        then {version = inherited;}
        else inherited;
    in
      base
      // (removeAttrs dep ["workspace"])
      // lib.optionalAttrs (base ? features || dep ? features) {
        features = lib.unique ((base.features or []) ++ (dep.features or []));
      }
    else dep);
  resolveTables = lib.mapAttrs (name: value:
    if lib.elem name ["dependencies" "build-dependencies" "dev-dependencies"]
    then resolveDependencies value
    else value);
  localCrateOverride = name: _: let
    manifest = lib.importTOML (../crates + "/${name}/Cargo.toml");
    standalone =
      (resolveTables manifest)
      // {
        package = lib.mapAttrs (key: value:
          if builtins.isAttrs value && (value.workspace or false)
          then workspaceManifest.package.${key}
          else value)
        manifest.package;
      }
      // lib.optionalAttrs (manifest ? target) {
        target = lib.mapAttrs (_: resolveTables) manifest.target;
      };
  in {
    extraRustcOpts = ["-Clto=thin"];
    postPatch = ''
      cp ${(pkgs.formats.toml {}).generate "${name}-Cargo.toml" standalone} Cargo.toml
    '';
  };

  # Committed metadata avoids import-from-derivation during evaluation. Each local
  # crate's source is filtered independently by crate2nix, including its SVG assets.
  crates = import ../Cargo.nix {
    inherit pkgs;
    # Match Cargo's release codegen default. rustc retains library bitcode by
    # default; enable the workspace's thin LTO when linking our own binaries.
    buildRustCrateForPkgs = p: p.buildRustCrate.override {defaultCodegenUnits = 16;};
    defaultCrateOverrides =
      pkgs.defaultCrateOverrides
      // lib.genAttrs (builtins.attrNames (builtins.readDir ../crates)) localCrateOverride
      // {
        gtk-layer-shell-sys = _: {
          nativeBuildInputs = [pkgs.pkg-config];
          buildInputs = [pkgs.gtk-layer-shell];
        };
        input-sys = _: {
          nativeBuildInputs = [pkgs.pkg-config];
          buildInputs = [pkgs.libinput];
          extraLinkFlags = ["-L${lib.getLib pkgs.libinput}/lib"];
        };
        xkbcommon = _: {
          buildInputs = [pkgs.libxkbcommon];
          extraLinkFlags = ["-L${lib.getLib pkgs.libxkbcommon}/lib"];
        };
        transcribe-cpp-sys = attrs: {
          nativeBuildInputs =
            [pkgs.cmake]
            ++ lib.optional (lib.elem "vulkan" attrs.features) pkgs.shaderc;
          buildInputs =
            lib.optionals (lib.elem "vulkan" attrs.features)
            [pkgs.vulkan-headers pkgs.vulkan-loader];
          dontUseCmakeConfigure = true;
          TRANSCRIBE_CMAKE_ARGS = transcribeCmakeArgs;
        };
        tylock = attrs:
          (localCrateOverride "tylock" attrs)
          // {
            buildInputs = [pkgs.linux-pam];
            extraLinkFlags = ["-L${lib.getLib pkgs.linux-pam}/lib"];
          };
      };
  };

  buildTool = name: crates.workspaceMembers.${name}.build;

  workspace = pkgs.symlinkJoin {
    name = "tys-tools-${commonArgs.version}";
    inherit (commonArgs) version;
    paths =
      lib.mapAttrsToList (name: _: buildTool name)
      (removeAttrs crates.workspaceMembers ["ty-common"]);
    meta = {
      description = "All ty* desktop tools (unwrapped binaries)";
      license = lib.licenses.mit;
      platforms = lib.platforms.linux;
    };
  };

  # Programs the shared display layer (ty-common) runs on Wayland; tools add their own on top.
  waylandDeps = with pkgs; [wl-clipboard];

  # A launcher entry (for rofi/dmenu/app menus) with the tool's icon. `exec` is run as-is.
  desktopItem = tool: {
    name ? tool,
    desktopName,
    exec,
    comment,
  }:
    pkgs.makeDesktopItem {
      inherit name desktopName exec comment;
      icon = tool;
      categories = ["Utility"];
      terminal = false;
    };

  # Wrap `bins` from the selected tool's build with GTK env (icons, GSettings schemas),
  # LD_LIBRARY_PATH for the dlopen'd appindicator, and `runtimeDeps` on PATH.
  # Installs one desktop entry running the tool's default command, plus `desktopEntries`.
  mkTool = {
    name,
    description,
    bins ? [name],
    runtimeDeps ? [],
    desktopEntries ? [],
    # Extra install commands (files under $out).
    extraInstall ? "",
  }: let
    unwrapped = buildTool name;
    items = map (desktopItem name) ([
        {
          desktopName = name;
          exec = name;
          comment = description;
        }
      ]
      ++ desktopEntries);
  in
    pkgs.stdenv.mkDerivation {
      pname = name;
      inherit (unwrapped) version;
      dontUnpack = true;
      strictDeps = true;
      nativeBuildInputs = with pkgs; [makeWrapper wrapGAppsHook3];
      buildInputs = guiLibs ++ [pkgs.gsettings-desktop-schemas];
      dontWrapGApps = true;
      installPhase = ''
        mkdir -p $out/bin $out/share/applications
        install -Dm644 ${../crates/ty-common/icons + "/${name}.svg"} \
          $out/share/icons/hicolor/scalable/apps/${name}.svg
        ${lib.concatMapStrings (item: "ln -s ${item}/share/applications/* $out/share/applications/\n") items}
        ${extraInstall}
      '';
      postFixup =
        lib.concatMapStrings (bin: ''
          makeWrapper ${unwrapped}/bin/${bin} $out/bin/${bin} \
            "''${gappsWrapperArgs[@]}" \
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath guiLibs} \
            --suffix PATH : ${lib.makeBinPath ([pkgs.libnotify pkgs.xdg-utils] ++ runtimeDeps)}
        '')
        bins;
      passthru = {inherit unwrapped;};
      meta = {
        inherit description;
        mainProgram = name;
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    };
in {
  inherit workspace cargoArtifacts commonArgs craneLib src;
  tyquote = (buildTool "tyquote").overrideAttrs (old: {
    meta =
      (old.meta or {})
      // {
        description = "Personal quotes in Syncthing-friendly files";
        mainProgram = "tyquote";
      };
  });

  tynotif = mkTool {
    name = "tynotif";
    description = "Desktop notifications with actions, history and do-not-disturb";
    runtimeDeps = [pkgs.systemd]; # --force can stop the owning Dunst user service.
  };

  tyboard = mkTool {
    name = "tyboard";
    description = "Clipboard history tray daemon";
    bins = ["tyboard" "tyboard-rofi"];
    runtimeDeps = waylandDeps ++ [pkgs.xdotool];
  };

  tykey = mkTool {
    name = "tykey";
    description = "Text expander tray daemon";
    runtimeDeps = [pkgs.wtype];
  };

  tycapture = mkTool {
    name = "tycapture";
    description = "Screen recording tray applet (X11 and Wayland)";
    runtimeDeps = with pkgs; [ffmpeg-full pulseaudio dunst i3 grim wf-recorder];
  };

  tyshot = mkTool {
    name = "tyshot";
    description = "Screenshot and annotation tool (X11 and Wayland)";
    runtimeDeps = waylandDeps ++ [pkgs.grim];
    desktopEntries = [
      {
        name = "tyshot-full";
        desktopName = "tyshot full";
        exec = "tyshot full";
        comment = "Copy a screenshot of the whole screen";
      }
      {
        name = "tyshot-delay";
        desktopName = "tyshot delayed";
        exec = "tyshot gui --delay 5";
        comment = "Select a screen region after 5 seconds";
      }
    ];
  };

  tyspeak = mkTool {
    name = "tyspeak";
    description = "Speech-to-text dictation tray daemon";
    runtimeDeps = waylandDeps ++ (with pkgs; [pulseaudio xdotool wtype]);
  };

  tylight = mkTool {
    name = "tylight";
    description = "Screen color temperature and monitor brightness daemon (X11 and wlroots)";
    runtimeDeps = [pkgs.ddcutil];
  };

  tygesture = mkTool {
    name = "tygesture";
    description = "Touchpad gesture daemon (libinput)";
    runtimeDeps = with pkgs; [xdotool wtype];
  };

  typort = mkTool {
    name = "typort";
    description = "Tray applet for your listening ports: open them in the browser or stop them";
  };

  tyci = mkTool {
    name = "tyci";
    description = "Open GitHub PRs and running CI in your tray (via the gh CLI)";
    runtimeDeps = [pkgs.gh];
  };

  # Authenticates through polkit's own helper (the socket-activated service or the setuid
  # binary the system provides), so nothing extra is needed at runtime.
  typk = mkTool {
    name = "typk";
    description = "Polkit authentication agent: a password or fingerprint prompt";
    # polkitd must know the `typk verify` action: on NixOS that takes the package in
    # environment.systemPackages (polkit reads /run/current-system/sw/share/polkit-1).
    extraInstall = ''
      install -Dm644 ${../crates/typk/typk.policy} $out/share/polkit-1/actions/tools.tys.typk.policy
    '';
  };

  # PAM and the session's display only; the PAM services are system configuration.
  tylock = mkTool {
    name = "tylock";
    description = "Screen locker for X11 and Wayland: wallpaper, clock, password or fingerprint";
  };
}
