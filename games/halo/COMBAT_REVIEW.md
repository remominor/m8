# Kevin and native OpenCE soldier combat

## Baseline and priorities

Comparison: pinned OpenCE `8102e711c353068d22d62efe6bf67ebc8a90a22e`, against the
latest local companion including all movement corrections. Existing local edits
remain preserved. This review does not introduce actor state or combat movement.

| Priority | Current gap | Reusable native mechanism / proposed change |
| --- | --- | --- |
| 1 | `companion_weapon_range` uses weapon names; range and bursts are independent of soldier settings. | `actor_combat_get_firing_variant_definition` (`actor_combat.c:441`) follows `weapon.ai_firing_parameters`, otherwise the actor variant. Read that metadata directly; use a loaded Marine variant matching the weapon as fallback, and its actor perception settings. Cache on weapon/lifecycle changes. Honor projectile maximum range and weapon AI minimum range. |
| 2 | Direct center aiming ignores projectile flight time and engine trajectory calculation. | `actor_combat_update` at `actor_combat.c:2196` estimates flight time, leads target velocity, then calls `weapon_aim` (`:2296`). These public weapon functions need no actor. Use a collision-confirmed center/head point, engine travel-time units, bounded leading and the native aim solution. Keep nonlinear/area/charged/unsupported weapons disabled until their path/collateral safety can be established. |
| 3 | `companion_safe_shot` checks camera-to-target geometry, substitutes current aim only for its friendly cone, and rescans all units for each candidate. | `trigger_create_projectiles` (`weapons.c:2365`) gets the weapon marker and calls `unit_adjust_projectile_ray`; ordinary Kevin shots need the same actual origin. Test both actual and intended firing rays, plus fresh spread-aware friendly bounds in one pass. No cache may authorize firing through a newly entered ally. |
| 4 | One fixed burst pattern; semiautomatic shots ignore burst pauses; weapon swaps retain old timing. | Native `actor_start_burst` (`actor_combat.c:1196`) and separation (`:588`) use actor-variant burst/reaction values. Use deterministic midpoints rather than adding actor randomness/damage modifiers. Respect trigger latched metadata and native runtime reload/heat/readiness. Reset transient fire timing on weapon/target changes. |
| 5 | Every tick scans all enemies, then does line-of-fire/friendly scans for candidates. A valid current target is retained regardless of a new nearby attacker. | Native perception refresh and target weighting (`actor_perception.c:2022`) distinguish recent damage, knowledge, range and retained targets. Bound global sensing to 5 Hz; revalidate the selected target and every emitted fire command immediately. Prefer recent attackers of Kevin/Chief with deterministic retention hysteresis, without copying actor prop state. |

## Native soldier settings and visibility

`actor_get_vision_distances` (`actor_perception.c:1348`) and
`actor_visibility_at_point` (`:2393`) use actor vision angles/distances, firing
variant vision overrides, awareness, lighting, fog and line of sight. They cannot
be called with a biped index. Adapt the distance/central/peripheral limits from
loaded Marine definitions; keep real collision visibility mandatory and keep a
known/recently damaging target trackable without requiring it to remain centered
in Kevin's view. This is similar soldier awareness, not identical prop perception.

Read-only inspection of extracted a10 tags found the armored Marine assault-rifle
variants use maximum firing range **13 world units (39.624 m)** and actor maximum
vision distance **40 world units (121.92 m)**. Kevin currently allows 12 and tracks
45 respectively. These are tag-specific values, not universal Halo constants.

Native `actor_looking.c` coordinates desired looking/aiming/facing; Kevin already
uses smooth aim and independent movement throttle. The recent player-physics
axis compensation must remain. `actor_firing_position.c` chooses combat locations;
that actor/encounter system is outside this change. No cover, pursuit or flanking.

## Weapon mechanics and safety boundaries

- `weapon_estimate_time_to_target` (`weapons.c:1433`) delegates to
  `projectile_estimate_time_to_target` (`projectiles.c:1866`), which divides
  distance by the runtime projectile velocity. The result is simulation ticks;
  object translational velocity is also per tick. Do not multiply by 30 again.
- `weapon_aim` (`weapons.c:702`) delegates to native `projectile_aim` (`:2061`).
  Its boolean only validates the trigger index: it does not propagate a failed
  ballistic solver. Validate the returned direction and supported linear mode.
- Native weapon triggers implement rate acceleration, ammunition, reloading,
  charging, overheat, dispersion and projectile distribution. Kevin must issue
  controls, not edit heat, ammo, damage or projectile parameters. Avoid adding
  tick-random aim error or a second arbitrary accuracy penalty.
- `ai_test_line_of_fire` (`ai.c:2209`) uses friendly collision pills from actor
  perception props. Its outer function requires actor state. Kevin can retain
  conservative physical friendly bounds and weapon-spread envelopes, while
  checking actual/intended rays from the muzzle. In particular a clear eye ray
  does not prove that a muzzle beside a wall is clear.
- Explosive/attached/supercombining projectiles, nonlinear gravity trajectories,
  charged/toggle/unaimable/origin-specific triggers and alternate fire need more
  specialized safety. Resolve these from metadata, not weapon-name spellings;
  leave autonomous fire disabled when the implementation cannot prove safety.
- Minimum target range is a native ranking restriction; treat it conservatively
  as a firing limit without adding a retreat or movement order.
- Vehicle/occupant tracking currently collapses hostile occupants into a hostile
  parent vehicle. Preserve that rule, and read parent velocity for a target riding
  another unit when estimating lead. No vehicle operation or attack relocation.

## Validation plan

