{self}:
import ./lib.nix {inherit self;} {
  name = "tybat";
  description = "Battery monitor with focus-grabbing critical countdowns";
  extraOptions = {lib, ...}: {
    chargeLimit = lib.mkOption {
      default = null;
      description = "Declarative charge limit; null disables charge control. Tray overrides persist until reset or this default changes.";
      type = lib.types.nullOr (lib.types.submodule {
        options = {
          percent = lib.mkOption {
            type = lib.types.ints.between 25 100;
            default = 80;
            description = "Default maximum battery charge percentage.";
          };
          command = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            description = "Privileged helper command and fixed arguments. The percentage is appended as one argument.";
          };
        };
      });
    };
  };
  toSettings = cfg:
    if cfg.chargeLimit == null
    then {}
    else {charge_limit = cfg.chargeLimit;};
  settingsExample = {
    warning_percent = 20;
    critical_percent = 5;
    countdown_seconds = 60;
    snooze_seconds = 60;
    poll_seconds = 5;
    action = "suspend";
  };
}
