{
  description = "ty* desktop tools: tyboard, tykey, tycapture, tyshot, tyspeak, tylight, tygesture, typort, tyci, typk, tylock";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = {
    self,
    nixpkgs,
    crane,
  }: let
    inherit (nixpkgs) lib;
    systems = ["x86_64-linux" "aarch64-linux"];
    forAllSystems = f: lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

    tools = ["tyboard" "tykey" "tycapture" "tyshot" "tyspeak" "tylight" "tygesture" "typort" "tyci" "typk" "tylock" "tynotif"];

    # Called directly on legacyPackages (no nixpkgs re-instantiation) for fast evaluation.
    mkPackages = pkgs:
      import ./nix/packages.nix {
        inherit pkgs;
        inherit (pkgs) lib;
        craneLib = crane.mkLib pkgs;
      };
  in {
    overlays.default = final: _prev: let
      built = mkPackages final;
    in
      lib.genAttrs tools (name: built.${name})
      // {tys-tools = built.workspace;};

    packages = forAllSystems (pkgs: let
      built = mkPackages pkgs;
    in
      lib.genAttrs tools (name: built.${name})
      // {
        workspace = built.workspace;
        default = pkgs.symlinkJoin {
          name = "tys-tools-${built.workspace.version}";
          paths = map (name: built.${name}) tools;
        };
      });

    homeManagerModules =
      lib.genAttrs tools (name: import ./nix/modules/${name}.nix {inherit self;})
      // {
        default = {imports = map (name: self.homeManagerModules.${name}) tools;};
      };

    checks = forAllSystems (pkgs: let
      built = mkPackages pkgs;
      inherit (built) craneLib commonArgs cargoArtifacts;
    in
      lib.genAttrs tools (name: built.${name})
      // {
        clippy = craneLib.cargoClippy (commonArgs
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--workspace --all-targets";
          });
        tests = craneLib.cargoTest (commonArgs // {inherit cargoArtifacts;});
        fmt = craneLib.cargoFmt {inherit (built) src;};
      });

    devShells = forAllSystems (pkgs: let
      built = mkPackages pkgs;
    in {
      default = built.craneLib.devShell {
        inputsFrom = [built.workspace];
        packages = with pkgs; [
          rust-analyzer
          cargo-nextest
          cargo-edit
          ffmpeg-full
          pulseaudio
          xdotool
          libnotify
          grim
          wf-recorder
          wl-clipboard
          wtype
        ];
        inherit (built.commonArgs) TRANSCRIBE_CMAKE_ARGS;
        shellHook = ''
          export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}:''${XDG_DATA_DIRS:-}"
        '';
      };
    });

    formatter = forAllSystems (pkgs: pkgs.alejandra);
  };
}
