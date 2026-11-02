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

## Current-footprint recovery — 2026-10-21

### Live evidence and native comparison

The latest running executable matched the rebuilt binary. All 128 sampled control
updates requested zero movement. Routing failed before a full path search at
Kevin's actual origin `(-49.714, 36.918, 0)`: the negative-Y footprint margin at
`(-49.714, 36.678, 0)` had no structure hit. Native controls/physics were being
asked to stand still. The logs cannot establish whether that missing margin lies
inside wall/doorway geometry or over an exposed edge.

Pinned native `actors.c:actor_find_pathfinding_location()` obtains a biped's
pathfinding location with `biped_find_pathfinding_surface_index()`;
`actor_moving.c:actor_path_input_new()` uses that center location as the path start.
Native movement separately uses directional `structure_test_line2d()` checks and
structure/object avoidance (`actor_move_test_avoidance_vector()`). Kevin's stricter
five-sample current-footprint check can reject the start before those directional
checks or bounded alternative routes are possible. Native actor functions cannot
be called with Kevin as though he had actor state.

### Correction

The full starting-footprint check and ordinary path/safety rules remain intact.
After an invalid-surface failure, a separate, bounded inward escape can restore
full support before normal routing resumes:

- Require grounded center support on a walkable BSP surface and low actual speed.
- Derive movement only away from existing unsupported margins; opposing missing
  edges have no eligible escape. Never substitute the distant goal direction.
- Validate eight short ground/BSP segments, monotonically removing unsupported
  margin bits. Previously supported margins may not become unsupported. Require
  a fully supported endpoint and an unobstructed standing-body physics-pill sweep.
- Limit the maneuver to at most 0.8 world units at 30% pace. Keep a 0.2-unit end
  reserve, reject drift outside its verified strip, and invalidate on order/BSP
  changes. Recheck current/next support, BSP continuity and physical clearance
  each tick; the bounded static trajectory is not repeatedly recomputed.
- Complete as soon as full footprint support returns and restart normal routing
  with a fresh progress window. A previously long wait must not become a false
  stall on the new route. Timeout after two seconds without completion and use
  a five-second retry cooldown. No jump, teleport, actor state or full path search
  is used for the escape itself. Existing full path-search budgets are unchanged.
- Report escape attempts, submitted steps and rejections in the existing debug
  query summary, with rate-limited selection/completion/rejection messages.

The earlier doorway regression tested supported travel across a triangulation
seam. It did not reproduce an already unsupported starting margin. New cases
exercise the logged coordinates with a low wall/doorframe, an exposed margin
(retreat only), normal route resumption, blocked escapes, missing center support,
narrow support islands, dynamic blockers, newly missing margin bits, stay orders,
BSP changes, expiry and corridor drift. The selected escape direction also feeds
the real native unit-control/biped-physics fixture and must produce displacement.
Negative controls disable escape and remove monotonic support protection to prove
that the relevant regressions fail.

Live map collision geometry remains a gameplay verification requirement: the
fixture reproduces the logged rejection and demonstrates a safe inward response,
but does not prove that this exact campaign wall permits the physical sweep.
No running game is restarted or modified by the source/build validation.

Verification: **606 native harness tests pass**, including recovery negative
controls, the long-wait regression, existing doorway/wall/cliff tests and native
physics displacement. The existing Linux release target builds successfully.
The refreshed binary-safe portable patch applies to a separate clean pinned
`8102e711` checkout, with all 50 resulting files byte-identical to the working
checkout. Reverse application and whitespace checks pass. No running game was
restarted or modified.

## Co-op consolidation — 2026-10-22

This section supersedes the earlier requirement to validate a complete footprint
in `companion_navigation_start`. The movement corridor still requires footprint
support; route topology does not. Baseline was the complete local implementation
saved at `/tmp/m8-coop-baseline`, including the inward escape and independent
Marines-free authored positions, rather than only root commit `77e0802`.

### Stage 1: navigation

- `companion_navigation_start` now calls native
  `biped_find_pathfinding_surface_index` (`source/units/bipeds.c:1535`). A fresh
  center ray confirms actual support, height, slope and walkable surface. Missing
  or stale cached positions cannot move the start away from the physical biped.
  This adapts `actor_find_pathfinding_location` (`actors.c:1437`) and
  `actor_path_input_new` (`actor_moving.c:534`) without manufacturing actor state.
- Full footprint checks remain in trajectory and jump-landing validation.
  `companion_safe_direction_check` still requires the entire stopping corridor,
  native known-face BSP continuity, immediate support and the physical pill sweep.
  No slope, drop, collision, cache-containment or jump threshold was loosened.
- Target switches re-evaluate tactical suitability without erasing path, recovery
  or measured-progress state. Weapon changes invalidate range suitability, not
  displacement history. A new BSP or movement mode may still discard a route.
  Partial paths, waypoint overshoot, obstacle refitting and inward escapes remain.
