# Virtual PLC - Structured Text Extension

VS Code extension providing IEC 61131-3 Structured Text syntax highlighting for Virtual PLC development.

## Features

- **Syntax Highlighting** for IEC 61131-3 Structured Text (.st files)
  - Keywords (IF, THEN, CASE, FOR, WHILE, etc.)
  - Declaration blocks (PROGRAM, FUNCTION, VAR, etc.)
  - Data types (BOOL, INT, REAL, TIME, etc.)
  - Operators (AND, OR, NOT, :=, etc.)
  - Comments (block `(* *)` and line `//`)
  - String literals with escape sequences
  - Numeric literals (decimal, hex, binary, octal)
  - Time literals (T#1s, TIME#500ms, etc.)
  - I/O addressing (%IX0.0, %QW0, etc.)
  - Standard function blocks (TON, CTU, etc.)

- **Code Folding** for program blocks and control structures

- **Bracket Matching** and auto-closing pairs

- **Comment Toggling** (Ctrl+/ for line comments)

## Installation

### From VSIX (Local Install)

1. Package the extension:
   ```bash
   cd vscode-vplc
   npx vsce package
   ```

2. Install in VS Code:
   - Open VS Code
   - Press `Ctrl+Shift+P` (or `Cmd+Shift+P` on macOS)
   - Type "Extensions: Install from VSIX..."
   - Select the generated `.vsix` file

### Manual Installation (Development)

1. Copy the `vscode-vplc` folder to your VS Code extensions directory:
   - **Windows**: `%USERPROFILE%\.vscode\extensions\`
   - **macOS**: `~/.vscode/extensions/`
   - **Linux**: `~/.vscode/extensions/`

2. Restart VS Code

3. Open any `.st` file to see syntax highlighting

## Usage with Virtual PLC

This extension complements the Virtual PLC toolchain:

```bash
# Compile your ST program
cargo run -p plc-daemon -- compile program.st -o program.wasm

# Validate the generated WebAssembly
cargo run -p plc-daemon -- validate program.wasm

# Run in simulation mode
cargo run -p plc-daemon -- simulate program.wasm --cycles 100
```

## Example

```iecst
(*
 * Example: Motor Control with Safety Interlocks
 *)
PROGRAM MotorControl
VAR
    start_button : BOOL := FALSE;
    stop_button : BOOL := TRUE;     // NC contact
    e_stop : BOOL := TRUE;          // NC contact
    motor_running : BOOL := FALSE;
END_VAR

// Check safety interlocks
IF stop_button AND e_stop THEN
    IF start_button AND NOT motor_running THEN
        motor_running := TRUE;
    END_IF;
ELSE
    motor_running := FALSE;
END_IF;

END_PROGRAM
```

## Supported Language Features

| Feature | Status |
|---------|--------|
| PROGRAM blocks | Highlighted |
| FUNCTION blocks | Highlighted |
| FUNCTION_BLOCK | Highlighted |
| VAR declarations | Highlighted |
| Control flow (IF, CASE, FOR, WHILE) | Highlighted |
| Data types | Highlighted |
| Time literals | Highlighted |
| I/O addressing (AT %) | Highlighted |
| Standard functions | Highlighted |

## Publishing to VS Code Marketplace

Before publishing to the VS Code Marketplace, update these placeholders in `package.json`:

1. **`publisher`**: Replace `"virtual-plc"` with your VS Code Marketplace publisher ID
2. **`repository.url`**: Replace `"https://github.com/your-org/virtual-plc"` with the actual repository URL

Then package and publish:

```bash
# Login to your publisher account
npx vsce login <publisher-id>

# Publish the extension
npx vsce publish
```

## Contributing

Contributions are welcome! Please see the main Virtual PLC repository for contribution guidelines:
https://github.com/hadijannat/virtual-plc

## License

This extension is part of the Virtual PLC project. See the main repository for license information.
