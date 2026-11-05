# Builds a Home Manager module for one ty* tool. Every tool gets the same shape:
#
#   programs.<name> = {
#     enable = true;
#     package = pkgs.<name>;      # from the overlay, else this flake's package
#     settings = { ... };          # written to ~/.config/<name>/config.yaml
#     service.enable = true;       # systemd user service bound to graphical-session.target
#   };
#
# Tools can add typed options (`extraOptions`) that are merged under `settings`
# (`toSettings`), and extra config such as assertions (`extraConfig`).
{self}: {
  name,
  description,
  serviceArgs ? "run",
  serviceDescription ? description,
  settingsExample ? {},
  extraOptions ? _: {},
  toSettings ? _: {},
  extraConfig ? _: {},
}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.${name};
  yaml = pkgs.formats.yaml {};
  settings = lib.recursiveUpdate (toSettings cfg) cfg.settings;
  configFile = yaml.generate "${name}-config.yaml" settings;
  system = pkgs.stdenv.hostPlatform.system;
in {
  options.programs.${name} =
    {
      enable = lib.mkEnableOption description;

      package = lib.mkOption {
        type = lib.types.package;
        default = pkgs.${name} or self.packages.${system}.${name};
        defaultText = lib.literalExpression "pkgs.${name}";
        description = "The ${name} package to use.";
      };

      settings = lib.mkOption {
        inherit (yaml) type;
        default = {};
        example = lib.literalExpression (lib.generators.toPretty {} settingsExample);
        description = ''
          ${name} configuration, written to {file}`~/.config/${name}/config.yaml`
          (see the ${name} README for keys). Merged over any typed options.
        '';
      };

      service.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Run ${name} as a systemd user service in the graphical session.";
      };
    }
    // extraOptions {inherit lib cfg;};

  config = lib.mkIf cfg.enable (lib.mkMerge [
    {
      home.packages = [cfg.package];

      # Written to the discovered path too, so CLI subcommands and hotkeys use the same
      # config as the service.
      xdg.configFile."${name}/config.yaml".source = configFile;

      systemd.user.services.${name} = lib.mkIf cfg.service.enable {
        Unit = {
          Description = serviceDescription;
          After = ["graphical-session.target"];
          PartOf = ["graphical-session.target"];
          X-Restart-Triggers = ["${configFile}"];
        };
        Service = {
          ExecStart = "${lib.getExe cfg.package} --config ${configFile} ${serviceArgs}";
          Restart = "on-failure";
          RestartSec = 2;
        };
        Install.WantedBy = ["graphical-session.target"];
      };
    }
    (extraConfig {inherit lib cfg settings;})
  ]);
}
