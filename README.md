# spice-lsp

A minimal language server for SPICE circuit netlists.

## Supported files

| Extension | Description |
|-----------|--------------|
| `.cir`, `.sp`, `.spi` | Circuit netlists |
| `.lib` | Model/subcircuit libraries |
| `.mod`, `.mdl` | Model files |

This server works on raw text (line/card scanning), not the `tree-sitter-spice` grammar, but
targets the same netlist file family - see that repo for the precise grammar coverage.

## Not supported

Other file types in the SPICE ecosystem aren't netlists and aren't understood by this server:

- **LTspice schematics** (`.asc`) and **symbols** (`.asy`)
- **LTspice simulation output** (`.raw`) - binary waveform data
- **NASA/NAIF SPICE toolkit kernels** (`.bsp`, `.tpc`, `.tls`) - spacecraft ephemeris data from an
  entirely unrelated "SPICE" (NASA's, not Berkeley's)

## Features

- **Completion**: dot-command directives with syntax + docs (`.tran`, `.ac`, `.model`, `.subckt`,
  ...), plus `.subckt`/`.model` names defined in the current file (and its `.include`/`.inc`/`.lib`
  files)
- **Hover**: built-in documentation for every dot-command directive (syntax summary + description),
  a subcircuit's port list, or a model's type
- **Go to definition**: jump from an `X...` instance or model reference to its `.subckt`/`.model`
  definition — including across `.include`/`.inc`/`.lib <path>` files. Also works directly on the
  include statement itself (jumps to the top of the referenced file)
- **Find references**: list every instance referencing a given `.subckt`/`.model`, across the
  current file and its includes
- **Document symbols** (outline): lists every `.subckt`/`.model` defined in the current file
- **Document formatting**: normalizes whitespace to single spaces between tokens, formats `+`
  continuation lines consistently, and trims trailing whitespace — while leaving quoted strings,
  `{...}` expressions, and comments untouched
- **Diagnostics**:
  - unbalanced `.subckt`/`.ends`, `.lib`/`.endl`, and `.if`/`.endif` blocks
  - `X...` instances referencing a subcircuit that isn't defined anywhere reachable
  - `X...` instances whose node count doesn't match the referenced `.subckt`'s port count
  - duplicate instance names within the same scope (top level, or within a `.subckt` body)
  - `D`/`Q`/`J`/`M`/`Z` instances referencing a `.model` that isn't defined anywhere reachable

### Known limitations

- Include resolution is one level deep (an included file's own includes aren't followed) and
  relative-path-only (no environment variable expansion).
- Reference detection uses a lightweight heuristic (the last non-`key=value` token on a component
  card), not a full parser — unusual device syntax may not be recognized.
- Hover documentation for dot-commands is a single unified reference, not split per dialect
  (HSPICE/ngspice/LTspice sometimes disagree on exact syntax for the same directive).
- The formatter normalizes inter-token whitespace only; it does not do columnar table alignment
  across consecutive cards.

## Editor support

- **Zed**: via [spice-lang](https://github.com/MrPoloGit/spice-lang) (downloaded automatically)
- **Neovim / Vim**: point `nvim-lspconfig` or `vim-lsp` at the binary
- **Emacs**: use `lsp-mode` with a custom server entry

## Installation

### From GitHub releases

Download the binary for your platform from the [releases page](https://github.com/MrPoloGit/spice-lsp/releases).

### From source

```bash
cargo install --git https://github.com/MrPoloGit/spice-lsp
```

## Usage

The server communicates over stdio (standard LSP transport):

```bash
spice-lsp
```

Point your editor's LSP client at `spice-lsp` with file types `*.cir`, `*.sp`, `*.spi`, `*.lib`,
`*.mod`, `*.mdl`.

### Related resources
- https://ngspice.sourceforge.io/docs.html
- https://www.analog.com/en/resources/design-tools-and-calculators/ltspice-simulator.html
