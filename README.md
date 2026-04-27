# dictaphile

A Windows dictation tool that records audio via a global hotkey, transcribes it locally with Whisper, polishes the text with a local LLM, and pastes the result wherever your cursor is. No account or cloud service required — audio and text never leave your machine.

## How it works

1. Press your configured hotkey (default **Alt+R**) — starts recording from the default microphone
2. Press it again — stops recording and:
   - Encodes audio as WAV and sends it to a local Whisper server
   - Sends the transcript to a local LLM for cleanup
   - Copies the result to the clipboard and simulates Ctrl+V to paste it

Hotkey, LLM endpoint, distillation mode, and custom prompts are all configurable via the tray icon → **Settings**.

## Prerequisites

| Dependency | Purpose |
|---|---|
| [whisper.cpp server](https://github.com/ggml-org/whisper.cpp) | Local speech-to-text |
| An LLM runner (e.g. [Ollama](https://ollama.com), llama.cpp server, Docker) | Local LLM for text cleanup |
| Rust (stable, MSVC toolchain) | Build the app |
| Visual Studio C++ Build Tools | Required by the MSVC Rust toolchain |

## Setup

**1. Start Whisper server**
```
server.exe -m models/ggml-large-v3-turbo.bin --port 8080
```

**2. Start an LLM runner**

Ollama is the simplest option:
```
ollama pull qwen2.5:7b-instruct
ollama serve
```

Or use your favourite LLM runner — any service that exposes an Ollama-compatible `/api/generate` endpoint works. Point `ollama_url` in `config.toml` at whatever host and port you use.

**3. Clone and build** (use the x64 Native Tools Command Prompt for VS)
```
git clone <repo>
cd dictaphile
cargo build --release
```

**4. Configure**

Copy or edit `config.toml` next to the binary:
```toml
whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
ollama_url    = "http://localhost:11434/api/generate"
whisper_model = "whisper-large-turbo"
llm_model     = "qwen2.5:7b-instruct"

prompt = """
Rewrite the following dictation as clear, concise English. \
Translate to English if needed. Fix grammar, remove filler, preserve intent. \
Output ONLY the rewritten text, no preamble.

---
"""
```

`ollama_url` is the LLM generate endpoint — update the host/port to match your runner.

**5. Run**
```
dictaphile.exe
```

The app runs silently in the system tray. Right-click the tray icon for settings or to exit; double-click to show the log window.

## Building the installer

Requires [Inno Setup 6](https://jrsoftware.org/isinfo.php). Run from the repo root:

```powershell
.\build-installer.ps1
```

This runs `cargo build --release` and then compiles `installer.iss`. Output: `installer\DictaphileSetup.exe`.

The installer:
- Installs to `%LocalAppData%\Dictaphile` (no UAC prompt)
- Creates a Start Menu shortcut
- Optionally adds Dictaphile to Windows startup (checked by default)
- Ships a default `config.toml` on first install; upgrades never overwrite it

## Running tests

```
cargo test
```

Tests cover WAV encoding correctness, config parsing, and the minimum-length guard. They do not require a running Whisper server or LLM.
