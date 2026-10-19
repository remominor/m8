# m8

A voice brain for AI companions in games. You talk to your mate out loud; it hears the game,
decides what to do, does it, and answers in a voice of its own.

m8 knows nothing about any one game. The game tells it what's going on, in words, and what the
mate can do right now; m8 sends back what the mate does, and its voice. Hook up any game that
can open a socket, plug in the models you like, write a persona.

> **Not a release.** This is a working setup shared as it is, for power users to pull, fork and
> bend to their own game and models. It runs natively on Linux or Windows; the checked-in
> configuration uses existing CachyOS and Unraid services. Windows PowerShell scripts are retained.

**Supported game: Halo: Combat Evolved**, as a mod of [OpenCE](https://github.com/OpenCommunityEdition/OpenCE),
the port of the Xbox game ([games/halo](games/halo)). More games can be added the same way. The persona that comes
with it is **Kevin**, a Spartan ([examples/kevin](examples/kevin)).

## How a turn goes

1. Something comes in: you said something (mic, or typed), or the game sent an event.
2. It goes in the feed: everything said and done, the mate's own actions too.
3. The decision model picks one of the actions the game offers (or none), and its target. An
   action can report back (a search: what it found goes in the feed, and the model picks again
   from what that offers), or need the player's view (the LLM looks at a frame from the game:
   says what's there, or picks one of the spots marked on the ground).
4. For the game's events, the decision model says whether the mate speaks. Your lines are always
   answered. After the mate stops talking, the game's events can't start it again until the
   cooldown (the game sends it).
5. The LLM writes the line, with how to say it ("[yelling over gunfire]") and sounds
   ("(laugh)"); the TTS says it, sentence by sentence; the game plays it.

Memory: the feed keeps about 16k tokens; past that, its oldest lines are folded into a summary
at a quiet moment or a save. The game's saves carry the memory: loading a save brings back what
the mate knew then.

## The models are endpoints

m8 runs no models of its own (bar Silero VAD, a 2 MB model that finds where speech starts and
ends in the mic's audio). Each job is a server it calls, set in `m8.toml`; run them where you
like, local or not. It speaks the APIs of what I used:

| Job | API m8 calls | Configured service |
| --- | --- | --- |
| Decide | llama.cpp's `/v1/systemone` (`[kev] url`, `model`) | Clef-Flash Artemis through Unraid llama-swap (`clef-flash-artemis`); m8 retains its existing scoring questions and newest 20 feed lines |
| Talk and see | OpenAI-style `/v1/chat/completions` (`[llm] url`, `model`), including image messages | Local llama-swap, Gemma 4 12B QAT HauhauCS Balanced Q4_K_M with MTP and vision (`gemma4-12b-mtp-vision`) |
| Speak | JSON `/v1/audio/speech` appended to `[mate] voice_server`, streaming PCM | [Breeze TTS API](https://github.com/remominor/breeze-tts-api) on Unraid, saved `soldier` profile |
| Hear | Multipart `/v1/audio/transcriptions` (`[stt] url`, `model`) | Parakeet Unified on Unraid (`parakeet`), 16 kHz mono WAV; no diarization or speaker identity |

Using something else (another API, half local and half hosted): each call is in one place to
change. Deciding is `Brain::kev` (`brain.rs`), talking `Brain::say`, seeing `Brain::look`,
speaking `Voice::stream` (`speak.rs`) and hearing `Transcriber::transcribe` (`listen.rs`).

## Plugging in a game

The game connects to m8 at `127.0.0.1:47800` (`[game] port`) and reconnects whenever m8
restarts. Frames are `[u8 kind][u32 big-endian length][payload]`:

| Kind | Way | Payload |
| --- | --- | --- |
| 1 | both | a JSON message |
| 2 | to the game | the mate's voice: mono 16-bit samples, at the `hello` message's rate |
| 3 | to m8 | dialogue the game played, to hear: `[u16 length][JSON: speaker, radio, react, sample_rate][mono 16-bit samples]` |
| 4 | to m8 | the player's view, when asked: `[u16 length][JSON: width, height, spots][RGB rows]` |

The game sends a `state` whenever things change (Kevin's sent one every tick):

```json
{"type": "state", "game": "Halo: Combat Evolved", "level": "a30", "in_combat": false, "on_radio": false,
 "state": "Kevin: unhurt, shields full, holding a pistol (12 rounds loaded), following {player}. {player}: close by ahead. Enemies: a grunt close (8 m) ahead.",
 "actions": [
   {"name": "follow", "description": "follows {player}"},
   {"name": "shoot", "description": "shoots at an enemy", "options": {"12": "grunt close (8 m) ahead"}},
   {"name": "search_weapons", "description": "looks around for weapons", "result": "shotgun (4 m) ahead",
    "then": [{"name": "pick_up", "description": "picks up a weapon", "options": {"88": "shotgun (4 m) ahead"}}]},
   {"name": "look", "description": "takes a look at something {player} wants him to see", "view": "describe"},
   {"name": "go_to", "description": "goes to a spot {player} can see", "view": "choose"}
 ]}
```

- `state` is read as words, so say it the way you'd tell someone. `{player}` becomes the
  player's name (`player_name.txt`).
- Each action is a line the decision model picks from ("Kevin follows Chasm"). `options` are its
  targets; `result` is what doing it finds, and `then` what can be done about that; `view` asks
  for the player's view (`describe`: say what's there; `choose`: pick one of its `spots`).
- m8 answers with `{"type": "action", "action": "shoot", "option": "12"}`.

Also from the game: `event` (`text`, and `react`: false to only note it), `saved` and `loaded`
(with the save's `id`), `direct` (a game master's line), `mic` (`muted`), `settings`
(`speech_cooldown`). From m8: `hello`, `say` (a line as it starts), `heard` (what you said),
`view` (send the view), `stop_audio`, `audio_end`.

## CachyOS / Unraid deployment

The patched Linux OpenCE game is already built at `~/Projects/m8/halo/build/linux/halo`,
with its extracted assets beside it. Keep that build and patch as they are. m8 does not start,
stop, or configure any model servers. Local llama-swap and the Unraid services are managed
independently.

The checked-in `m8.toml` uses:

| Setting | Value |
| --- | --- |
| `[mate] voice_server` | `https://tts.remominor.com` (base URL; m8 adds `/v1/audio/speech` once) |
| Voice / sample rate / seed / instructed guidance | `soldier` / `24000` / `42` / `4.0` |
| `[stt] url` / `model` | `https://stt.remominor.com/v1/audio/transcriptions` / `parakeet` |
| `[kev] url` / `model` | `https://llama.remominor.com/v1/systemone` / `clef-flash-artemis` |
| `[llm] url` / `model` | `http://127.0.0.1:9292/v1/chat/completions` / `gemma4-12b-mtp-vision` |

STT, System One, and LLM URLs are complete endpoint URLs. The voice is a saved server-side
profile (`clone:soldier` also works); do not supply a reference WAV or transcript. Delivery
instructions come from Kevin's existing bracketed notes and persist until the next note.
An uninstructed line omits `instructions` and uses guidance `1.0`.

Build the Rust application natively (Rust's pinned toolchain, a C/C++ toolchain, `pkgconf`,
and ALSA development files are required; on CachyOS/Arch, ALSA headers come with `alsa-lib`).
Only the small Silero VAD model is needed locally; paths are relative to `m8.toml`:

```bash
cd ~/Projects/m8
mkdir -p models
# Once, if this file is missing:
curl -fL https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx -o models/silero_vad.onnx
cd m8
rustup component add rustfmt
cargo fmt
cargo test
cargo build --release
```

Optionally put your name in `~/Projects/m8/player_name.txt`. Ensure the independent services
are reachable, then start m8 before the existing game in separate terminals:

```bash
# Terminal 1: Rust application, with the repository-root configuration
cd ~/Projects/m8
./m8/target/release/m8 ./m8.toml
```

```bash
# Terminal 2: existing Linux OpenCE executable and assets
cd ~/Projects/m8/halo/build/linux
./halo
```

The game connects and reconnects to `127.0.0.1:47800`; the socket is never bound to the
network. Use headphones for barge-in. **M** toggles the game's microphone mute; typed input
in m8's terminal can exercise dialogue without a microphone.

Microphone capture runs in **m8**, through the desktop's default input. In KDE **System
Settings → Sound → Input Devices**, select the microphone and make it the default (the
default-device button in the volume applet also works). m8 follows that default through
PipeWire/WirePlumber, including changes while running, so an external microphone needs no
device name in `m8.toml`. Keep applications on the default input instead of assigning m8
a fixed device in an audio mixer. If no microphone is connected at launch, m8 retries every
two seconds; capture-stream errors also trigger reopening the current default.

For debugging:

- Open `http://127.0.0.1:8090` or `http://<host-ip>:8090` from the LAN; `/history`
  is available at either address. The brain web server binds to `0.0.0.0:8090`.
  Model requests, errors, first-token and
  first-audio timings are also saved under `logs/`. Logs and model assets stay untracked.
- Check local and remote `/v1/models` for the exact configured routing IDs. Clef must
  support `/v1/systemone`; chat completions are not a substitute for its decisions.
- Breeze `/health` may return **503 while unloaded**. m8 initializes at the configured
  sample rate without calling it; synthesis loads the model on demand. Actual synthesis
  failures remain visible in stderr and history. HTTP 409 is retried up to twenty times.
- Breeze must return raw PCM with `X-Sample-Rate: 24000` and `X-Sample-Format: s16le`.
  Missing or mismatched headers fail the line before audio reaches the game. The configured
  sample rate is also sent in the game's `hello`; do not change it independently of the server.
- Test STT with `./m8/target/release/m8 ./m8.toml --transcribe /path/to/mono.wav` from
  the repository root, or use `--simulate-mic` to include local resampling and Silero VAD.
- Check microphone permissions/input selection and the VAD model path if `[mic]` reports
  an error. No local Parakeet model, Sortformer, reference voice audio, or transcoding is needed.
  If `snd_pcm_hw_params` fails, check `wpctl status` for a default (`*`) under **Sources**,
  and check KDE for a connected input. A motherboard input can be listed while all its jacks
  are unplugged; PipeWire then has no usable default and reports `no target node available`.
  Select a connected input in KDE, or use `wpctl set-default <source-id>` (IDs can change).
  Check the host's ALSA/PipeWire default capture device:
  `arecord -D default -d 1 -f FLOAT_LE -r 44100 -c 2 /dev/null` should work independently
  of m8. A failure there is a host capture issue; m8 keeps retrying, and typed input and file
  transcription still work. [WirePlumber's default-device policy](https://pipewire.pages.freedesktop.org/wireplumber/daemon/configuration/settings.html#linking-follow-default-target)
  controls live switching (`linking.follow-default-target`, enabled by default).
- Mock Rust tests use loopback HTTP and a simulated game; they do not prove live GPU
  compatibility. Validate dialogue, vision, decisions, transcription, and audible game playback
  against the running services when diagnosing deployment issues.

## Playing it in Halo (original Windows setup)

The mod is m8's changes to OpenCE ([games/halo](games/halo)); you bring the models (Setup,
below) and your own copy of the game.

1. Get the game and put the mod on it: `games/halo/setup.ps1` clones OpenCE into `halo/` and
   applies the changes.
2. Build it, on Windows, in `halo/` (Python, [ninja](https://ninja-build.org/) and LLVM with
   clang 22 or later):

   ```bash
   python configure.py --release
   ninja windows
   ```

   That makes `halo/build/windows/halo.exe`.
3. The game data: start `halo.exe` once and give it an Xbox disc image of Halo: Combat Evolved,
   as OpenCE's README ("Game data") says; it extracts the maps.
4. Set up m8 (Setup, below), then run `start-all.ps1`: it starts the models, m8 and the game,
   which connects to m8 by itself.
5. In a campaign level the teammate appears beside you. Talk to him; **M** mutes the mic
   (an icon shows it); typing `kevin <text>` in the game's console gives him a line as a game
   master; **KEVIN SETUP** on the profile menu sets his speech cooldown, his volume beside you
   and over the radio, and how far off he goes to the radio.

What the mod changes: [games/halo/README.md](games/halo/README.md).

## A persona

A persona is a folder (`persona` in `m8.toml`; `name` is what the mate is called):
`system_prompt.txt`, and if you like `combat_prompt.txt`, `radio_prompt.txt` (added while the
state says `in_combat` or `on_radio`) and `levels/<level>.txt` (the story so far, by the state's
`level`). See [examples/kevin](examples/kevin).

## Original model-server setup (Windows / WSL)

These historical instructions describe the original local servers. The current voice client
requires the saved-profile JSON API above; the old multipart-only Breeze patch is insufficient.
The existing Windows scripts are unchanged; use them only with a matching configuration and
compatible server. CachyOS/Unraid users should follow the deployment section above.

You need an NVIDIA GPU (the three servers took about 25 GB of video memory, on an RTX 5090 with
32 GB), a CUDA build of [llama.cpp](https://github.com/ggml-org/llama.cpp) with `/v1/systemone`
([pull request 29818](https://github.com/ggml-org/llama.cpp/pull/29818); this ran on build
b11580) in `llama.cpp/`, WSL with Ubuntu for Breeze, and Rust (rustup; `rust-toolchain.toml`
picks the version).

1. The models, into `models/` (the scripts and `m8.toml` expect these names):

   ```bash
   hf download ggml-org/Kev-4B-GGUF Kev-4B-Q8_0.gguf --local-dir models/kev-4b
   hf download HauhauCS/Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-Q4_K_M.gguf mmproj-Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-BF16.gguf --local-dir models/gemma-4-12b-hauhaucs
   hf download BreezeBlue/breeze-tts-2 --local-dir models/breeze-tts-2
   ```

   And from sherpa-onnx's [asr-models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models)
   release: `silero_vad.onnx` into `models/` (m8's VAD), and, for the speech-to-text server,
   `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2` unpacked into `models/`.

2. Breeze, in WSL: clone breeze-tts into `~/breeze-tts`, set it up as its README says (its
   `.venv`), then apply this repo's patch, which encodes the voice clip once instead of with every
   line:

   ```bash
   git -C ~/breeze-tts am /mnt/<path to this repo>/patches/breeze-tts-reference-cache.patch
   ```

3. A voice clip and its transcript in `voices/` ([voices/README.md](voices/README.md)).
4. Your name, as the mate calls you: one line in `player_name.txt`.
5. Build m8 (and `stt-server`, beside it):

   ```bash
   cd m8
   cargo build --release
   ```

Then `start-all.ps1` starts it all, each in its own window: the decision model
(`start-kev.ps1`), the LLM (`start-chat.ps1`), Breeze (`start-breeze.ps1`; its first start takes
about 4 minutes), speech to text (`start-stt.ps1`), m8 once they're up, and the game if it's
built in `halo/`. Change the scripts for your own servers and game.

## Watching it

- **Brain page**, http://127.0.0.1:8090 (LAN: `http://<host-ip>:8090`): the prompt as the models get it, every decision and
  line as it happens, buttons to send test events, and the persona's prompts to edit.
- **History page**, http://127.0.0.1:8090/history (LAN: `http://<host-ip>:8090/history`): every model call of a session, exactly what
  went in and what came out, with the pictures the mate saw. Each session is kept in `logs/`.
- Type a line in m8's window to say it without a mic.

## The code

`m8/src`: `brain.rs` (the feed, decisions, the LLM, the mate's eyes, memory), `listen.rs` (the
mic, Silero, speech to text), `speak.rs` (the voice), `link.rs` (the game), `site.rs` (the
pages), `picture.rs` (the view's numbered spots), `config.rs` (`m8.toml`), and
`bin/stt-server.rs` (Parakeet behind the transcriptions endpoint).

## Licence

MIT. The models, Breeze and llama.cpp have their own licences.
