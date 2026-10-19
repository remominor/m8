# Sprint 1: native companion controller

Kevin follows and fights inside OpenCE without waiting for Clef. Movement orders and fire
permission are independent. Spawn, respawn, checkpoint load and level load restore follow
with fire enabled; reconnecting m8 preserves the live native orders.

## Player commands

Use the microphone, m8 console, web chat, or the game's `kevin <text>` console command.
Clef gets one focused intent choice. A conservative phrase guard also requires the selected
intent to match the utterance. Unsupported, conditional, quoted and compound requests leave
orders unchanged. Common forms (with optional Kevin/Kev, please, or can you):

| Intent | Examples | Effect |
| --- | --- | --- |
| `none` | Ordinary conversation; “maybe go somewhere” | Preserve both orders |
| `follow` | “Follow me”, “Come with me” | Follow Chief |
| `stay` | “Stay here”, “Hold position” | Hold the current position |
| `go_aim` | “Go there”, “Move where I'm aiming” | Move to captured, validated ground and remain there |
| `hold_fire` | “Hold fire”, “Stop shooting”, “Don't shoot” | Disable firing, preserve movement |
| `resume_fire` | “Resume fire”, “Fire at will” | Enable firing, preserve movement |

Aim is captured at the next game tick when an utterance enters m8, before the brain queue or
Clef classification. The native console captures before forwarding its text. Changing aim
while Clef works does not change that destination. Requests expire after 30 simulation seconds;
a load, respawn or reconnect invalidates captured requests. Paused/unavailable games can time
out. Dialogue uses the native acknowledgement and reports uncertainty after a missing reply.
A rejected location leaves the current safe orders and active path intact.

## Navigation and combat limits

- Follow uses a stable trailing goal and approximately two/four metre stop/resume distances.
  Existing valid paths continue while a new destination waits for the search budget.
- BSP paths contain up to four waypoints. Consuming a partial segment schedules continuation.
  There is no direct-to-destination movement fallback after a failed search.
- Full path searches are limited to one per half second. Failed routes back off to one, two,
  then five seconds. Progress stalls trigger revalidation and deterministic alternative goals;
  exhausted recovery waits for a changed destination, relocation or reopened obstruction.
- Ground probes cover the body footprint and a braking corridor in both requested movement
  and current velocity directions. Missing ground, unsafe slopes and abrupt drops stop movement.
- Automatic jumping is deliberately limited to low obstructions over continuous structure
  ground. Landing support, clearance, the engine-derived jump envelope and runout must all pass.
  No gap jumping, parkour, arbitrary scenery landings or route-wide safety guarantees.
- Catch-up remains a last resort beyond 40 world units (about 122 metres), after at least ten
  seconds of separation and failed recovery, with a 30-second cooldown. Only follow can use it.
- Combat acquires nearby visible hostiles, retains usable targets, stops firing immediately on
  occlusion, and replaces unusable targets. An occluded target has at most 0.75 seconds of tracking
  grace when no visible alternative exists. Target selection never changes the movement order.
- Aim is smoothed without fresh random error each tick. Halo's weapon spread remains. Trigger
  metadata distinguishes latched firing from automatic bursts; existing automatic reload remains.
- Firing checks obstruction and allies along both the intended line and actual aim with a spread
  margin. This is conservative direct-fire safety, not projectile prediction or splash planning.
  Rockets, fuel-rod/plasma cannon and flamethrower profiles do not fire autonomously.
- Legacy special actions and vision helpers remain in source but are outside routine decisions.
  No actor AI, cover tactics, grenades, vehicles, scavenging, mission planning or model changes.

## Diagnostics

Significant transitions use OpenCE's existing `halo.log` facilities with `Kevin nav:`,
`Kevin combat:`, `Kevin order:` and `Kevin command:` prefixes. Navigation summaries include
order, coordinates, path status, distance, failure count and recovery stage, at most once per
second. Combat messages include full target ID and distance in metres. Numeric navigation
states: 0 idle, 1 valid, 2 blocked, 3 unreachable, 4 invalid surface.

Enable optional details with `HALO_DEBUG_COMPANION=1` when launching the game, or
`debug.companion = true` in its existing config. The default is false. Orders and meaningful
native notices appear in m8 history. Navigation notices are never added to Gemma's memory or
sent to Clef for a speech decision. Command results do enter dialogue context.

## Automated validation

Local results: **376 OpenCE harness checks and 26 Rust tests passed**. The Linux release
target and Rust release binaries built successfully. The regenerated patch applied to a clean
pinned checkout, and all 44 resulting files matched the working sources byte for byte. Gameplay
acceptance remains pending the checklist below. The original game executable is preserved at
`halo/build/linux/halo.before-sprint1`.