Extend the extracted controller harness for native range/profile selection,
center/head visibility, stationary and moving targets, fast/slow projectiles,
metadata-defined bursts and semiautomatic releases, target/weapon changes,
reload/overheat, unsafe actual/intended rays, friendly entry during a burst,
hold-fire, unsupported weapons and bounded perception costs. Include native
weapon/projectile adapters where practical rather than relying only on fake aim.
Retain the movement and order regressions. Refresh the binary-safe portable patch,
verify it against a separate pinned checkout, and build the existing Linux target.
Actual combat competence and map collision/animation still require gameplay.

## Implemented changes

- `companion_update_combat_profile` (`companion.c:840`) caches the loaded Marine
  firing/vision metadata and weapon overrides until the held weapon changes or
  the companion resets. A matching weapon variant is preferred; otherwise a
  loaded Marine supplies perception only; generic firing defaults use the actual
  projectile. No weapon-name matching remains. Projectile range, minimum target range and conservative unsupported
  projectile/trigger checks are applied before enabling autonomous firing.
- `companion_enemy_in_view` (`:1021`) adapts the native partial-distance angular
  interpolation, including the January perception view's field ordering. The
  a10 Marine values are 40 units central distance, 50–70 degrees of transition,
  120 degrees outer angular limit and zero peripheral distance. A target at 60
  degrees therefore has an intermediate sight range, rather than disappearing
  at the central-angle boundary. Already tracked and recent damaging enemies
  can remain known while Kevin turns, but still require actual collision visibility.
- `companion_refresh_enemy` (`:985`) tests real center-of-mass/head points and
  stores the exposed point, replacing sphere-edge visibility followed by blind
  center aiming. Global sensing is capped at 5 Hz. The selected target is refreshed
  every control tick before aiming; stale sensing never authorizes fire.
- `companion_recent_threat` (`:958`) reads existing four-slot attacker records for
  Kevin and Chief. Target selection (`:1169`) prioritizes recent direct attackers,
  retains an otherwise valid target, and requires a half-second between voluntary
  switches. A much nearer equivalent threat can supersede a far target. Dead or
  changed-team targets cannot survive the old visibility grace.
- `companion_aim_solution` (`:1073`) leads the visible point using the native travel
  time and the target/parent unit's per-tick velocity, weighted by the soldier's
  lead fraction. It calls `weapon_aim`, validates a linear normalized solution,
  and rejects shots outside the native range or beyond a two-second prediction
  budget. There is no tick-random custom aim error.
- `companion_firing_origin` (`:1056`) uses the native primary-trigger marker and
  `unit_adjust_projectile_ray`, including camera projection and hidden weapon
  parent markers. A missing marker fails closed. `companion_safe_shot` (`:1117`)
  checks blocking geometry along both intended and actual aim from their firing
  origins. One fresh friendly-unit pass tests both spread envelopes, including
  projectile collision radius. This runs before each permitted fire request;
  friendly safety is not cached.
- `companion_trigger_on` (`:1243`) uses deterministic midpoints of native burst
  and separation values, with a full burst beginning after the reaction delay.
  Latched triggers receive release ticks **and** burst pauses. Target/weapon
  changes reset timing. `companion_reload_needed` (`:1271`) uses the primary
  trigger's magazine index and ammunition requirement. Native reload, readiness,
  age and overheated states gate fire (`:1292`); no ammo, heat or damage is edited.
- A persistently unusable shot/aim solution gets a one-second target cooldown
  after 0.75 seconds, allowing another visible threat to be chosen. Brief ally
  crossings cancel fire without immediately changing targets. This never changes
  Kevin's movement order or invokes combat pathfinding.
- Optional once-per-second `Kevin combat queries` reports profile loads, enemy
  scans, visibility probes, decisions, aim solves, safety rays, friendly scans,
  fire requests and blocked requests, plus effective range/vision/support.

`actor_looking.c:1921` emits separate facing and aiming vectors and can force
actor movement near `:1973`; Kevin retains his existing independent route and
player-physics throttle instead. `actor_firing_position.c:1500–1550` evaluates
candidate positions using actor/encounter context and LOS. That relocation
machinery is deliberately not imported. All **31** extracted movement/navigation
functions match the pre-combat working source byte for byte.

## Measurements and tests

The deterministic 90-tick run below uses the **real native** `weapon_aim`,
`weapon_estimate_time_to_target`, linear projectile aiming/time estimation and
`unit_adjust_projectile_ray`, together with the controller. Collision, animation,
markers and world population are controlled fixtures. The ally-crossing run puts
Chief into the firing line for six eligible ticks of an otherwise identical burst.

| Work / opportunity | Clear lane | Chief crosses for six eligible ticks |
| --- | ---: | ---: |
| Targeting decisions | 90 | 90 |
| Full enemy scans | 15 | 15 |
| Visibility rays, including selected-target refresh | 135 | 135 |
| Native aim solutions | 90 | 90 |
| Intended/actual firing safety rays | 108 | 108 |
| Fresh friendly scans | 54 | 54 |
| Emitted fire requests | 54 | 48 |
| Requests blocked by safety | 0 | 6 |
| Profile/tag scans | 1 | 1 |

These are successful **control opportunities**, not measured hits or projectile
counts: the engine still decides whether a trigger emits a projectile. Enemy
scanning formerly ran once per tick; this fixture verifies 15 rather than 90
scans in the same interval. Candidate ranking now performs no friendly scans.
Debug counters measure primitive calls, not CPU milliseconds or their internal
BSP workload. Actual map timings and combat effectiveness remain gameplay work.

Regressions cover stationary/fast/slow-projectile targets, moving target and
parent-vehicle lead, exposed head versus hidden body, native vision interpolation,
weapon metadata overrides/minimum ranges, recent attackers and retention,
dead/occluded targets, blocked muzzle and current ray, friend entry/spread,
reload/overheat/ammo exhaustion, weapon switching, automatic/semiautomatic burst
pauses, hold/resume-fire and all movement orders. Eight tests additionally run
native weapon/projectile/origin adapters. Mutation controls remove leading,
actual-ray checking, muzzle alignment, friendly exclusion, visibility, burst
pauses or scan throttling and must fail. The fixture requires exactly one case
to execute, preventing an unknown/empty case from counting as a pass.

