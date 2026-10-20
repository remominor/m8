# Kevin and native ground movement

## Comparison baseline

OpenCE `8102e711c353068d22d62efe6bf67ebc8a90a22e`, the revision pinned by
`setup.ps1`. Engine references below were read locally and checked against
`git show HEAD:<path>`. Kevin references describe the working controller after
the doorway, support-cache and wall-footprint fixes, before this corrective pass.
Those uncommitted changes and the separate speech prompt are preserved.

## Priorities and proposed changes

| Priority | Finding and source locations | Minimal change |
| --- | --- | --- |
| 1 | `actor_moving.c:2103` `actor_destination_update` advances intermediate steps by proximity, completion, or a passed next-leg projection while movement is thwarted. Kevin's `companion_walk_direction` (`companion.c:2088` before this pass) only advances within 0.2 units. A missed point can turn him backward and invalidate an otherwise usable path. | Adapt a normalized next-leg projection with a bounded lateral tolerance. Only skip a passed point if a fresh native `structure_test_pill2d` test permits the shortcut. Keep center connectivity and all ordinary direction safety checks. Do not skip final or partial-path endpoints as if they were intermediate corners. |
| 2 | Native `actor_destination_update` refreshes paths on actor timeslices (`actor_moving.c:2108`); `path_state_build_path` (`path.c:837`) smooths and then calls `path_avoid_obstacles`. Kevin invokes that same chain only on initial/dirty/invalid routes, so a new biped or door can invalidate a stationary goal's old detour. | On physical obstruction, refit the remaining steps with public `path_avoid_obstacles`, using a freshly grounded start and a separate half-second attempt budget. This reuses the native disc/tangent search without another full BSP route search. Commit a successful, nonempty result atomically; apply all direction safety checks before moving. Retain existing backoff/recovery if it fails. |
| 3 | Kevin's `companion_validate_corridor` (`companion.c:1653`) checks the stopping corridor plus three ticks of optional travel for cache reuse. It currently treats failure anywhere in that padding as failure of the immediate movement. Native movement has destination tolerance and stopping-distance control (`actor_moving.c:1110`, `:2644`); it does not demand a straight continuation beyond every route point. | Accept only the already verified prefix when it still contains the **entire** required stopping corridor plus the cache end margin. Store the shorter actual extent; never cache the failed segment. This removes an unnecessary rejection without shortening the required braking line. |
| 4 | `actor_move_calculate_free` (`actor_moving.c:2931`) projects movement into facing axes. Kevin does that too (`companion_control`, `companion.c:2428`). But his player-physics biped scales these axes with different speeds (`bipeds.c:3817–3855`). Aiming sideways/backward therefore changes the world movement direction. | Compensate throttle components using the engine's applicable forward/backward/side speed settings before limiting their magnitude to the requested pace. Keep desired facing, aim, firing and orders independent. Verify world displacement while aiming at several headings. |
| Later | A fixed minimum straight lookahead can still extend past a close corner; final goals get no explicit throttle taper. Native actor stopping constants are for animation-driven actors and vehicles, while Kevin uses player physics. | Measure remaining failures in gameplay before changing braking tolerances or implementing a speed/turn controller. Do not copy actor constants or shrink cliff protection merely to pass a doorway. |

## Facilities Kevin already uses

- `path_input_new`, `path_input_set_start`, `path_state_new`,
  `path_state_destination`, `path_state_find`, `path_state_build_path`:
  the native BSP graph search, not a separate planner.
- `path_smooth` and `path_avoid_obstacles` indirectly through path building.
  `path_smoothing.c` supplies radius-aware corner/tangent points.
- `obstacles_get_discs_in_sphere` (`path_obstacles.c:178`) indirectly through
  obstacle avoidance. This excludes the source, invisible/dead objects,
  non-obstacles and suitable open machines, reads collision-model pathfinding
  spheres, and marks fast forward-moving bipeds optional.
- `obstacles_test_pill` (`path_obstacles.c:356`), `obstacles_disc_tangents`
  (`:395`) and `path_find` (`path_obstacle_avoidance.c:860`) indirectly. The
  local native search tries tangents around inflated discs, validates links
  against BSP navigation, and can retry without optional discs.
