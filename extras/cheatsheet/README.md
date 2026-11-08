# driftwm-cheatsheet

A terminal cheatsheet of the keybindings your running driftwm has loaded. It
reads `driftwm msg --json bindings`, so it shows the *effective* bindings —
built-in defaults merged with your config, including anything you unbound —
and follows config reloads each time it is opened.

Bindings are grouped (Apps, Windows, Navigation, Zoom, Bookmarks, Session,
Mouse, Modifier taps), described in plain words, and shown next to the action
as written in the config. Keys bound to the same action share a row.

- **Type** to filter (every word must match the keys, wording, action or
  section); **Esc** clears the filter, then quits.
- **↑ ↓ PgUp PgDn Home End** scroll.

Gesture, touch and hot-corner bindings are not listed (the `bindings` request
does not report them yet).

## Build and run

It is a standalone crate (not part of the compositor's build):

```sh
cargo build --release --manifest-path extras/cheatsheet/Cargo.toml
extras/cheatsheet/target/release/driftwm-cheatsheet
```

`--from <file|->` reads the `bindings` JSON from a file or stdin instead of
asking the compositor, to try it outside a session.

## Bind it to a key

It is an ordinary terminal program, so any terminal will host it:

```toml
[keybindings]
"mod+slash" = "spawn foot -a cheatsheet driftwm-cheatsheet"
```

Give that `app_id` a window rule to size and place it as a floating overlay.
