# waybright

Unified brightness control for Wayland desktops.

The `waybright` daemon combines physical brightness control and software
dimming into one virtual brightness range. The same executable is its CLI:

```console
waybright daemon
waybright list
waybright get @focused
waybright set @focused +5%
waybright set @all 50%
waybright refresh
```

The workspace contains four libraries:

- `waybright`: application state, brightness model, protocol, daemon, and client
- `waydark`: Wayland software dimming overlays
- `waydim`: backlight and DDC/CI hardware control
- `wayfocus`: focused-output detection for supported compositors

Clients connect through `$WAYBRIGHT_SOCKET` or
`$XDG_RUNTIME_DIR/waybright.sock`. The NixOS module configures a socket-activated
user service.

License: MIT
