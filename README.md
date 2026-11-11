# Ez2Nec
Convert EZNEC .ez files to .NEC for NEC2, NEC4, and NEC5 engines

EZNEC models are binary files that only EZNEC reads. Ez2Nec decodes them
without EZNEC installed, so the models stay usable whatever happens to the
program.

## Status

The decoder is in place; writing NEC decks is not implemented yet.

    ez2nec --dump model.ez

prints the decoded model in its own display unit: wires, sources, loads,
transmission lines, L networks, ground, wire loss, insulation, plot and sweep
settings, and the EZNEC version and engine that saved it. It then lists

- fields whose place in the file is known but whose meaning is not, and
- any non-zero byte that the decoder did not account for.

The decoder refuses a file whose layout it doesn't recognise, and treats an
unknown code in a field that changes the model as an error. It never guesses.

## Building

    cargo build --release

There are no dependencies beyond the Rust standard library.

## Tests

    cargo test

The tests build `.ez` images in code, so the repository holds no EZNEC files.
To decode a collection of your own files as well, name its directories:

    EZ2NEC_CORPUS=~/models:~/more cargo test --test corpus -- --nocapture

That test fails if any file is refused or has unaccounted bytes.

## License

Apache-2.0. See [LICENSE](LICENSE).
