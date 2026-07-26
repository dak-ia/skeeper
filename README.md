SessionKeeper

## Manpage

`docs/man/skeeper.1` is generated from the CLI definition via `cargo mangen` (which runs the `xtask` crate under the hood).

To install locally:

```sh
cp docs/man/skeeper.1 /usr/local/share/man/man1/
# `man skeeper` should then show it
```

To regenerate after editing the CLI:

```sh
cargo mangen
```