The existing navigation harness, native obstacle-solver harness, command-context
and persistent-order tests remain part of the full regression run. No weapon or
projectile engine source was changed; the new controller only calls its APIs.

## Limitations and next gameplay pass

This is not full native actor perception: fog, lighting attenuation, alertness,
hearing and encounter coordination are not copied. Recent **damage**, rather than
actor prop firing-intent state, supplies threat priority. No combat movement,
weapon scavenging, alternate fire, overcharging, nonlinear trajectory firing,
splash attacks or damage modifiers were introduced. In particular the stock
plasma pistol primary uses charging metadata and remains disabled here, rather
than risking an unintended charge/release attack. Rifles and supported ordinary
linear primary triggers use native mechanics.

Test a10 rifle combat against stationary and moving grunts/elites, firing while
following and holding, nearby Marines crossing the lane, enemies exposed above
cover, native reloads, weapon swaps, hold/resume-fire and transitions around doors.
Confirm muzzle markers/animation in the actual game and inspect optional counters
when explicitly requested. A new ally can move into an already airborne projectile;
pre-fire checks do not guarantee future collision-free flight. General interception,
curved-path/collateral checks and specialized charged weapons need a separate pass.
No gameplay was driven or restarted, and no live gameplay logs were inspected.

## Completed verification

- Full native harness: **476 passed**, including **55 combat checks**. Native
  weapon/projectile/origin adapters and mutation controls are included.
- Existing Linux release target builds successfully; a subsequent dry run has
  no work. No game process was restarted or patched in memory.
- `games/halo/m8.patch` applies to a separate clean checkout of the full pinned
  revision. All **46** resulting patched files match byte for byte; all **39**
  original patched paths and their existing binary assets are preserved.
- Reverse patch checking against the working checkout and whitespace checks pass.
- Actor combat/perception/looking/firing-position and weapon/projectile engine
  implementations still match the pinned checkout. All **31** navigation/movement
  helpers match the source before this combat pass.
- No conversational/model/server/web/voice code was changed by this pass. The
  existing uncommitted speech prompt and previous navigation changes remain.


## Live non-engagement correction

The first gameplay report after this review exposed a weapon eligibility error.
Live m8 state showed fire enabled, follow active and an assault rifle with 24
rounds loaded. Terminal output showed no acquired targets; optional combat query
logging was disabled. Read-only inspection of the extracted a10 tags established
that both the AR and pistol projectiles have `air_gravity_scale = 1` with
projectile flags zero. The controller rejected any positive gravity, so these
weapons received a zero engagement range and could never acquire a target.

The engine's `projectile_aim` selects ballistic aiming only when **both** its
private ballistic flag (bit 1) and positive air gravity are present. Kevin now
uses that same condition. Ordinary rifle/pistol linear aiming is enabled;
ballistic/area/charged weapons retain their existing conservative restrictions.
Friendly-fire checks still gate every requested shot.

Three additional review findings are corrected with this fix:

- A fallback Marine supplies perception only. Firing metadata comes from a
  matching weapon variant or explicit weapon override. Otherwise cadence and
  reaction use the existing generic defaults, lead remains one, and range comes
  from the actual projectile damage/max range, capped at 12 world units and by
  vision/projectile limits.
- Empty `(0, 0)` burst duration/separation bounds retain generic burst defaults.
  A genuine zero reaction delay is allowed, and specified intervals starting at
  zero remain valid.
- A missing primary trigger marker suspends fire and logs weapon, definition and
  effect object IDs once per outage. Marker recovery is logged once. This origin
  failure does not mark a target obstructed or put it on a targeting cooldown.

Four regressions failed against the previous controller, both with fake aim and
with extracted native weapon/projectile functions (eight failures). They cover
actual a10 rifle gravity/flags, linear-versus-ballistic combinations, acquisition
and friendly exclusion, unrelated Marine firing data, empty timing geometry and
marker failure/recovery diagnostics. Existing movement helpers remain untouched.
The running game was neither restarted nor changed in memory. Relaunch is
necessary to verify the rebuilt controller in gameplay.

Verification: **484 native harness checks pass**, including **63 combat checks**.
The Linux release target builds, and the regenerated portable patch applies to a
separate clean pinned checkout; all 46 patched files match byte for byte and all
39 original paths/binary assets remain preserved.

## Routed combat approach within Chief's 75-metre leash

Under the persistent follow order, an acquired, currently visible target with
fire enabled and a visible enemy now gives Kevin an enemy-relative standoff
destination. He approaches using the same validated path, waypoint, obstruction,
stuck recovery and safe jump pipeline used to follow Chief. The destination stops
inside the equipped weapon's firing range, on Kevin's current side of the enemy,
instead of orbiting around a fixed lateral-step anchor. Aim and fire remain
independent of travel direction.

Combat approach is enabled only under the follow order while fire is allowed and
the weapon profile is supported. The requested destination is clamped to a
**75-metre** radius around Chief. When Kevin is farther than that, or loses a
visible target, he resumes ordinary following. Stay and go-to orders retain
priority; hold-fire suppresses combat movement and returns to following. Route
mode transitions invalidate the previous path and safety cache, keeping retries
and failure budgets separate. Native Marines use actor firing-position selection
and actor path state; Kevin cannot use those actor-bound functions, so this adapts
their routed approach with a target-relative stop point and Kevin's existing
validated pathfinder. It does not add autonomous flanking or pursuit beyond the
Chief leash.