From the project root, with pytest available in an isolated Python environment:

```sh
python -m pytest -q halo/tools/harness
cargo test --manifest-path m8/Cargo.toml
cargo build --release --manifest-path m8/Cargo.toml
```

Then build OpenCE using its existing workflow:

```sh
cd halo
python configure.py --release
ninja linux
```

Native harness tests compile the real controller functions with fake path/collision results.
They cover moving follow, search failure/backoff, partial paths, progress, slopes/cliffs, capsule
occupancy, low-obstacle jump safety, teleport restrictions, targeting, friendly fire, captured
locations, parsing, lifecycle invalidation and independent orders. Negative controls deliberately
break safety/visibility/retry behavior and must fail. Rust tests use loopback native/model mocks;
they cover focused classification, command guards, acknowledgements, timeouts and existing voice,
vision and memory behavior. These checks do not prove gameplay acceptance.

The portable `m8.patch` includes all existing m8 additions, binary menu assets, controller
changes and harness fixtures. Its base remains OpenCE `8102e711c353068d22d62efe6bf67ebc8a90a22e`.
Patch checking must use a separate clean checkout, never reset or clean the working installation:

```sh
git -C /path/to/clean-pinned-opence apply --check /path/to/m8/games/halo/m8.patch
git -C /path/to/clean-pinned-opence apply /path/to/m8/games/halo/m8.patch
```

## Pillar of Autumn gameplay checklist — pending user testing

Launch the rebuilt game and m8 through the existing workflow. Start with the normal pistol or
assault rifle. Run movement checks before combat checks; use “Hold fire” to isolate movement, then “Resume fire”
for combat. Ordinary movement/combat must continue
with Clef unavailable; conversation needs its existing services. Keep aim on safe ground until
speech transcription completes for a location command.

| Area | Scenario | Expected observation |
| --- | --- | --- |
| Movement | Walk an unobstructed corridor | Follows automatically with stable spacing |
| Movement | Turn corners | Follows BSP waypoints without running at the wall |
| Movement | Pass ordinary doorways | Enters without persistent wall contact |
| Movement | Run down a corridor | Full running pace; no recurring replan pauses or growing straight-line gap |
| Movement | Block a route, then reopen it | Bounded recovery/safe idle; resumes after the obstruction clears |
| Movement | Present a low obstacle on continuous safe floor | Jumps only when clearance, landing and runout pass; a conservative refusal is possible |
| Movement | Approach an unsafe drop or obstructed landing | Stops; no jump or movement over the edge |
| Movement | Order “Stay here” and move Chief away | Holds the recorded position; no catch-up teleport |
| Movement | Order “Follow me” | Resumes following |
| Location | Aim at safe floor, say “Go there”, then change aim | Uses the captured location and stays there |
| Location | Aim at sky, a wall, cliff edge or inaccessible floor | Refuses and preserves previous safe orders |
| Combat | Nearby visible grunt | Acquires, aims, reacts and fires automatically |
| Combat | Distant invisible elite and closer visible grunts | Chooses visible usable enemies; never locks onto the invisible elite |
| Combat | Kill or invalidate the target | Switches or clears it without indefinite retention |
| Combat | Remove all valid targets | Releases trigger |
| Combat | Empty a magazine with spare ammunition | Reloads automatically |
| Combat | Follow during an encounter | Moves and fires independently |
| Combat | Stay during an encounter | Defends the position without rushing/strafe tactics |
| Combat | Chief or a marine crosses the firing line | Stops shooting while the line/cone is unsafe |
| Combat | “Hold fire” while following or staying | Stops firing and preserves movement |
| Combat | “Resume fire” | Resumes autonomous combat and preserves movement |
| System One | Inspect request history after routine play | No routine action/target/obstacle questions; event requests only use `speak` `noul` |
| System One | An event during/after speech cooldown | Existing speech relevance and cooldown behavior |
| System One | Ordinary conversation | Dialogue continues; both gameplay orders remain unchanged |
| System One | Explicit commands through each input route | One focused intent choice; native acknowledgement; correct order dimension changes |
| System One | Ambiguous, quoted, hypothetical or compound commands | No unrelated action or silent order change |
| Lifecycle | Respawn, checkpoint/level load, reconnect | Defaults on respawn/load; live orders persist on reconnect; stale commands refused |

Record observations and give instructions before log inspection. No gameplay log monitoring is
part of this implementation. Next sprint should tune the movement/jump thresholds against these
observations and expand deterministic collision fixtures before adding tactical breadth.