- `structure_surface_index_from_point` for goal projection and
  `structure_test_line2d` with known endpoint faces for ground continuity.
- Structure-only collision ground rays, floor normals/navigation flags,
  `biped_get_physics_pill`, `collision_move_pill` and existing feature queries
  for support and body clearance. Limited jumping already requires verified
  landing, continuous ground, standing clearance and an engine-derived envelope.
- `unit_control` with in-combat animation state. The alert base seat actually
  suppresses strafing and running (`bipeds.c:3835`); that was already corrected.

The local port adds `path_obstacle_search_reach` to the pinned engine's obstacle
collection (pinned default: 4 units; Kevin temporarily requests at most 16).
Preserve this existing integration, including restoration after companion calls.
Do not change ordinary NPC behavior.

## What cannot be reused directly

`actor_path_refresh`, `actor_destination_update`, `actor_move_update` and
`actor_get_stopping_distances` all read actor data, definitions, destinations,
timeslices, movement completion and animation controls. Kevin has no such actor.
Calling them with his unit index or fabricating partial actor data is invalid.

`actor_move_vector_avoidance` and its weighted sense/avoidance ray fan are used
by directional flying vehicles (`actor_moving.c:3060`), not ordinary ground
bipeds. Importing that entire mechanism would add the wrong dependencies and
unnecessary collision queries. For ground actors, the reusable local machinery
is the path obstacle/disc solver and path refresh, not that ray fan.

`movement_thwarted` in `actor_move_calculate_movement` is set when facing does
not permit movement; it is not a collision-contact or stuck detector. Kevin's
requested-motion versus displacement/progress tracking remains necessary.
Native actors' occasional path refresh also is not evidence of a generic native
stuck-recovery ladder. Preserve Kevin's bounded recovery and last-resort catch-up.

## Safety and overhead observations

Five independent footprint rays can enter a wall or disagree at triangle seams;
those are confirmed sources of prior false rejection. The existing wall case
now needs two fixed-wall hits plus support immediately before the wall. Preserve
that protection rather than deleting edge support. A successful 2D pill query
alone is insufficient cliff proof: the pinned implementation has permissive
endpoint behavior and can leave parts of its output unset when a side has no
starting face (`path_structure_bsp.c:596`). Use its boolean obstruction result
only as an additional geometric gate, never as replacement ground authorization.

The per-tick center/footprint support, known-face BSP continuity and immediate
physical sweep overlap some path geometry work, but serve different purposes:
actual body position, new missing support, slope/drop checks and moving object
collisions. No broad removal is justified by this source comparison. Existing
three-tick corridor caching must retain full spatial containment and immediate
support/obstruction checks. Optional padding can be shortened only to verified
samples beyond the whole existing braking line.

Do not full-search every tick. Count waypoint shortcuts/local refits separately
from full path searches, and aggregate optional diagnostics once per second.
Local refits may produce a four-step partial route; preserve `steps_finish_path`
and continue through the normal route builder when it ends.

## Verification plan

Use extracted controller functions with deterministic collision/path fixtures
for narrow corridors, turns, doors, ramps, dynamic blockers, missed waypoints,
unsafe shortcuts, short validated cache extents and firing-facing movement.
Include negative controls that reinstate the old failures. Native route/physics
integration and actual a10 performance still require gameplay; a fake-world pass
must not be presented as proof that every map doorway works.

Build the existing Linux target, run the native harness, refresh the binary-safe
portable patch and apply it in a separate checkout of the pinned revision.
Do not restart the user's game, inspect new gameplay logs without a request,
change models/web/voice, or make ordinary recovery teleport more often.

## Implemented corrections

- `companion_advance_waypoint` (`companion.c:1798`) adds the bounded next-leg
  projection and known-face center/pill gate. Failed shortcut geometry is retried
  at most every three ticks. Final/partial endpoints retain their normal handling.
- `companion_refresh_obstacles` (`:1853`) calls native `path_avoid_obstacles`
  directly on remaining steps after an immediate physical obstruction. Attempts
  are capped at one per 15 ticks, independently of the existing full-search
  backoff. Fresh grounded input replaces stale path-search origins. Empty/failed
  results cannot overwrite the route; partial results remain partial. A successful
  detour must pass the full ordinary ground/collision safety checks before moving.
