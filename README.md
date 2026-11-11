# Ez2Nec
Convert EZNEC .ez files to .NEC for NEC2, NEC4, and NEC5 engines

EZNEC models are binary files that only EZNEC reads. Ez2Nec decodes them
without EZNEC installed and writes NEC decks, so the models stay usable
whatever happens to the program.

## Usage

    ez2nec --engine nec4 -o model.nec model.ez
    ez2nec --engine nec4 --sweep -o model.nec model.ez
    ez2nec --dump model.ez

`--engine` takes `nec2`, `nec4` (NEC-4.2) or `nec5`. The deck goes to standard
output unless `-o` names a file.

The deck is written for that one engine and runs on it as the model states,
with nothing else needed. Segment choices and element values follow EZNEC; the
cards are the engine's own:

| EZNEC | NEC-2 | NEC-4.2 | NEC-5 |
|---|---|---|---|
| Voltage source | `EX 0` on the segment | `EX 0` on the segment | `EX 0` at the nearest segment end |
| Current source | dummy wire and `NT` | `EX 6` | `EX 4` |
| Split source | the two segments at the junction | the same | one source at the junction |
| R + jX, series and parallel RLC loads | `LD 4`, `LD 0`, `LD 1` | the same | the same |
| Trap and Laplace loads | `LD 4` at the model's frequency | the same | the same |
| Wire loss | `LD 5` | `LD 5` | `LD 5` |
| Insulation | equivalent radius and `LD 2` | `IS` | equivalent radius and `LD 2` |
| Transmission line | `TL` | `TL` | `TL` at segment ends |
| Line with loss, L network | `NT` at the model's frequency | the same | the same |
| Shorted stub | `TL` to a dummy wire | the same | the same |
| Free space, perfect ground | no `GN`, `GN 1` | the same | the same |
| Real, high accuracy | `GN 2` | `GN 2` | `GN 2` |
| Real, MININEC | `GN 1`, `GD`, `RP 3` | `GN 1`, `GD` | `GN 1`, `GD` |

Geometry is in the model's display unit with a `GS` card, and source values
are peak (EZNEC's are RMS). Each stepped-diameter section stays its own wire;
no stepped-diameter correction is applied.

Whatever the engine can't express exactly is said in the deck, in `CM ez2nec:`
lines, and on standard error: for example an element computed at one
frequency, or a one-segment feed wire that NEC-5 needs as two segments.

The deck runs the model's frequency. With `--sweep` it runs the frequency
sweep stored in the model instead: start, then each step up to stop.

- Usually that is one `FR` card that steps the frequency.
- A trap or Laplace load, a line with loss, an L network, or a load resistance
  that scales with frequency has a different value at each frequency. The deck
  then has a block for each frequency: every `LD`, `FR`, the ground cards,
  every `NT` and `TL`, and `RP`. (NEC replaces all loads when new `LD` cards
  follow a run, and all networks when new `NT` or `TL` cards do.)
- NEC-5 with the MININEC-type ground gets a block for each frequency too.
  Given `GN 1` and `GD` once, NEC-5 solves the first frequency over perfect
  ground and the later ones over the real ground.

Without `--sweep`, a stored sweep is noted and not written.

`--dump` prints the decoded model in its own display unit: wires, sources,
loads, transmission lines, L networks, ground, wire loss, insulation, plot and
sweep settings, and the EZNEC version and engine that saved it. It then lists

- fields whose place in the file is known but whose meaning is not, and
- any non-zero byte that the decoder did not account for.

The decoder refuses a file whose layout it doesn't recognise, and treats an
unknown code in a field that changes the model as an error. It never guesses.

## Not yet confirmed against EZNEC

- A second ground medium, and L network ports with R, L and C in parallel.
- Whether a stub can be open as well as shorted; only shorted stubs have
  been seen.

## Building

    cargo build --release

There are no dependencies beyond the Rust standard library.

## Tests

    cargo test

The tests build `.ez` images and models in code, so the repository holds no
EZNEC files. To decode a collection of your own files as well, name its
directories:

    EZ2NEC_CORPUS=~/models:~/more cargo test --test corpus -- --nocapture

That test fails if any file is refused or has unaccounted bytes.

## License

Apache-2.0. See [LICENSE](LICENSE).