- `companion_failed_destination` / `companion_remember_failure` maintain eight
  local failed destinations, suppressed for five seconds. A different BSP or
  moving at least one world unit from the failed approach permits reconsideration.
  Tactical candidate selection consults this cache before expensive validation.
- Diagnostics distinguish unsupported start, disconnected route, dynamic
  obstruction, unsafe trajectory and requested movement without displacement.
  `last_failure` in the one-second query report is the most recent failure, not
  an assertion that a currently moving biped is still blocked. Existing physical
  displacement/control diagnostics remain authoritative for movement evidence.
- The shared half-second path-search budget and increasing failure backoff remain.
  Stage 1 passed **609 harness tests** and the Linux release build before tactics
  were added. The saved review boundary is `/tmp/m8-coop-navigation-stage.c`.

### Native geometry and control coverage

`test_companion_geometry.py` compiles the actual biped location resolver,
collision-surface closest-point/projection routines and `structure_test_line2d`.
Two connected polygon faces model doorway transitions, ramps, nonwalkable walls,
open edges, missing caches, stale cached positions and unsupported centers. A
small synthetic ray dispatcher uses native polygon containment/projection; it is
not the entire map collision runtime. Polygon edge winding follows the native
closest-point and BSP traversal routines.

Existing deterministic corridor, cliff, braking-distance, wall-margin, dynamic
obstacle, jump and escape cases remain. The separate obstacle harness compiles
native disc/tangent refitting; the physics harness compiles `unit_control`,
`biped_update_moving`, `biped_update_physics` and native base-seat selection.
Perception/position/path output feeds that physics fixture for measured movement,
including recovery and regroup, as well as authored and blocked-lane movement.
These fixtures do not recreate every live animation or map collision condition.

### Measured work and limits

Identical stable 90-tick motion, before versus after this pass:

| Counter | Baseline | Current |
| --- | ---: | ---: |
| Companion ground probes | 1,440 | 1,080 |
| BSP queries | 270 | 270 |
| Full corridor validations | 30 | 30 |
| Safety cache reuse | 60 | 60 |
| Physical capsule sweeps | 90 | 90 |

Ground work falls **25%** without reducing corridor frequency or physical sweeps.
These are deterministic fixture query counts, not measured game frame times.
The baseline/current benchmark uses the same synthetic world; native biped cache
misses in a real map can incur additional engine work. Recovery evaluates at most
five positions and three threats per position, with one shared-budget path search.

No game was running at final validation. Actual a10 doorway geometry, practical
stuck recovery, recovery-position usefulness and frame cost still need a fresh
Pillar of Autumn run with `HALO_DEBUG_COMPANION=1`. The executable is rebuilt;
this does not retroactively update any already running process. No game assets,
services, dialogue, player commands, socket protocol or LAN binding changed.

Final verification for this pass: **633 native harness tests passed**; existing
`--release` Linux target built successfully with `ninja -C halo linux`. The refreshed
binary-safe patch applies to clean pinned `8102e711`; all **52 patched files** match
the working OpenCE checkout byte for byte, preserving all 50 previous patch files.
Reverse application and whitespace checks pass. The two added files are the native
geometry fixture and its extraction runner. No commit or push was made in this pass.

## Automatic crouch and low-obstacle jumps — 2026-10-22

- A verified standing-body obstruction now permits a crouch preview against the
  same ground, cliff, BSP and immediate pill checks. Preview uses the native
  crouching height and slower crouch braking acceleration. Kevin requests the
  normal crouch control flag and waits for the **actual** `biped.crouch` fraction
  before authorizing movement; a request alone cannot shrink his collision test.
  The wait is bounded to two seconds. Failed attempts are rate limited.
- While crouched, full standing clearance is checked at five Hz, including a
  short forward sweep. Kevin only releases automatic crouch after this clears.
  Holding position under an obstruction does not force unsafe standing. Ordinary
  step clearance uses achieved posture and the physical pill, including Kevin's
  actor-less spherical-physics case. Native sneak-axis speeds are included in
  sideways/backwards throttle compensation.
- Jump recovery no longer requires blocked ordinary walking to pass first. A
  confirmed low obstacle may be jumped when either the physical sweep or ordinary
  terrain continuity rejects walking. Launch support, full landing footprint,
  walkable support across the entire runout, upper-body clearance and the native
  impulse/gravity envelope are still required. No jump occurs while crouched or
  holding position. The three-second attempt cooldown remains.
- Only airborne support checks allow footprint samples at different elevations
  across a small raised obstacle: every sample must remain between 0.1 world units
  below and 0.3 above the launch floor. Ordinary walking and landing-footprint
  rules are unchanged. Upper clearance starts above this obstacle band, and the
  available jump apex must be at least 0.5 world units. Missing support, excessive
  drops, taller obstacles and low ceilings remain rejected. This is low-obstacle
  traversal, not gap jumping or climbing arbitrary walls.

