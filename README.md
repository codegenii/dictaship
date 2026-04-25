# dictaphile

A Windows dictation tool that records audio via a global hotkey, transcribes it locally with Whisper, polishes the text with an LLM, and pastes the result wherever your cursor is.

## How it works

1. Press **Ctrl+Alt+R** — starts recording from the default microphone
2. Press **Ctrl+Alt+R** again — stops recording and:
   - Encodes audio as WAV and sends it to a local Whisper server
   - Sends the transcript to a local Ollama model for cleanup
   - Copies the result to the clipboard and simulates Ctrl+V to paste it

## Prerequisites

| Dependency | Purpose |
|---|---|
| [whisper.cpp server](https://github.com/ggml-org/whisper.cpp) | Local speech-to-text |
| [Ollama](https://ollama.com) | Local LLM for text cleanup |
| Rust (stable, MSVC toolchain) | Build the app |
| Visual Studio C++ Build Tools | Required by the MSVC Rust toolchain |

## Setup

**1. Start Whisper server**
```
server.exe -m models/ggml-large-v3-turbo.bin --port 8080
```

**2. Pull and run Ollama model**
```
ollama pull qwen2.5:7b-instruct
ollama serve
```

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

**5. Run**
```
dictaphile.exe
```

The app runs silently in the background. Use Ctrl+Alt+R to toggle recording.

## Running tests

```
cargo test
```

Tests cover WAV encoding correctness, config parsing, and the minimum-length guard. Tests do not require Whisper or Ollama to be running.
