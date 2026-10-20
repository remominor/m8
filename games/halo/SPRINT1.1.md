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

## Doorway transition correction after gameplay

The first live Pillar of Autumn report showed Kevin stopped at a supported corridor doorway.
The terminal recorded `traverse=5 build=5` (successful path search/build), followed by
`state=4` (terrain rejected) at goal `(-30.12, 36.45, 0.01)`, about 0.67 world units away.
The old diagnostics did not identify the individual failed probe, and the OS denied debugger
attachment, so the exact original rejection branch was not captured.

The safety controller now retains the collision face identified by each ground ray. It passes
both known endpoint faces directly to Halo's `structure_test_line2d`, which handles boundary
cases that the former unknown-destination surface lookup could reject. The actual supporting
face also anchors validation when the biped's cached pathfinding face belongs to the other side
of the seam. Both faces must remain marked walkable, and the connection must reach the sampled
point with matching elevation. Existing footprint, slope/drop, collision, cache-containment and
jump checks remain in force. Navigation rejection messages now include the failed check and
probe coordinates to distinguish missing/steep support, footprint elevation, BSP transition
and physical obstruction on the next run. Native messages are written to the game's terminal
stderr; `debug.txt` does not capture these platform messages.

A synthetic doorway regression failed against the old controller before the change. New cases
cover supported doorway crossing, following across that threshold, actual BSP obstruction,
nonwalkable faces, and a cliff beyond the doorway; a negative control removes the known target
face. **396 native harness checks pass**. Stable-run query counts remain 1,440 ground probes and
270 BSP queries over 90 ticks. The Linux release target was rebuilt, and the portable patch was
verified against a separate clean pinned checkout with all 44 resulting files matching. The
previous executable is preserved at `halo/build/linux/halo.before-corridor-fix`.

Replay the reported doorway after launching the rebuilt game to verify this correction in the
actual map. The harness reproduction is not a claim that the live doorway has passed yet.

## Missing biped support cache correction

The next live run reached `(-29.921, 30.021, 0.007)` and repeatedly logged
`safety=current support surface`, with successful path results. This reason identifies a
short-circuit rejection on a missing cached biped support/navigation index: the footprint rays
had not run. The preceding seam correction addressed a later check and did not resolve this
blocker. Two regression cases reproduced that rejection before changing the cache guard.

Navigation now derives its start from a fresh five-point structure-ground footprint and the
hit face's walkable navigation flag. A missing biped cache is neither proof of unsafe ground
nor permission to move: the actual rays, slope/drop checks, BSP connections and physical
capsule sweep must still pass. Path searches and goal-surface lookup use this same verified
start, so they do not reintroduce the missing-cache failure. Cached indices can still invalidate
previously validated corridors when they change, but cannot prevent fresh validation.

Exhausted terrain recovery checks local safety again at most every five simulation seconds.
It resumes path search only when that check succeeds; a persistent cliff still idles without
launching searches. Missing ray hits and steep/invalid hits now have distinct diagnostic reasons.
The navigation harness extracts the real start/goal-surface helpers instead of stubbing them.
New tests cover missing physical support, missing navigation cache, both missing while following,
actual cliffs/missing ground, safe recovery wakeup and bounded rejection while unsafe. A negative
control reinstates the old support-cache guard and must fail.

**402 native checks pass**, the Linux release target builds, and the refreshed portable patch
again applies to a separate clean pinned checkout with all 44 files matching. Stable-run counts
remain 1,440 ground probes and 270 BSP trajectory queries per 90 ticks. The executable from the
previous run is preserved at `halo/build/linux/halo.before-support-cache-fix`. The player
subsequently confirmed this correction fixed that case. The Rust release binary was also
rebuilt to include the requested unsolicited-speech prompt.


## Wall-side footprint correction

The following live report repeated successful path results followed by `no structure hit under
footprint` at `(-51.749, 35.970, 0.000)`, with a goal about 0.67 world units away. Read-only
inspection of the extracted a10 collision geometry established that this ray lies inside a
solid wall, not over an exposed floor edge. In BSP 3, floor face 71 ends at y=36.0287; vertical
base face 1112 rises there to z=0.1, bevel face 1579 meets wall face 39 at y=35.9787, z=0.15,
and that wall continues to z=1.0. The five-point footprint's extra margin could enter this wall
and incorrectly stop an otherwise supported corridor trajectory. The harness reproduced this
rejection with the logged probe coordinates and the adjacent wall/base dimensions before the fix.

Only an outer footprint sample can now use the wall case. It requires front-facing, near-vertical
fixed structure hits between the supported center and the edge at both foot and standing-body
heights, with consistent normals. A fresh ground ray immediately on the near side of the wall
must also pass the existing support/slope/elevation checks; a wall across a gap cannot supply
support. The center always needs actual walkable floor. Existing BSP continuity, capsule
clearance, stopping corridor/cache bounds and jump checks remain mandatory. These extra rays
run only after an outer support probe fails. Debug query totals include `wall_probes`, and
rate-limited rejection diagnostics now identify the validation stage, origin and waypoint.

New cases cover the reported geometry, repeated movement beside a wall, exposed edges, low
curbs, object/back-facing/invalid wall hits, unsupported centers, a gap before a wall, actual
body obstruction, and unsafe jumping. Negative controls remove the wall handling and the
near-wall ground requirement and must fail. **410 native harness checks pass**. Stable 90-tick
movement remains at 1,440 ground probes and 270 BSP queries (60% and 62.5% below the Sprint 1
baseline respectively), with 30 full validations, 60 cache reuses and 90 capsule sweeps.

The portable patch applies to a separate clean pinned checkout with all 44 files matching,
including all 39 original patched files and previous binary assets. The Linux release target
is rebuilt; the previous executable is preserved at `halo/build/linux/halo.before-wall-footprint-fix`.
Replay after relaunching Halo is still needed to confirm the live result. The currently running
game was not restarted or modified in memory.
