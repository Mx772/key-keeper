# AGENTS.md

Notes for coding agents working on KeyKeeper. The README covers features, install and the release steps. This file covers how to build, test and **look at** the app while working on it.

## Layout

| Crate | What it does |
|---|---|
| `crates/keykeeper-core` | Model, encrypted vault, import/export, search and sort (`query.rs`), prices (`prices.rs`), trades. No GUI, no network. |
| `crates/keykeeper-steam` | Steam, IsThereAnyDeal and gg.deals clients behind a trait; tests use recorded responses. |
| `crates/keykeeper-app` | The RUNE desktop app. `src/ui/` holds the views, `src/*_actions.rs` the state changes, `src/tests.rs` drives the real UI headlessly. |

The binary is `keykeeper` (`target/debug/keykeeper.exe` on Windows).

## Checks (what CI runs)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # warnings fail CI
cargo test --workspace
```

CI also requires `keykeeper-core` line coverage of at least 80% (`cargo llvm-cov`), runs `cargo-deny`, and renders every scene with `--snap` on Windows and macOS (uploaded as `snapshots-*` artifacts).

## Conventions

- Serde names in `keykeeper-core` are part of the vault file format. Add fields with `#[serde(default)]`, and never rename or remove one.
- A library column needs to be added in three places: `COLUMNS` in `layout.rs`, the `TableColumn` and its cell in `ui/library.rs`, and, if it sorts, a `SortField` in `keykeeper-core/src/query.rs` mapped in both directions (`sort_for` in `app.rs` and `sort_to_string` in `layout.rs`).
- Saved column orders don't include new columns. RUNE adds those at the end, after the saved ones.
- UI tests find elements by `.key(...)`, so give anything clickable a stable key (for example `deal-{id}`, `steam-price-{id}`).
- Releases: bump `version` in the root `Cargo.toml`, generate the vault fixture, open a `release/<version>` PR, merge it, then push the `v<version>` tag. The README's "Releasing" section has the details.

## Seeing and driving the app (RUNE agent tooling)

RUNE builds agent tooling into every debug build. You can render the real app offscreen, drive it with scripted input and read its element tree, all without a window or a person. Run `target/debug/keykeeper.exe --help` for every flag.

### Screenshots of a scene

```sh
cargo build -p keykeeper-app
target/debug/keykeeper.exe --scene library --size 1600x900 --snap out.png
```

- Scenes are defined in `SCENES` in `crates/keykeeper-app/src/main.rs` and run on demo data from `demo.rs`, never a real vault: `lock create library detail hover-card row-menu bulk-edit match-review grouped view-menu grid datepick cart-button new-key import import-start settings trades cart templates shortcuts empty update update-notes update-lock light`. Add a scene there when a new screen needs one.
- The PNG is at scale 2 (pass `--scale 1` for smaller files). Open it with the Read tool to look at it.
- `--crop KEY` crops to one element, but only one that is on screen.
- `--theme light|dark` sets the system appearance.

### Element tree: find what is where

```sh
target/debug/keykeeper.exe --scene detail --size 1600x900 --tree tree.json
```

`tree.json` → `elements[]`, each with `key`, `path`, `rect` `[x, y, w, h]` in points, `label`, `help` (tooltip text) and `role`. Use it to get click coordinates, to check that an element exists, or to read the text in a cell. Column headers are keyed by column id (`steam_price`, `deals`, ...). Rects can extend past the viewport for content that is scrolled away.

### Scripted input: click, hover, scroll, type

```sh
target/debug/keykeeper.exe --scene library --size 1600x900 --script s.json --snap after.png
```

`s.json` is a list of timed events, in points, with `t` in seconds of virtual time:

```json
[
  {"t": 0.3, "type": "click", "x": 1560, "y": 209},
  {"t": 1.0, "type": "move",  "x": 1260, "y": 300},
  {"t": 1.2, "type": "wheel", "x": 1400, "y": 500, "dy": -500},
  {"t": 1.5, "type": "type",  "text": "hades"},
  {"t": 1.8, "type": "key",   "name": "Enter"},
  {"t": 2.5, "type": "mark",  "name": "end"}
]
```

Other event types: `double-click`, `secondary-click`, `drag` (`dx`, `dy`, `dur`), `scroll` (an eased swipe), `menu-pick` and `file-dialog`. The snapshot is taken at the `end` mark, or `--at SECONDS` / `--at mark:NAME`. Hovering long enough (more than the hover delay, about 0.5 s) shows tooltips and the game hover card. A negative `dy` on `wheel` scrolls down.

Typical loop: dump `--tree` to get coordinates, write a script, `--snap`, look at the PNG.

Other tooling: `--frames N --out DIR` dumps frames, `--video out.mp4` (needs ffmpeg), `--trace` writes motion traces, and `--window-script FILE` replays a script in a real window.

### Gotcha: your real config.toml applies

Snapshots and scripts read the user's real `config.toml` (`%APPDATA%\KeyKeeper\config\config.toml` on Windows), including column widths, hidden columns, sort, theme and density. Saved widths can push columns off screen, so a snapshot can look broken when the code is fine. For a default-layout render, back the file up, remove `[library.column_widths]`, render, then restore it and check it matches the backup. Never leave the user's config modified.

### Headless UI tests

`crates/keykeeper-app/src/tests.rs` drives the same UI in-process: `runtime(state)` or `runtime_sized(state, (w, h))`, `click(&mut rt, "element-key")`, `rt.find(key)`, `rt.frame(dt)`, `rt.update(|s, tx| ...)`, `rt.take_url_requests()` for links the UI opened, and `wait_for_steam` for background jobs. Network services are fakes that implement the `keykeeper-steam` trait (see `PricedSteam`). `click` presses the element's centre, so if a table is wider than the test window, use `runtime_sized` with a wider size or the click lands off screen.

## RUNE itself

RUNE is pinned by git rev in the root `Cargo.toml`. Its source, with its own `AGENTS.md`, `ARCHITECTURE.md` and `README.md`, is in the cargo git checkout: `~/.cargo/git/checkouts/rune-*/<short rev>/`. The tooling flags are defined in `crates/rune/src/tooling/args.rs` and script events in `crates/rune/src/tooling/script.rs`. Widgets (`Table`, `Menu`, `Button`, ...) come from `rune-kit`.