- `companion_validate_corridor` (`:1659`) can retain a verified shorter prefix
  only after it covers the unchanged stopping reach plus 0.02 end margin. Cache
  containment uses the shorter actual length, so later movement cannot consume
  unverified padding. No minimum stopping distance, slope/drop threshold,
  footprint margin, jump rule or immediate support check was reduced.
- `companion_movement_throttle` (`:2376`, wired into `companion_control` at
  `:2580`) compensates the player-physics speed axes. It honors the native old
  player-physics/cinematic speed overrides, keeps the throttle magnitude at the
  requested pace, and leaves aiming/fire and persistent orders separate.
- Optional once-per-second counters now include `local_refits`, `shortcuts` and
  `shortened`. Early failed searches clear stale traverse/build diagnostics.

The pinned native actor/path implementations were not rewritten. The existing
`path_obstacle_search_reach` integration remains the only local engine change in
these four compared source files. The new adapter restores its previous value.

### Tests and measurements

The controller harness now includes failures reproduced before the correction
for missed waypoints, blocked proximity shortcuts and optional-padding rejection.
New cases cover narrow corridors, ramps, shortcut bounds/elevation/invalid faces,
blocked shortcut query budgets, dynamic obstacles and reopened doorways, atomic
failed/empty refits, partial-route continuation, unsafe detour rejection, and
five aiming headings. Mutation controls remove each new behavior and must fail.
Existing doorway seams, wall margins, cache boundaries, cliff speeds, jumping,
recovery, movement/fire orders and combat tests remain included.

`test_companion_obstacles.py` additionally extracts the **real native**
`path_avoid_obstacles`, tangent search, heap, circle/pill and obstacle grouping
functions together with Kevin's adapter. A deterministic flat BSP and object
collector fixture verifies an actual native lateral detour, a moving blocker,
a corridor too narrow for a detour, a fresh start and a partial route. A negative
control disables disc collisions and must fail. Engine map/object loading and
biped collision/animation still require gameplay; these fixtures do not replace
that integration check.

Stable 90-tick measurement (same fixture and archived Sprint 1 source):

| Query/work | Sprint 1 | Before this pass | After this pass |
| --- | ---: | ---: | ---: |
| Ground probes | 3,600 | 1,440 | 1,440 |
| BSP queries | 720 | 270 | 270 |
| Full corridor validations | Not counted | 30 | 30 |
| Safety cache reuses | 0 | 60 | 60 |
| Immediate capsule sweeps | 0 | 90 | 90 |

This pass removes false rejection and route churn rather than claiming another
straight-corridor query reduction. Full search counts stay unchanged for waypoint
overshoot and a successful local detour. Local refits are capped at 2 Hz and
failed shortcut tests at 10 Hz; both are absent from ordinary straight following.

### Gameplay still required

After launching the rebuilt Linux binary, replay narrow doors and the previously
reported corridor transitions, walk/run around both directions of a corner, test
ramps, let a marine cross/block the route, reopen the route, and pass a waypoint
while Kevin aims across or behind his travel direction. Verify following and
persistent stay/go-to/hold-fire behavior, cliff refusal and safe low jumping.
Use optional query counters to measure actual map costs. The running game was
not restarted and no new gameplay logs were inspected for this source review.

Full speed-aware braking/turn anticipation, replacement of the current footprint
scheme with navigation geometry, and changes to physical obstruction tolerances
remain deferred until these changes have live evidence. No new teleports, actor
state, model requests or unrelated configuration changes were introduced.

### Completed verification

- Full native harness: **438 passed** (including native solver and mutation controls).
- Existing Linux release target: `ninja -C halo linux` succeeded; a subsequent dry
  run reports no work. No unrelated components were rebuilt. Previous binary:
  `halo/build/linux/halo.before-native-route-fix`.
- Refreshed `games/halo/m8.patch` applies to a separate clean checkout of the full
  pinned revision; all **46** patched files match the working files byte for
  byte, including all **39** original patched paths and existing binary assets.
  Reverse application against the working checkout and whitespace checks pass.
- No Rust implementation changed in this pass; the earlier speech-prompt edit
  remains preserved. No game process was restarted or patched in memory.
