# A capture environment, separate from the app packages so existing builds can be reused.
{pkgs}: let
  python = pkgs.python3.withPackages (p: [p.pygobject3 p.pycairo p.dbus-python]);
in
  pkgs.stdenvNoCC.mkDerivation {
    pname = "tys-tools-screenshots";
    version = "1";
    src = ../site/screenshots;
    nativeBuildInputs = [pkgs.wrapGAppsHook3 pkgs.gobject-introspection pkgs.makeWrapper];
    buildInputs = [pkgs.gtk3 pkgs.libdbusmenu-gtk3];
    installPhase = ''
      mkdir -p $out/bin $out/share/tys-tools-screenshots
      cp -r . $out/share/tys-tools-screenshots/
      makeWrapper ${python}/bin/python $out/bin/ty-screenshots \
        --add-flags $out/share/tys-tools-screenshots/capture.py \
        --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.bash pkgs.bubblewrap pkgs.xvfb pkgs.dbus pkgs.openbox pkgs.xdotool pkgs.xterm pkgs.coreutils]} \
        --set SCREENSHOT_FONT_DIR ${pkgs.dejavu_fonts}/share/fonts/truetype \
        --set SCREENSHOT_ICON_DIR ${pkgs.adwaita-icon-theme}/share/icons \
        --set LOCALE_ARCHIVE ${pkgs.glibcLocales}/lib/locale/locale-archive
    '';
    meta.mainProgram = "ty-screenshots";
  }