Native references: `biped_get_physics_pill` (`bipeds.c:665`), `biped_jump`
(`:1865`), sneak-speed interpolation (`:3812`) and crouch transition (`:3925`),
plus normal crouch control-to-animation routing (`:4327`). No biped physics,
animation seats, actor state, player orders or weapon logic were rewritten.

New regressions exercise the complete movement path for overhead obstruction and
raised-obstacle jumping, actual-posture waiting, safe standing, impossible ducking,
cliff rejection with slower crouch braking, bounded waits, stay priority and
native crouched sideways motion. Negative controls restore premature crouch
movement and flat-only jump support. Gameplay must still establish success on
specific map obstacles; passing a control/geometry fixture is not proof of a live
jump or duck-under traversal.

Validation: the complete 644-test native suite passed after the implementation,
followed by the additional slower-braking cliff regression (passed separately).
The Linux release build succeeds and is current. All 52 patch files apply cleanly
to pinned `8102e711` and match the working checkout byte for byte; reverse-apply
and whitespace checks pass. No game process was running for live confirmation.

## Review corrections — 2026-10-22

- Inward footprint escape now scales its returned movement vector to the verified
  0.3 pace. `companion_movement_throttle` consumes that vector directly; it no
  longer receives full-strength input during the slow escape. The navigation
  fixtures no longer multiply a separate pace into simulated movement.
- A feedback regression sends every escape request through real `unit_control`,
  `biped_update_moving` and `biped_update_physics`, then supplies resolved position
  and velocity to the next controller tick. The escape must restore full support
  within forty ticks without overspeed cancellation before ordinary routing resumes.
- Failed combat destinations release their route ownership before commitment can
  retain them. A different verified destination can leave exhausted recovery;
  invalidation itself preserves failure history and the shared search budget.
- The retry fallback to Chief now applies only to ordinary following. A temporary
  obstruction during investigation preserves the frozen observation destination
  and existing investigation limits. No movement safety threshold was changed.

Validation: **655 native harness tests passed**, including four negative controls
that restore the reported defects, compatible-weapon route retention, and the
native physics feedback regression. The Linux release target rebuilt successfully.
The refreshed portable patch applies to clean pinned `8102e711`; all **52 patched
files** match the working checkout byte for byte. Exact live doorway recovery
still requires gameplay evidence. A running game must restart to use this build.

## Respawn placement correction — 2026-10-31

`companion_spawn` and last-resort catch-up formerly used the generic extra-enemy
placement search. `coop_enemies_spot_free` requires a vertical line of 1.6 world
units from 0.1 above the floor: about 5.2 metres of clear height. This can reject
normal campaign corridors even when the Spartan standing body fits. Its failure
was also conflated with engine object-creation failure in the manual command.

`companion_spawn_position` now evaluates at most 36 nearby candidates using the
actual biped standing pill, full supported footprint, walkable BSP floor and
known-face continuity. A native pill sweep from Chief to the candidate rejects
walls and occupied destinations, ignoring Chief's own body. No full path search
is required, and the generic co-op enemy placement code is unchanged. Failed
automatic spawns and catch-up placement attempts retry at most once per second;
catch-up retains its existing distance, separation and recovery requirements.

Manual respawn creates the replacement before deleting the old Kevin. Placement
or object-creation failure preserves the old body, orders and captured contexts.
Successful creation resets the normal spawn lifecycle. Object pointers are
refetched after creation/weapon cloning in case object memory was compacted.
Console/history distinguish unsupported or obstructed placement from engine
object-creation failure. Native logs aggregate candidate rejection causes.

The actual generic spot check rejects a synthetic 0.9-unit corridor while the
replacement passes full standing-body clearance there. Regressions also cover
insufficient height, missing ground, walls, occupancy, ramps, cliff margins,
missing native caches, both aliases, successful replacement, failed allocation,
failed placement, player unavailability, multiplayer restrictions and negative
controls restoring excessive headroom and deletion before successful creation.

Validation: **671 native harness tests passed**. The Linux release target built
successfully. The binary-safe portable patch applies to clean pinned `8102e711`,
with all **54 files** matching the working checkout byte for byte, preserving
all 52 previous patch files. Live placement still requires the restarted game;
the existing game process was left running with its old executable.

## Bulkhead turn lookahead — 2026-10-31

The reported follow stall at `(-61.013, -33.788, 2.032)` rejected a footprint
sample at `(-60.989, -33.074, 2.032)` while aiming toward the waypoint
`(-60.992, -33.366, 2.032)` and goal `(-60.50, -33.41, 2.03)`. Native path
traversal/build reported success (5/5), but the controller exhausted trajectory
recovery (state 4, failures/recovery 4). This is not proof of a physical collision.

