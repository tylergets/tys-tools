# tykey keeps typed options for its rules (they are merged under `settings`).
let
  optional = name: value: cond:
    if cond
    then {${name} = value;}
    else {};
in
{self}:
import ./lib.nix {inherit self;} {
  name = "tykey";
  description = "tykey text expansion daemon";
  settingsExample = {watch = true;};

  extraOptions = {lib, ...}: let
    inherit (lib) mkOption types;
  in {
    expansions = mkOption {
      type = types.listOf (types.submodule {
        options = {
          trigger = mkOption {
            type = types.str;
            description = "Typed trigger text.";
            example = "sig;";
          };
          expansion = mkOption {
            type = types.str;
            description = "Text or macro sequence to emit.";
            example = "Thanks, Tyler{{KEY:ENTER}}";
          };
        };
      });
      default = [];
      description = "Expansion rules.";
    };

    snippets = mkOption {
      type = types.listOf (types.submodule {
        options = {
          title = mkOption {
            type = types.str;
            description = "Menu item label shown in the tray menu.";
            example = "Work email";
          };
          content = mkOption {
            type = types.str;
            description = "Text copied to the clipboard when the item is clicked.";
            example = "tyler@company.com";
          };
        };
      });
      default = [];
      description = "Clipboard snippets shown in the tray menu.";
    };

    matchBehavior = mkOption {
      type = types.nullOr (types.enum ["immediate" "boundary"]);
      default = "immediate";
      description = "When triggers fire.";
    };

    boundaryChars = mkOption {
      type = types.nullOr types.str;
      default = null;
      description = "Boundary characters used when matchBehavior is boundary.";
    };

    globals = mkOption {
      type = types.attrsOf types.str;
      default = {};
      description = "Template macro globals, keyed by macro name.";
      example = {
        SIGNOFF = "Thanks, Tyler{{KEY:ENTER}}";
        ROCKET = "{{EMOJI:rocket}}";
      };
    };

    notifications = {
      onExpansion = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = "Notify when an expansion fires (null keeps the tykey default).";
      };
      onSnippetCopy = mkOption {
        type = types.nullOr types.bool;
        default = null;
        description = "Notify when a tray snippet is copied (null keeps the tykey default).";
      };
    };
  };

  toSettings = cfg:
    {expansions = cfg.expansions;}
    // optional "snippets" cfg.snippets (cfg.snippets != [])
    // optional "match_behavior" cfg.matchBehavior (cfg.matchBehavior != null)
    // optional "boundary_chars" cfg.boundaryChars (cfg.boundaryChars != null)
    // optional "globals" cfg.globals (cfg.globals != {})
    // (let
      n =
        optional "on_expansion" cfg.notifications.onExpansion (cfg.notifications.onExpansion != null)
        // optional "on_snippet_copy" cfg.notifications.onSnippetCopy (cfg.notifications.onSnippetCopy != null);
    in
      optional "notifications" n (n != {}));

  extraConfig = {settings, ...}: {
    assertions = [
      {
        assertion = (settings.expansions or []) != [];
        message = "programs.tykey: at least one expansion is required (expansions or settings.expansions).";
      }
    ];
  };
}