Focused regressions cover a routed enemy approach, 74/75/76-metre leash edges,
standoff before crossing the target, stay/go-to/hold-fire priority, resuming follow
when combat ends, a failed route with no direct-walk fallback, dynamic blocker
rejection, safe ramps and a bounded 600-tick approach. Existing corridor, cliff,
waypoint overshoot and dynamic obstacle cases continue to exercise the navigation
pipeline; the full suite now reports **495 passed**. This harness verifies route
selection and safety interfaces, but cannot establish gameplay pacing or whether
the chosen weapon-range standoff feels natural in every encounter.

Kevin now moves along paths toward visible enemies within his weapon range while
Chief remains within the 75-metre leash; when he reaches standoff range he stops
and keeps aiming/firing. Verify that behavior against Marines in live gameplay,
especially around doors, narrow spaces and moving enemies. Adjust the leash and
standoff ratio after that test if needed.

## Stationary-combat correction

Two gates explained Kevin standing still against visible enemies: target
selection used the weapon's firing range, so targets outside that range never
reached the approach controller; and the approach destination kept the elevated
head/body aim-point height instead of resolving walkable ground. Failed aim
solutions were also counted as target obstruction, so simple range or alignment
failures could unnecessarily cool down a target.

Selection now uses the Marine perception radius, capped at the 75-metre pursuit
envelope. Fire control retains the actual native weapon minimum/maximum range and
the existing muzzle, line-of-fire, ally and aim checks. A failed aim solve no
longer creates an obstruction cooldown; only a shot rejected by the actual safety
rays can do that. A lone target on cooldown remains available for movement, while
another eligible visible threat can still replace it.

Combat goals now resolve a five-point support footprint at the requested XY,
including the height difference between Kevin and the enemy, and pass the
verified ground surface into pathfinding. Ground results are cached for small
target shifts and retried at a bounded rate after failure. Marine firing-position
evaluation uses weapon combat-range bounds when configured and otherwise rewards
positions inside 80% of maximum firing range. Kevin uses the configured range
midpoint when present, with that native range preference as fallback; a 0.75-unit
hysteresis prevents small target changes from repeatedly toggling movement. A
repeated unsafe shot may select one deterministic nearby lateral firing position,
still through the existing safe ground and path pipeline. A per-step geometric
check prevents the route from crossing Chief's leash.

Optional `debug.companion` summaries now report approach and aim-failure counts,
fire requests rejected/not ready, requested movement, observed displacement,
and transitions for grounding, preferred range, blocked-shot repositioning and
the Chief boundary. Existing navigation logs still report path rejection and
unsafe terrain. These values show controller requests and measured displacement;
they do not prove a projectile hit or a successful engagement.

The updated full native harness passes **508 tests**, including end-to-end target
selection into routed movement, out-of-range acquisition without firing, elevated
and ramp ground projection, near-maximum firing, preferred-range hysteresis,
blocked-shot repositioning, route failure and the leash boundary. Gameplay is
still needed to tune real-world positioning and confirm actual movement/fire
behavior in Halo's collision and animation systems.

## Firing-position evaluation after ae4dab5

### Established causes and native comparison

The earlier correction separates perception, firing range and grounded pursuit,
but its `companion_combat_goal()` still treats distance as sufficient reason to
hold. A firing position inside preferred range is never evaluated for LOS or
friendly obstruction unless an actual fire request first reaches
`companion_safe_shot()`. Reaction delay, burst timing, weapon readiness and aim
alignment all precede that check. Consequently a poor firing lane can leave
Kevin stationary indefinitely without triggering repositioning. The old retreat
entry and approach-exit conditions also overlap: a too-close target can turn
movement on, then off on the next tick. The old blocked-position latch has no
successful-arrival/resolved-obstruction exit.

These are established controller defects. Replacing only the position policy with
its archived ae4dab5 implementation makes two movement assertions fail: movement
from a blocked lane inside preferred distance, and sustained retreat across
successive ticks. The assertions examine movement, without depending on new state
fields. This does not establish which branch caused the reported live encounter.
No local Halo process was running during this investigation, and the saved
terminal captures predate ae4dab5.

At pinned OpenCE `8102e711`, `actor_firing_position.c`'s
`pre_evaluator_attack()` evaluates weapon/combat ranges;
`post_evaluator_attack()` separately rewards clear target LOS and can reject
unusable firing positions. Its movement and friend-attack-vector evaluations
favor short movement and avoid interfering firing lanes. `actor_moving.c` then
routes the chosen destination and converts world movement to facing-relative
throttle. These principles are reusable without fabricating actor data.

Native `units.c` maps `_unit_animation_state_in_combat` to standing/crouching.
The alert base seat's sideways restriction does not follow from
`_unit_aiming_speed_alert`. Kevin's animation, aiming settings and speed
compensation are retained.

### Bounded position controller

- Independently assess the current intended firing lane every 15 ticks, even
  while aim is misaligned, the weapon is reloading or no trigger is requested.
  Candidate origins translate the resolved muzzle to each grounded location;
  they are estimates of future usefulness, never authorization to shoot.
- Evaluate at most five deterministic local candidates: forward/retreat, two
  lateral and two forward-lateral. Horizontal displacement is capped at three
  world units; lateral offsets are 1.2 units. Require supported walkable ground,
  standing clearance, a clear target ray/friendly cone and Chief's 75-metre
  leash. All existing trajectory, wall, cliff, slope, obstruction and jump checks
  continue to govern actual movement.
- Rank clear candidates by preferred-range error, then travel distance and fixed
  candidate order. Require a lane improvement or 0.75 world units of range
  improvement. Holding a clear effective position remains valid. A transition in
  existing recent-damage awareness requests reevaluation, without forcing motion.