Read-only inspection of the decompressed local a10 cache identifies BSP 6 at
these coordinates. Its collision polygons place the origin and waypoint on floor
surfaces 190 and 344 at approximately z=2.032495. The rejected point lies over
raised bulkhead trim, including an upward surface at z=2.257496, rather than a
drop. A short down probe below that raised surface cannot hit the old floor.
This offline polygon inspection does not substitute for a native runtime ray.
No map data was modified or added to the portable patch.

The controller's full straight stopping corridor extends past the nearby turn;
normal waypoint advancement still waits for proximity/overshoot. The earlier
inward escape addresses a missing current footprint, so it cannot resolve this
case where the present position remains supported. A synthetic raised-trim
regression at the logged coordinates reproduced the deadlock before the fix.

`companion_bypass_waypoint` now tries one existing next leg when the ordinary
trajectory fails within 0.75 units of the intermediate waypoint. The next point
must lie within two units. Fresh ground and known-face BSP continuity, native
`structure_test_pill2d` for the entire shortcut, and unchanged full trajectory
checks for requested motion and actual velocity must all pass before advancing
the waypoint. Failed shortcuts preserve the original rejection reason and
obstruction classification. Attempts share the existing three-tick waypoint
probe interval, require no full path search, and add no work to safe ordinary
movement. Partial/final endpoints without a next leg are not bypassed.

Regressions cover the raised-trim turn, rejected native pill/corner geometry,
missing next-waypoint support, unsafe incoming velocity and partial endpoints.
Negative controls remove the bypass or its pill gate. The selected direction
also passes through the real native control/physics fixture for measured
displacement in its synthetic world. Native log summaries now include path
step/count and partial/final status for future live diagnosis. Exact bulkhead
collision and practical recovery still need confirmation with the rebuilt game.

Validation: **679 native harness tests passed**, including the bulkhead
regressions, negative controls and native control/physics check. The Linux
release build succeeded. The binary-safe patch applies and reverse-checks on
clean pinned `8102e711`; all **54 files** match the working checkout. No running
game was restarted, and no live traversal is claimed from these fixtures.

## Passenger and gunner integration — 2026-11-01

`companion_vehicle_update` now runs before perception and control. Follow targets
Chief's current vehicle only. `companion_vehicle_seat` ranks compatible, vacant
non-driver seats, preferring gunner over passenger. It verifies native seat markers,
entry animations and vacancy without actor state or eviction. Seat selection stays
stable during approach; mounted occupants never change seats opportunistically.

References against pinned OpenCE `8102e711`: `source/ai/action_vehicle.c`,
`action_vehicle_desirable` and `action_vehicle_evaluate_seat` (upright/stopped
checks, entrance, distance/facing gates); `source/units/units.c`,
`unit_has_animation_to_enter_seat`, `unit_can_enter_seat`, `unit_is_busy`,
`unit_get_seat_entrance_point`, `unit_enter_seat` and `unit_try_and_exit_seat`.
The actor-bound vehicle action itself is not used.

Entrance goals are ground-projected with full standing clearance and sent through
the existing bounded route/corridor/obstacle/cliff controller. Approach selects its
own pace rather than inheriting a previous tactical pace. Entry requires native
range/facing gates, a clear approach, grounded feet and an interruptible animation.
There is no timed fake entry. Five navigation failures or forty seconds without
measured progress abandon an approach for five seconds; native path searches retain
the shared budget. Seat and ground evaluation runs at 2 Hz. No available seat means
waiting rather than routing toward Chief inside the hull. Catch-up teleportation
is disabled while Chief is aboard.

Voluntary exit requires an upright, stopped vehicle close to supporting structure,
walkable nearby ground, full-footprint support and standing clearance. A dry-run
`biped_fix_position(..., dont_teleport=TRUE)` validates local placement without
moving Kevin. The same checks run again at exit-animation completion. If terrain
or speed changes, native seated idle resumes and exit waits. A verified exit uses
that local position instead of `biped_exit_seat_end`'s relocation fallback and
suppresses the exit impulse. Forced scripted/death removals bypass this voluntary
animation gate. No new ordinary-recovery teleport is introduced.

Stay and go-to override riding and wait for a safe dismount. Stay holds the actual
landing position. Go-to validates its captured ground and a route from a verified
exit before acceptance; a delayed stay cannot overwrite that destination. Hold-fire
allows riding and continues to suppress personal and mounted firing. No socket,
conversation, model, LAN, game-data or voice configuration changes were made.

**731 native harness tests passed**, and the Linux release build and link check
passed. All **57 patch files** apply to clean pinned OpenCE and match the working
checkout. Validation details and live-test limits are recorded in
[VEHICLES.md](VEHICLES.md).
