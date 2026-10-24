# Deus Ex (2000) Target Configuration

This module contains configuration and hot function information for the Deus Ex (2000) game binary.

## Known Hot Functions

- `npc_ai_update` - Non-player character AI update loop (High impact)
- `physics_step` - Physics simulation step (High impact)
- `pathfinding` - Navmesh pathfinding resolution (Medium impact)
- `render_submit` - Render submission thread (Low impact)

## Optimization Passes

- `loop_unrolling` - Unroll tight AI loops
- `simd_vectorization` - Vectorize physics math operations
- `cache_friendly_layout` - Reorder pathfinding data for cache locality
- `constant_folding` - Fold constant expressions
- `dead_code_elimination` - Remove unreachable code paths
- `function_inlining` - Inline small helper functions

## Quirks to Preserve

- `ai_frame_timing` - AI frame timing must not be altered
- `physics_substep_order` - Physics substep order must be preserved