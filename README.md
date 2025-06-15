# chunimidi

> [!NOTE]
> This is only for the JVS RGB lighting part, for the HID part, see [midkb](https://github.com/korewaChino/midkb).
> Initially designed for use with SDVX/K-Shoot, but works with pretty much any keyboard/mouse based input scheme.

chunimidi is a JVS-to-MIDI shim for CHUNITHM that translates JVS commands into Novation-compatible MIDI lighting notes.

This is initially designed only for Linux to be used with yet another named pipe-to-unix socket bridge, so it listens to a Unix domain socket for JVS commands. Maybe someone could fork it to actually listen to Windows named pipes?
