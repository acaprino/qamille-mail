# Meeting processing: provenance

No Meetily implementation code is copied in this module. Meetily informed the boundaries between capture, durable audio, speech recognition and summaries; see the project reference register.

## Local transcription runtime

Since the compute server (spec `docs/specs/2026-11-07-server-calcolo-locale-remoto-design.md`, D3), whisper.cpp runs in `qamille-server`, not in the app: the runtime, the models and their download live in `compute/src/speech.rs`, and the two license notices in `compute/src/`. The app keeps the 15-second windows, the cache and the jobs, and sends each window to `POST /v1/audio/transcriptions`. On Linux the server does not download the runtime: it runs a `whisper-cli` found in the speech models folder, next to `qamille-server` or on `PATH`.

- Project: https://github.com/ggml-org/whisper.cpp
- Pinned release: `v1.9.2`.
- Official asset: `whisper-bin-x64.zip`, 8,194,445 bytes.
- SHA-256 from GitHub release metadata: `49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a`.
- Only `whisper-cli.exe`, `whisper.dll` and `ggml*.dll` are installed. SDL, Parakeet and example executables are not extracted.
- The MIT notice in `compute/src/WHISPER_CPP_LICENSE.txt` is written into the installed runtime directory.
- Runtime installation is explicit, hash-verified and Windows x64 only. The build does not download or execute the runtime.

## Models

The multilingual Tiny, Base and Small GGML files come from `https://huggingface.co/ggerganov/whisper.cpp` at commit `5359861c739e955e79d9a303bcbc70fb988958b1`. Exact LFS SHA-256 digests and lengths are recorded in `compute/src/speech.rs`; the server rechecks an installed model once per process (and again when the file changes) before using it. OpenAI's MIT notice is included as `compute/src/OPENAI_WHISPER_LICENSE.txt` and installed beside the models. Models are downloaded only through an explicit install job.

## Validation performed

On 2 November 2026 the official runtime archive was downloaded into an isolated temporary directory, its SHA-256 matched the release metadata, and `whisper-cli.exe --help` exited with code 0 on this Windows host. The minimal DLL selection above was verified separately. This test used no microphone, no personal audio and no speech model; transcription accuracy and model inference still require a representative audio/model test.

## Import

Symphonia decodes supported files packet by packet. It is a separate Cargo dependency, not Meetily source. Its dependency license and notices must be included by the application's distribution process.

The selected source is copied into the meeting directory through a synced staging file and atomic rename. Decoding uses this snapshot. Failed imports remove their snapshot and decoded chunks; the external source is never removed. Five-second PCM chunks keep decoder output bounded. Resumable speech windows are cached within the meeting directory, keyed by mixed audio content, runtime/model, language and the endpoint's identity (`local` for the local compute server whatever its port, the address of a remote one, or the configured external URL); credentials are excluded. Windows cached before the compute server keep their keys.

## Remote transcription

Explicit remote jobs use the configured OpenAI-compatible endpoint and its API key, with the selected transcription model. Requests contain one 15-second 16 kHz mono WAV window. Whisper-compatible models request `verbose_json`; GPT transcription models request `json`, per https://developers.openai.com/api/reference/resources/audio/subresources/transcriptions/methods/create. Responses without segment timestamps are represented by the full window interval, not word-level timing. A synthetic loopback HTTP test checks the path, authorization, multipart model/language/format and response parsing without using external services or real credentials.

## Current boundaries

Speech recognition runs after capture, through a cancellable CPU child process, with an energy silence gate and 500 ms overlap. It does not implement neural VAD, diarization or live captions. Windows x64 is the bundled local runtime target. Summary mapping and reduction cover the complete input through bounded chunks and shrink them on provider context errors; provider errors or invalid citations fail the job instead of publishing a fabricated fallback. Existing AI transport requests have a 180-second upper timeout, so cancellation takes effect immediately before/after a request rather than during that request.
