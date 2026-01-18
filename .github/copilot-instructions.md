# Copilot Instructions for musicPlayer

This project is a minimal Rust command-line audio player. It uses the `radio` crate for audio decoding and playback.

## Project Structure
- **src/main.rs**: Entry point. Handles command-line arguments, file I/O, and audio playback logic.
- **Cargo.toml**: Declares dependencies and project metadata.

## Key Patterns & Workflows
- **Audio Playback**: Uses `radio::{Decoder, OutputStream, Sink}`. The main flow is:
  1. Parse a single audio file path from command-line arguments.
  2. Open the file and create a `Decoder`.
  3. Set up an `OutputStream` and `Sink`.
  4. Append the decoder to the sink and block until playback ends.
- **Error Handling**: Exits with error messages if file open or decode fails, or if arguments are missing.
- **Usage**: Run with `cargo run <audio_file_path>`.

## Developer Workflows
- **Build**: `cargo build` (standard Rust workflow)
- **Run**: `cargo run <audio_file_path>`
- **Dependencies**: Managed in `Cargo.toml`. Only `radio` is used.
- **No tests**: There are currently no test files or test frameworks set up.

## Conventions
- All logic is in `main.rs` (single-file app).
- Error messages are printed to stderr and the process exits on failure.
- No configuration files or environment variables are used beyond standard Rust/Cargo.

## Integration Points
- Relies on the `radio` crate for all audio handling. No other external APIs or services.

## Extending the Project
- Add new features by expanding `main.rs` or splitting logic into additional modules under `src/`.
- Follow Rust idioms for error handling and argument parsing.

---
For more details, see `src/main.rs` and `Cargo.toml`.
