{self}: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.programs.tyquote;
in {
  options.programs.tyquote = {
    enable = lib.mkEnableOption "tyquote personal quote collection";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.tyquote;
      description = "The tyquote package to install.";
    };
  };
  config = lib.mkIf cfg.enable {home.packages = [cfg.package];};
}
