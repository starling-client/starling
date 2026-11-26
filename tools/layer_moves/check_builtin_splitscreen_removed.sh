#!/usr/bin/env bash
set -euo pipefail
pattern='split[- _]?screen|SUNRISE_SPLIT|SUNRISE_ACCOUNT_SESSIONS|sunrise\.(split\.|pages\.(open\.)?split_)|(super|crate|runtime)::split::|\bmod split\b|split::(Split|Hud)|\bsplit_(setup|settings|window|auth|in_main_window|guest|hud|layout|map|profiles|ui|players|seat|role)\b|\b(SplitSetup|SplitHelp|SplitRole|PadRouter|paced_by_split|paced_elsewhere|join_split|leave_split|keyboard_handoff|SPLIT_(TWO|THREE|SETTINGS)|open_split_settings)\b'
# git grep exits 1 for no matches, but any other failure must fail the check.
raw=$(git grep -InEi "$pattern" -- crates) || test "$?" -eq 1
test -z "$raw" || { printf '%s\n' "$raw"; exit 1; }
paths=$(git ls-files -- crates | grep -Ei '(^|/)(splitscreen|split/|split_(layout|map|profiles|ui))' || test "$?" -eq 1)
test -z "$paths" || { printf '%s\n' "$paths"; exit 1; }
# Check identifier tokens, not whole lines: an ordinary split_* token must not mask
# another token on the same line. These names are unrelated data, text, or render splits.
ordinary=(
    SPLIT_BLOCKS_JOBS SPLIT_CAPACITY SPLIT_ON_DEATH SplitClip SplitFn SplitMix64 SplitRow SplitWhitespace
    split_address split_all split_args split_at split_at_checked split_at_mut split_blocks
    split_by_dimension_chunk_and_span split_by_entity_sums_to_the_functions_total
    split_by_slow_separates_the_slow_ticks_samples split_by_state_sums_to_the_node_total_of_the_wall_samples
    split_by_thread_id_divides_by_the_named_threads_own_ticks split_child split_controls split_count
    split_cube_children split_dead_cube split_directive
    split_first split_first_chunk split_first_mut split_flags split_for_impl split_formatted split_frame
    split_hover split_id split_inclusive split_last split_limit split_line split_lines split_list split_loot_stacks
    split_name split_narrows_to_a_span_or_a_context_value split_off split_once split_part_fixtures
    split_paths split_points split_ranges split_registry_data split_rows split_rows_keep_their_wire_names
    split_sampler split_scene split_secret split_solid split_style split_t split_terrain split_tests split_top
    split_top_level split_type split_url split_uses_parent_random_and_width_for_both_cube_types split_variant
    split_versions split_w split_whitespace split_x
)
ids=$(git grep -IohE '\b(split_[[:alnum:]_]+|Split[A-Z][[:alnum:]_]*|SPLIT_[[:alnum:]_]+)\b' -- crates) || test "$?" -eq 1
extra=$(comm -23 <(printf '%s\n' "$ids" | sed '/^$/d' | LC_ALL=C sort -u) \
    <(printf '%s\n' "${ordinary[@]}" | LC_ALL=C sort -u))
test -z "$extra" || { printf '%s\n' "$extra"; exit 1; }
printf 'No matches or feature paths.\n'
