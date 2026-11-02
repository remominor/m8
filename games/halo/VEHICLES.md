# Kevin vehicle support

While following, Kevin approaches Chief's stopped vehicle and prefers a free,
compatible gunner seat, falling back to a passenger seat. He never drives, evicts
an occupant, or swaps seats during a ride. Let the vehicle stop and allow him time
to approach the authored entrance. No extra player command is needed.

- Stay or go-to: dismount when safe, then honor the movement order.
- Hold-fire: ride normally, with personal and mounted firing disabled.
- Chief exits or changes vehicle: safely leave the old vehicle first.
- Unsafe landing, moving/overturned vehicle, unavailable animation or seat:
  wait conservatively. There is no forced boarding or dismount teleport.
- Direct-fire machine-gun turrets are supported. Explosive, ballistic, charged and
  unsupported weapons refuse autonomous firing.

Enable `HALO_DEBUG_COMPANION=1` when launching OpenCE for existing navigation,
combat and physics diagnostics plus rate-limited vehicle wait reasons. Approach,
boarding, abandoned attempts and safe-exit transitions are logged. Diagnostics
report controller requests and measured displacement separately.

## Automated validation

The source-extraction harness exercises production seat selection, vehicle state,
exit preflight/completion hooks, safe navigation and mounted combat. Fixture
collision, animation and world responses are synthetic; this is not a live map or
an animation-asset test. Native weapon-math adapters and physics/control regression
coverage remain in the full suite.

New cases cover gunner preference, passenger fallback, incompatible/occupied seats,
entry races, facing, moving/overturned vehicles, blocked entrances, airborne/busy
entry, order priority, route retry/progress budgets, 2 Hz sampling, safe/unsafe and
changed exits, forced-removal bypass, validated seated go-to starts, delayed stay
state, mounted weapon ownership, actual turret alignment, all-muzzle safety,
occupants in the firing lane, hold-fire and unsupported weapons. Navigation cases
verify boarding follows the existing route controller and still rejects cliffs.

Run the complete suite and release target:

```sh
uv run --no-project --with pytest python -m pytest -q halo/tools/harness
ninja -C halo linux
```

Portable patch verification applies the full binary-safe patch to a separate clean
checkout of pinned `8102e711c353068d22d62efe6bf67ebc8a90a22e`, compares every
patched file with the working checkout, and checks reverse applicability. Existing
binary assets and unrelated integration changes are retained.

Final validation on 2026-11-01: **731 native harness tests passed** (80.80 s),
including negative controls, native weapon adapters and physics/control tests.
The Linux release target passed, including its link check. The refreshed patch
applied and reverse-checked against clean pinned OpenCE; all **57 patched files**
matched the working checkout byte for byte, including existing binary assets.

## Gameplay still required

Test on a campaign map with a Warthog; Pillar of Autumn alone cannot establish
vehicle acceptance. Verify approach and entry animations with the actual Spartan
asset, Chief driving without control interference, gunner aim/traverse limits,
emitted shots and hits, passenger firing where the seat permits it, moving rides,
Chief dismount, stay/go-to dismount, occupied seats and hold/resume-fire. Repeat
exits beside walls, ramps, cliffs and blocked doors; an unsafe exit must wait.
A checkpoint load, death and vehicle destruction should retain native lifecycle
behavior. No running game is restarted by this change; launch the rebuilt binary
to evaluate it. Automated requests do not prove actual boarding, motion or hits.
