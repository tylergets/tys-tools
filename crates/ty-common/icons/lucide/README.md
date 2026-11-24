# Lucide menu icons

SVGs from [Lucide](https://lucide.dev), revision
`7981a0798a4f7184296bf42e94b6a8677f3ef2a0`, under the included ISC/MIT licenses.
The upstream files are unchanged.

`MenuIcon` in `src/tray.rs` maps actions to these glyphs and renders them in the
GTK menu foreground color. Menus export image data so tray hosts use this set
rather than substituting desktop-theme icons.

Use the same glyph for the same action across tools. Give distinct actions
distinct glyphs within a menu; leave repeated content rows and numeric choices plain.
