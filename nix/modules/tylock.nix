{self}:
import ./lib.nix {inherit self;} {
  name = "tylock";
  description = "tylock screen locker";
  # Started by the idle daemon (xss-lock, swayidle), not as a service.
  hasService = false;
  settingsExample = {
    blur = 12;
    dim = 0.35;
    time_format = "%-I:%M";
    quote_command = ["tyquote" "today"];
    colors.accent = "#88c0d0";
  };
  extraOptions = {lib, ...}: {
    wallpaper = lib.mkOption {
      type = lib.types.nullOr (lib.types.either lib.types.path lib.types.str);
      default = null;
      example = lib.literalExpression "config.stylix.image";
      description = ''
        Image behind the clock, scaled to cover each monitor. Null uses
        {file}`~/.background-image` when it exists.
      '';
    };
  };
  # Interpolated, so a path in your config is copied to the store and stays readable.
  toSettings = cfg:
    if cfg.wallpaper != null
    then {wallpaper = "${cfg.wallpaper}";}
    else {};
}
