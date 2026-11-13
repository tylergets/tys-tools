{self}:
import ./lib.nix {inherit self;} {
  name = "tyquote";
  description = "tyquote personal quote collection";
  hasService = false;
  settingsExample = {
    directory = "~/Quotes";
    default_author = null;
    show_author = true;
    author_prefix = "— ";
    day_boundary = "utc";
  };
}
