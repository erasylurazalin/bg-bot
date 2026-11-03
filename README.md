# bg-bot

A Hearthstone Battlegrounds simulator in Rust, with PPO self-play training in
Python. It never touches the real game client: the aim is a suggestion window
that watches my games and tells me what to buy.

What is in here:

- `crates/bgcore`: the game. Shop phases, fights, heroes, trinkets, Dark
  Gifts, with every card and every rule number as data in the toml files.
- `crates/bglog`: reads the game's `Power.log` into boards and hands.
- `crates/bgbench`: the fuzzer, the benchmarks, bot matches, and the checks
  against real games.
- `crates/bgsandbox`: a terminal sandbox to play shop phases by hand.
- `crates/bgpy`: PyO3 bindings, so Python can step many games at once.
- `train/`: the PPO loop, the evals and the run scripts.

Build and test:

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --release -p bgbench
./target/release/bgbench fuzz 20000
./target/release/bgbench games 10
```

The card data the engine reads (`ref/`) is built locally from the installed
game's files and is not in the repo; `docs/card-data.md` says how, and
`tools/export_cards.py` does it.

The rules are in `docs/rules/design.md`, and `docs/engine.md` maps the code.

Timewarp content and a few heroes are not built yet. `TODO.md` has the rest
of the plan.