- Use explicit hold/approach/retreat/reposition states. Separate retreat entry
  and exit thresholds; finish approach within navigation's 0.2-unit arrival
  tolerance. Clear temporary repositioning at a useful position or when the
  obstruction resolves. Preserve selected destinations through small target
  shifts, with a one-second replacement interval. Recheck the selected lane and
  Chief boundary; target/weapon changes invalidate relevant combat state.
- Use scratch native path results and at most one candidate search per shared
  15-tick search interval. Commit a successful complete route atomically and
  reuse it without another search. Failed replacement searches preserve an
  existing route. Exhausted candidates back off at 1, 2 and then 5 seconds.
  Refresh candidate ground, clearance and lane after waiting for a search slot.
- Keep the existing short target-visibility grace; only local last-known-point
  positions are allowed during it. Expiry cancels combat movement, and firing
  still requires current visibility. Player movement orders and hold-fire retain
  priority. No new tactical teleportation or direct-walk fallback is introduced.

### Evidence and runtime diagnosis

The physics harness compiles the actual native `unit_control()`,
`biped_update_moving()` and `biped_update_physics()` functions, plus the actual
base-seat switch from `units.c`. Its deterministic floor/blocker collision fixture
shows world-direction movement while aiming sideways, backwards and diagonally.
Blocking collision, landing and suspension instead produce zero measured
movement from nonzero input. An additional regression passes an acquired target's
actual selected navigation vector into these native routines. Dropping stored
throttle or selecting the alert seat makes the sideways-motion regression fail.
This verifies native control/physics behavior in the fixture, not the live map's
collision geometry, animation assets or hits.

Other regressions cover an effective hold, independent blocked/friendly lane
repositioning, occlusion grace/expiry, retreat stability, completed/resolved
repositioning, goal arrival tolerance, selected-lane changes, weapon changes,
Chief movement, failed replacement atomicity, missing ground and retry budgets.
Existing corridor, doorway, ramp, ledge, jump, order and firing-safety cases remain.

Measured deterministic costs: a stationary-origin, slowly moving-target fixture
uses **one path search, four position evaluations and 12 position rays across 60
ticks**. Unreachable candidate sets use **15 searches, 80 position rays and 275
ground probes across 600 ticks**, with at least 15 ticks between searches. These
are fixture query counts, not gameplay timings.

`HALO_DEBUG_COMPANION=1` adds once-per-second position query summaries and
`Kevin control:` / `Kevin physics:` records. Control reports the decision state,
nav state/retry, goal distance, world walk, submitted/stored throttle, velocity,
seat/desired state, physics mode, control/biped flags, parent, measured prior-tick
displacement and fire rejection reason. The small physics hook in `bipeds.c`
observes desired local motion and collision-resolved displacement/velocity,
collision delta, actual seat/animation flags and suspension, without additional
collision tests. Existing navigation logs distinguish unsafe terrain and route
rejection. Platform messages go to terminal stderr, not `debug.txt`.

Gameplay still needs to establish whether the reported encounter was an
intentional hold, failed grounding/position selection, path/retry failure,
trajectory rejection or zero physical displacement despite stored throttle.
Use matching control/physics records from the rebuilt executable; nonzero inputs
alone cannot establish movement or successful shots.

Verification: **540 native harness checks pass** (`pytest tools/harness/tests`),
and `ninja linux` succeeds with the existing release configuration. The refreshed
portable patch applies cleanly to a separate pinned `8102e711` checkout; all **49**
patched files match the working sources byte for byte, preserving all **46**
previously patched files and their binary assets. Reverse application checks
against the working checkout and whitespace checks also pass.

### Review follow-up: candidate coverage and partial combat routes

Both reported gaps were reproduced before the fixes. At exactly six world units
(the configured preferred distance in the fixture), a blocked lane wider than
1.2 units left no selected destination. A successfully built nonempty partial
route was also rejected before movement could start.

`companion_position_candidates()` retains five candidate slots and the existing
three-unit travel envelope. Its outer pair now uses lateral offsets of 2.4 units
alongside the range adjustment, while the inner pair stays at 1.2 units. At zero
range adjustment this produces four distinct lateral destinations, so an
obstruction that blocks the inner pair can still have a safe outer alternative.
All existing grounding, range-improvement, clearance, LOS, leash and route checks
apply. Evaluation and route-search budgets are unchanged.

`companion_combat_goal()` now accepts nonempty partial results from
`companion_build_path()`, as ordinary navigation already does. The final grounded
combat destination remains separate from the segment endpoint. Exhausting the
segment invalidates that segment; normal navigation resumes toward the same
destination after the shared search interval. A partial result is not proof that
the final destination can be reached: existing progress checks, recovery limits
and terrain validation continue to apply.

New regressions exercise perception/selection through movement for the exact
preferred-distance wide obstruction and partial-route continuation. They check
one-search route commitment, no early continuation search, preservation of the
final grounded destination, and immediate refusal after support disappears on an
accepted partial route. Negative controls reinstate each reported defect and
must fail its regression. These tests establish controller behavior against a
synthetic world; physical displacement in campaign terrain remains a gameplay
check.

Verification: **545 native harness tests pass**, including the new negative
controls, and the existing Linux release build succeeds (`ninja -C halo linux`).
The refreshed binary-safe patch applies cleanly to a separate checkout of pinned
OpenCE `8102e711c353068d22d62efe6bf67ebc8a90a22e`; all **49** patched files match
the working checkout byte for byte, preserving all previous patched paths and
binary assets. Reverse application and whitespace checks pass.

## Marine combat adaptation — 2026-10-20

This section supersedes the earlier local-position-only combat policy. The
baseline is `77e0802` plus the pending candidate-coverage/partial-route fixes.
Comparison remains pinned OpenCE `8102e711`; Kevin still has no actor datum.

### Established controller problems

