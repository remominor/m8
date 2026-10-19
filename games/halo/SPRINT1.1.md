# Sprint 1.1: intent, latency and navigation cleanup

Baseline: m8 `894a6d5`, OpenCE `8102e711c353068d22d62efe6bf67ebc8a90a22e`.

## Behavior and interfaces

Clef owns semantic interpretation of the current player utterance. There is no literal phrase
list or regex guard. One `choice` selects `none`, `follow`, `stay`, `go_aim`, `hold_fire` or
`resume_fire`; unsupported/malformed answers resolve to `none`. The question distinguishes
indirect requests from conversation, questions about behavior, quotations, hypotheticals and
ambiguous/conflicting instructions. Native context validation, terrain/reachability validation,
independent persistent orders and acknowledgements remain the execution boundary. Natural
language quality still depends on Clef; mock tests prove routing, not classification accuracy.

`[kev].timeout_ms` defaults to **5000** and must be positive. The separate System One HTTP
agent uses that overall request timeout and a connect timeout of at most 1000 ms. No retry is
added. Intent failure sends no command and adds failure context for the normal player dialogue;
event decision failure skips optional speech. Successful and failed calls retain request,
response/error and elapsed time in history, including confidence/probabilities if returned.
The dialogue, vision and summary agent keeps its 60-second timeout. The HTTP library's system
DNS resolver cannot be interrupted; an OS resolver stall can exceed its overall timeout.

The existing sequential brain and FIFO input channel retain every player message with its
original context. Event batching preserves the first following non-event input. Up to **64**
ordinary game events may be outstanding, including the active batch; additional events are
dropped and counted in a history diagnostic. This admission limit is shared by native events,
transcribed game dialogue and web event injection. Player input and lifecycle messages bypass
that limit. A delayed command may expire in OpenCE after 30 simulation seconds or on lifecycle
changes; processing never recaptures the player's current aim. Native rejection is included
in dialogue instead of claiming execution. Barge-in and speech cooldown behavior are retained.

Conversational `in_combat` now requires a living hostile in the existing enemy list that is
visible to Kevin and within **50 metres**. Dead Kevin reports false. Tracking range and weapon
targeting ranges are unchanged. The web interface still listens on **0.0.0.0:8090**.

## Navigation safety and measurements

Every moving tick checks the actual body footprint's ground support, immediate next-step
support/BSP continuity, and a sweep of the physical capsule from the actual body position.
This grounded sweep retains standing body height and avoids the expanded maneuver capsule
overlapping its own supporting floor. Jump/location validation keeps its conservative clearance.
Detailed footprint/BSP braking corridors can be reused for at most three ticks (10 Hz at the engine's 30 Hz simulation) when the entire current
stopping line remains inside the validated strip. The cache includes three ticks of travel
lookahead, the original footprint margin, ground heights and surface/waypoint/path/order/recovery
identity. Changed geometry eligibility, turns, lateral displacement, longer braking distance,
expiry or insufficient remaining corridor causes immediate validation. Invalid support or
obstructions reject movement and invalidate the cache. Jumps use fresh detailed landing and
clearance checks; cache results never authorize a jump. Replanning, orders, jump/airborne
transitions, recovery and relocation invalidate or fail cache eligibility. No direct fallback
movement or broadly relaxed slope/drop limits were introduced.

The same deterministic fake-world straight run was executed for 90 ticks at 0.075 world units
per tick using both the Sprint 1 source and the new source:

| Operation | Sprint 1 | Sprint 1.1 |
| --- | ---: | ---: |
| Ground collision probes | 3,600 | 1,440 (60% fewer) |
| BSP trajectory queries | 720 | 270 (62.5% fewer) |
| Full corridor validations | 180 (two coincident directions per tick) | 30 (10 Hz) |
| Cache reuse | 0 | 60 |
| Capsule sweeps | 0 | 90 (immediate obstruction check) |

These are query counts, not CPU timings. Complex terrain, different velocity/heading directions,
frequent surface transitions or cache misses require more work. Capsule sweeps have a different
cost from ground probes; gameplay profiling must establish the resulting simulation time.

With `HALO_DEBUG_COMPANION=1` / `debug.companion = true`, `Kevin nav queries:` aggregates full
corridors, ground probes, clearance checks, BSP queries, path searches, reuse and rejections
at most once per second. These counters do not enter conversational memory.

Reproduce the comparison using a clean checkout with the Sprint 1 patch applied as the baseline:

```sh
python halo/tools/harness/tests/test_companion.py \
  --baseline-source /path/to/sprint1/port/linux/game/companion.c
```

The archived function set and current function set are both extracted from actual source and
compiled against the same harness motion/collision fixtures. The baseline's printed `full=0`
means that old code had no counter; its four samples on each of two corridors establish the
180 validations. The current stable-run test requires at least 50% fewer ground probes.

## Changed files

Behavior changes are in `m8/src/brain.rs`, `config.rs`, `main.rs`, `site.rs`, `m8.toml`, and
OpenCE `companion.c`, represented in `games/halo/m8.patch`. Both native companion harnesses
and their Python drivers were extended. README/Sprint notes document the changes. The requested
`cargo fmt` also formats existing code in `link.rs`, `listen.rs`, `picture.rs` and `bin/stt-server.rs`;
those files have no behavior changes.

## Verification and remaining gameplay work

Completed locally: **33 Rust tests and 392 native harness checks passed**; Rust and Linux
OpenCE release builds succeeded. Formatting and whitespace checks passed. The refreshed patch
applied to a separate clean pinned checkout, and all **44** resulting files matched the working
sources byte for byte, including the existing binary assets.

Rust tests cover natural intent routing without the phrase guard, malformed output handling,
config defaults/overrides/zero rejection, preserved response probabilities, actual brain-loop
buffering during delayed Clef and Gemma calls, timeout recovery, native acknowledgements,
stale contexts, event batches/flood limits and an event following a newer player order. The
Gemma delay test exceeds the test Clef budget and still completes. All HTTP/native peers are
loopback mocks; no deployed model, microphone or game is needed.

Native cases cover stable movement and ramps, turns, sudden headings, cliffs at several speeds,
cache boundary/expiry, changed paths/waypoints/surfaces, actual versus stale projected position,
new obstruction/support loss, physical floor contact, fresh safe/unsafe jumps, and all requested combat-state cases.
Negative controls must fail when corridor containment, cliff validation or existing retry,
path-failure and target-visibility protection is deliberately removed.

Build/test commands remain:

```sh
cargo fmt --manifest-path m8/Cargo.toml
cargo test --manifest-path m8/Cargo.toml
cargo build --release --manifest-path m8/Cargo.toml
python -m pytest -q halo/tools/harness
cd halo
python configure.py --release
ninja linux
```

The refreshed portable patch preserves previous source additions and binary assets. Validate
it in a separate clean pinned checkout and compare all patched files with the working sources;
never reset the working game installation or replace extracted assets.

Use the existing [Pillar of Autumn checklist](SPRINT1.md#pillar-of-autumn-gameplay-checklist--pending-user-testing)
for movement/combat acceptance. Also check natural spoken command variations, ambiguous conversation,
LAN chat/history, response latency and optional navigation counters on ordinary corridors,
doors, ramps and near ledges. Gameplay acceptance and actual map CPU timings remain pending.
Gameplay logs are inspected only when explicitly requested.
