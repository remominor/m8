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
  loaded Marine's ordinary settings supply the baseline. No weapon-name matching
  remains. Projectile range, minimum target range and conservative unsupported
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