- Native `actor_combat_update()` admits a new burst at alignment `0.97` beyond
  1.5 world units, interpolating from `0.70710677` at zero distance. Its optional
  start-before-aligned actor flag bypasses this initial alignment delay. Kevin
  instead required both requested and actual vectors to exceed `0.999` every
  firing tick. His extra aim smoothing preceded the engine's own aiming update.
- Native `action_fight_perform()`, `actor_select_firing_position()` and the attack
  evaluators choose among authored encounter positions using combat-range bands,
  LOS, travel, crowding and attack vectors. Kevin's five generated offsets and
  midpoint-based hold did not provide that choice. The native fight selector
  does not generate Kevin's five-point local fallback.
- An empty candidate list between tactical evaluations was incorrectly treated
  as an exhausted search. Successful route commitment clears that list, so
  ordinary ticks accumulated retry backoff while following a valid route. A
  600-tick fixture stopped at x=2.85 after one three-unit segment, with no
  navigation failure, and could not continue approaching. The corrected test
  reaches x=6.82 (within effective range) using three route searches and 91
  moving ticks. Restoring the faulty accounting fails this regression.

These are code/fixture findings. They do not prove which gate dominated a
particular live encounter or that the real biped moved or hit an enemy there.

### Firing and target selection

`companion_fire_requested()` now uses explicit waiting, bursting and pausing
states. Native distance-dependent alignment applies to burst entry; fresh
visibility, range, readiness and shot-safety checks apply throughout. Reload,
heat, invalid solutions, hold-fire and unsafe shots cancel or prevent firing.
Moving/new-target patterns, burst tracking, lead, configured AI trigger rate,
reaction timing and relevant timing/rate difficulty scales come from the matching
Marine firing profile or explicit weapon override. Bounded timing ranges use
midpoints for deterministic behavior. Unsupported weapons still fail closed.
Weapon damage and native projectile dispersion are unchanged.

Combat desired aim is submitted directly to native unit aiming, which retains
the Spartan biped's acceleration, speed and animation bounds. Idle/conversation
look smoothing remains. The obsolete modulo cadence helper and unused aim-error
constants were removed.

Both actual and intended firing rays still check blocking geometry. The single
fresh friendly scan now also checks the entire cone between those aim directions,
including weapon spread and their firing-origin separation. A new regression
first demonstrated that an ally between two individually clear endpoint cones
could otherwise be missed by the wider alignment allowance. The sweep adds no
collision queries or object scans. Nearly opposite/uncertifiable sweeps fail
closed; safety never depends on merely passing the alignment threshold.

`companion_target_weight()` adapts native ranged-weapon range/knowledge tiers,
current-target retention and distance preference. Actual firing direction and
recent damage provide threat evidence; Chief/direct-attacker priorities remain
companion preferences. Marine full/partial sight-distance calculations are
compared against the native perception function across 33 angles, with its
historical overlay names mapped to public tag offsets. Existing body/head LOS
checks and bounded five-Hz enemy scans remain. The former 75-metre target cap is
removed; sensing range and weapon firing range remain distinct.

### Authored positions and navigation

- Cache the nearest 16 valid authored positions within the native ground search
  radius of 15 world units in the current BSP. Include default-team map positions
  independently of any Marine, plus allied Human encounters, not just Chief's
  exact Player team. Explicitly hostile encounters are excluded. Player and
  Human are distinct native teams.
- Cheap range/travel scoring ranks the shortlist before at most five detailed
  checks per half-second evaluation. Native-style attack/combat-range, LOS,
  friendly firing-lane and enemy attack-vector terms guide selection. Reuse the
  actual actor-free `actor_perception_aiming_vector_test_blockage()` primitive.
- Read live friendly actor position claims and body clearance without claiming,
  moving or creating actors. Recheck pending candidate availability when the
  shared path-search budget becomes available. All grounding, standing clearance,
  partial-route continuation and existing movement trajectory checks remain.
- Exhaust authored batches/routes before using the bounded generated local
  fallback. Local fallback is a companion adaptation for absent/unusable authored
  data, not a native Marine feature. It can also get out of Chief's firing lane.
- Preserve useful selected goals through small target movement and native-profile
  residence times; do not rebuild the same destination when residence expires.
  BSP changes invalidate old routes/cache before movement and cannot repeatedly
  reset ordinary following after combat ends. Retry accounting runs for actual
  evaluated/attempted candidate work, rather than every simulation tick.
- Remove Chief's leash from combat goals and movement. Stay and go-to still
  override tactical movement; go-to permits defensive fire, and hold-fire disables
  firing/pursuit. Ordinary following resumes after combat. Catch-up teleportation
  is suppressed during active combat and on the first tick of a new target.

No corridor, doorway, slope, cliff, wall, jump or collision thresholds were
relaxed. No new actor integration, tactical framework, conversation, network or
model-service changes were introduced. The existing LAN interface is untouched.

### Diagnostics, evidence and limits

Optional once-per-second diagnostics report actual/requested alignment, native
threshold, burst state, distinct rejection counts, authored/local goal source,
position score, cache/context scan counts, requested movement and measured native
displacement. A diagnostic observer in native `trigger_create_projectiles()`
counts successfully created companion-owned projectiles; requests, emitted
projectiles and hits are different quantities. The observer does not change
weapon mechanics or authorize a shot.

Deterministic measurements:

| Fixture | Result |
| --- | --- |
| Native angular-acceleration tracking, 120 ticks | 72 safe trigger requests; only 27 of those meet the old `0.999` alignment; 12 alignment waits |
| Local approach, 600 ticks | 91 moving ticks, three route searches, x=6.82; no overshoot or invented candidate failures |
| Unreachable positions, 600 ticks | 15 route searches with interval/backoff enforcement, 80 position rays, 275 ground probes; no direct movement |
| Combat, 90 ticks | 15 enemy scans, one profile scan, 54 safe fire requests |
| Same fixture with six-tick ally crossing | Six immediate safety rejections; firing resumes without target jitter |
| Authored shortlist | At most 16 cached points and five detailed candidates per evaluation; blocked lanes avoid route searches |

