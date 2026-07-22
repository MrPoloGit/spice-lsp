# spice-lsp

A minimal language server for SPICE circuit netlists (`.cir`, `.sp`, `.spi`, `.lib`, `.mod`, `.mdl`).

## Features

- **Completion**: dot-command directives (`.tran`, `.ac`, `.model`, `.subckt`, ...), plus
  `.subckt`/`.model` names defined in the current file
- **Diagnostics**: unbalanced `.subckt`/`.ends`, `.lib`/`.endl`, and `.if`/`.endif` blocks

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