The harness uses native aiming acceleration, weapon/projectile aiming math,
friendly-lane geometry, unit controls and biped physics where applicable. The
acquisition-to-authored-position result also feeds the real native control/physics
fixture and must produce measured displacement. Fixtures do not reproduce live
animation assets, squad knowledge, encounter scripting or campaign collision
geometry completely. Kevin does not acquire Marine encounter/squad assignments,
scripted firing-position group permissions, shared prop knowledge or the full
emotional/action controller. Those are deliberate limits of the selected
actor-free approach, rather than claims of exact NPC parity.

Next gameplay validation: compare Kevin and nearby Marines in the same encounter
with `HALO_DEBUG_COMPANION=1`, checking emitted projectiles, actual aim lag,
authored-position availability, measured displacement, friendly crossings and
return to following. A stationary but effective firing position remains valid.

Final verification: **582 native harness checks pass**
(`python -m pytest -q halo/tools/harness/tests`), including negative controls for
alignment, burst continuation, aiming-sweep safety, authored priority and the
between-evaluation retry defect. The existing Linux release build succeeds
(`ninja -C halo linux`). The regenerated binary-safe patch applies cleanly to a
separate pinned `8102e711` checkout; all **50** resulting files match the working
checkout byte for byte, retaining the original **49** patched paths and their
binary assets plus the native projectile observer. Reverse application and
whitespace checks pass. No live game was launched or restarted for validation.

### Live follow-up: default encounter allegiance

The captured a10 session showed mostly settled actual aim, 49 fire requests and
49 observed projectile spawns, but no authored position selections. Nonzero
navigation requests did produce displacement; the stationary intervals had zero
requested movement. This establishes a position-selection problem in that session,
not a demonstrated failure of the biped controls. Shot safety also rejected 304
opportunities; those rejections remain conservative.

Inspection of the loaded a10 cache found all 39 encounter definitions use the
`default` team, including encounters supplying friendly Marines. The former
allied-team tag filter therefore discarded every encounter. Encounter tag team
is not sufficient to infer the allegiance of its live actors.

Kevin now evaluates nearby default-team authored positions directly, with no
living, dead, or previously encountered Marine needed. Authored points are map
geometry, not orders borrowed from an actor. The short-lived association-based
fix was too restrictive and has been removed, along with its extra unit scan and
64-encounter association cap. Explicit hostile encounter restrictions still apply;
neutral/default points are not owned by a hostile actor merely because it uses
the same encounter. Like native encounter activation, an unspecified BSP reference
is accepted, while a different explicit BSP is rejected.

The existing 16-point shortlist, five detailed candidates per evaluation, shared
path-search budget and local fallback remain unchanged. Per-position walkable
surfaces, grounding, occupancy, Marine claims, friendly lanes, path checks and
trajectory safety remain in force. Kevin neither acquires an actor nor modifies
Marine claims or squad state. Squad-specific firing-position group permissions
remain outside this actor-free adaptation; each point is evaluated for Kevin's
current target and physical navigation instead.

Debug output reports nearby cached positions on cache refresh, plus ground,
occupancy, lane and score rejection counters in the existing one-second combat
summary. These distinguish unavailable authored points from unsafe candidate
terrain without extra collision probes. The separately observed `no structure
hit under footprint` rejection is not bypassed by this fix.

Regressions cover default-team selection with no Marine present, unspecified BSPs,
continued availability after Marine death, independence of hostile actor
associations, rejection of explicitly hostile encounters and different explicit
BSPs, unsafe ground, failed routes and bounded retries. The no-Marine case passes
through target acquisition and safe routing into the native control/physics
displacement fixture. A negative control restoring default-team rejection must
fail the case. Live campaign geometry and the improvement in gameplay still
require a new game process using the rebuilt executable.

Independent-selection verification: **591 native harness tests pass**, including
no-Marine acquisition-to-native-physics displacement and the negative control.
The Linux release build succeeds. The regenerated binary-safe patch applies to a
separate clean pinned `8102e711` checkout; all 50 resulting files match the working
checkout byte for byte. Reverse application and whitespace checks pass. No game
process was launched or restarted for validation.

## Bounded co-op purposes and observed threat memory — 2026-10-22

This pass builds on the local navigation/combat fixes above. It does not add full
actor AI or require a living Marine. Persistent orders remain independent of a
small temporary-purpose state: engage, reposition, recover, investigate, regroup.

### Behavior changes

- **Observation:** `companion_observe`, `companion_insert_enemy` and
  `companion_look_around` retain at most twelve timestamped visual observations.
  Only a successful LOS sample updates a position. Occluded entries use the last
  observed position and Kevin's position at that observation; live hidden enemy
  coordinates cannot update their movement destination. Entries expire after
  three seconds, and visible contacts take priority over nearer remembered ones.
  Death/team validation may invalidate memory, but cannot refresh its coordinates.
- **Investigation:** a hidden selected target permits a grounded route toward its
  frozen observation for at most three seconds. The endpoint is within five world
  units of Kevin's observation-time position. Measured travel is also bounded
  (stop request at 4.8 units, reserving 0.2 for stopping); movement does not renew
  the investigation envelope. Fresh visibility and shot checks remain mandatory
  for firing. Nearby visible enemies supersede unusable remembered targets.
- **Recovery:** `companion_tactical_goal` enters at shields <=25% with a known
  target, and leaves at >=65% or threat expiry. `companion_exposure` tests at most
  three existing observations. Five local grounded candidates are ranked by lower
  exposure, then shorter travel. Concealment does not require a firing lane.
  Candidates still need walkable ground, clearance, a native route and all normal
  trajectory checks. No available safer position means defensive holding, with
  safe firing still permitted; it does not cause blind retreat or teleporting.
- **Engagement:** retain effective firing positions. Incoming damage can request
  an exposure-improving position on the next two-Hz evaluation. Existing friendly
  lane, occupancy, weapon-distance and authored-position scoring remain. A valid
  selected move is committed for at least one second unless movement safety
  rejects it, its purpose changes, or a changed weapon invalidates its range.
  Target changes alone preserve route and progress. Safety and per-shot friendly
  checks continue every tick, independent of commitment.
- **Regroup:** while following, separation greater than 20 metres and increasing
  for one second activates regroup. It remains active until within 12 metres.
  Kevin uses the ordinary follow route and can defend while returning. This is a
  preference, not a hard combat leash. Shield recovery takes priority over regroup.
- **Orders:** stay and go-to retain movement priority. Hold-fire prevents firing
  and autonomous combat movement, while ordinary following remains available.
  Tactical state never rewrites an order. No new model calls or socket messages.
- **Firing:** native weapon math, projectile leading, burst timing, reload and
  alignment rules are unchanged. Rejections now distinguish invalid observation,
  missing firing origin, unsafe aim sweep, obstructing geometry/object, and
  friendly cones. Alignment/origin failures do not accumulate blocked-target
  cooldown. Tactical lane evaluation remains independent of trigger success.

### Source references and adaptation boundaries

- OpenCE `8102e711`: `elite_decide_action` (`actor_type_elite.c:78–107`) and
  `actor_action_handle_active_cover_seeking` (`actions.c:1777`) motivate separate
  shield entry/exit thresholds. Kevin reads his own vitality directly; no actor
  defensive state or damage modifiers are imported.
- `actor_firing_position.c` supplies the range/LOS/context evaluation pattern;
  `actor_moving.c:2103` supplies stable route advancement and obstruction handling.
  Attack and concealment are evaluated for different purposes.
- [Quake III, ai_dmq3.c](https://github.com/id-Software/Quake-III-Arena/blob/master/code/game/ai_dmq3.c):
  committed movement and bounded last-seen pursuit inform the independent local
  implementation; random dodging and arena traversal were not copied.
- [ReGameDLL_CS attack state](https://github.com/rehlds/ReGameDLL_CS/blob/master/regamedll/dlls/bot/states/cs_bot_attack.cpp):
  exposure and combat circumstances inform purposeful repositioning.
- [Xonotic Havocbot](https://gitlab.com/xonotic/xonotic-data.pk3dir/-/blob/master/qcsrc/server/bot/default/havocbot/havocbot.qc)
  and [Red Eclipse AI](https://github.com/redeclipse/base/blob/master/src/game/ai.cpp):
  progress-aware failure memory informs the bounded unsuccessful-destination cache.
- [Left4Bots](https://github.com/smilz0/Left4Bots/blob/main/root/scripts/vscripts/left4bots_ai.nut):
  measured movement and companion-follow coordination inform the separation of
  requested motion, observed displacement and regrouping. No foreign library,
  physics, navigation data, random strafing or teleport recovery was imported.

### Regression and measurement evidence

Tests cover frozen hidden coordinates (including a negative control that restores
live coordinates), expiry, a full memory list, target switching during movement,
shield hysteresis, cover without a firing lane, no safe cover, missing support,
regroup hysteresis, all movement/fire order combinations, investigation duration
and distance, shared search budgets, and distinct shot rejection reasons. Negative
controls also undo recovery hysteresis, regrouping and failed-destination memory.
Existing native aim/projectile, friendly crossing, burst, reload, side/backwards
movement, route safety and emitted-projectile observer tests remain.

For the same 90-tick combat fixture, baseline and current are identical: 90 target
choices, 15 global scans, 135 visibility probes, 90 aim solutions, 54 accepted fire
requests; inserting a friendly produces six blocked requests and still 54 accepted
requests. Ground-query reduction is documented in `NAVIGATION_REVIEW.md`.
Accepted requests are not measured hits or live emitted projectiles. The existing
projectile-emission callback regression verifies reporting, not combat effectiveness.

A new Pillar of Autumn run is still needed to establish actual advancement,
concealment quality, stopping distance during investigation, doorway recovery,
emitted projectiles and hits. Local recovery examines only five positions and can
miss better cover farther away. Observations expire quickly by design. No grenade,
vehicle, scavenging, melee expansion or squad-state integration is included.

Final verification: **633 native harness tests passed**, Linux release build
succeeded, and all **52 patched files** were verified byte for byte after applying
the regenerated patch to a separate clean pinned `8102e711` checkout. No live
movement or hit-rate improvement is claimed from these results.

## Review corrections — 2026-10-22

`companion_combat_goal` now consults failed-destination memory before retaining a
selected local position, including during its initial commitment. Previously a
clear predicted firing lane could retain an unusable position indefinitely even
with an invalid route. Regressions verify that a different candidate is evaluated,
receives a validated route and produces movement after exhausted recovery.

Weapon changes reject a selected position inside the new minimum-distance margin
while preserving compatible routes and their progress history. A regression raises
the weapon minimum range and verifies that retreat cannot keep the old forward
approach; another verifies that a compatible swap does not trigger another search.
Investigation retries retain their remembered destination instead of inheriting
the ordinary follow fallback to Chief. Orders and per-shot safety remain intact.

Validation: **655 native harness tests passed**, including negative controls for
all four reported defects and the native escape physics feedback described in
`NAVIGATION_REVIEW.md`. The Linux release build succeeded, and the refreshed patch
was applied to clean pinned `8102e711` with all **52 files** matching the working
checkout. These results establish controller behavior, not live movement or hits.
