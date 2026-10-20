# Ported tests

One row per Python test: `| python_file::test_name | rust_file::test_name or - | note |`.
Rust paths are relative to `rust/agent-config`. `ported_map_complete` (in `tests/guard.rs`) checks
every row: the Rust function must exist in the file the row names (a function of the same name in
another file does not count), a `-` row must carry a note, and the row count per Python file must
match (130, 64, 23, 199, 11, 18, 14 and 2 for the eight files of this list, 461 rows in all).

The four skill-script files (`test_spawn.py`, `test_route.py`, `test_whatsleft_scan.py`,
`test_harness_stats.py`) use plain `file::test_name` rows. The spawn and route bins already carry a
unit test per Python case in their own `mod tests` (`src/bin/spawn.rs`, `src/bin/route.rs`); the rest
are process tests in `tests/skill_bins.rs`, which run the bin on a temp repo or temp HOME.

`scripts/test_agent_config.py` is class-based and several classes share a test name, so its rows name
the class too (`Class::test_name`). Its Rust twins keep the Python name, with the class as a prefix
(`test_cursor_hooks_dry_run_writes_nothing`) where the name occurs in more than one class. They are
unit tests in `src/agent_config_tests.rs` (helpers, and the parts that write files, called as
functions with a temp HOME and a fake repo) and process tests in `tests/agent_config.rs`
(`agent-config cursor|opencode|user-rules|claude-settings`); the five tests of the short output are in
`tests/agent_quiet_output.rs`, because they spend process-wide counters. A `-` row is behavior only
the Python has and the Rust tool does not need (one row so far: the Python hook loaded a sibling module).

Notes on the rows that are not a line-for-line port:

- The binaries run as hooks have no seam for Python mocks. Where a Python test mocked `pace`, the
  Rust test writes the statusline cache file the binary reads first (same trick as
  `tools/hook_test_shim.py`); `GUARD_PLATFORM` stands in for `sys.platform`.
- Python tests that called a Python function directly run the same scenario through the binary, or
  are unit tests in `src/bin/guard.rs` / `src/pace_core.rs` (`mod tests`).

| Python test | Rust test | Note |
|---|---|---|
| claude/hooks/test_guard.py::test_heavy_agents_denied | tests/guard.rs::test_heavy_agents_denied | |
| claude/hooks/test_guard.py::test_router_table | tests/guard.rs::test_router_table | |
| claude/hooks/test_guard.py::test_router_review_cases | tests/guard.rs::test_router_review_cases | |
| claude/hooks/test_guard.py::test_router_never_raises_cost | tests/guard.rs::test_router_never_raises_cost | |
| claude/hooks/test_guard.py::test_router_opus_rows_follow_the_table | tests/guard.rs::test_router_opus_rows_follow_the_table | fake pace = statusline cache file |
| claude/hooks/test_guard.py::test_router_silent_when_nothing_changes | tests/guard.rs::test_router_silent_when_nothing_changes | |
| claude/hooks/test_guard.py::test_router_skips_fork_and_subagents | tests/guard.rs::test_router_skips_fork_and_subagents | |
| claude/hooks/test_guard.py::test_router_pace_ahead_downgrades_opus | tests/guard.rs::test_router_pace_ahead_downgrades_opus | fake pace = statusline cache file |
| claude/hooks/test_guard.py::test_router_merges_with_warnings | tests/guard.rs::test_router_merges_with_warnings | |
| claude/hooks/test_guard.py::test_w3_browser | tests/guard.rs::test_w3_browser | |
| claude/hooks/test_guard.py::test_w4_budget | tests/guard.rs::test_w4_budget | |
| claude/hooks/test_guard.py::test_g1_missing_effort_or_model_is_filled_by_router | tests/guard.rs::test_g1_missing_effort_or_model_is_filled_by_router | |
| claude/hooks/test_guard.py::test_g1_ok | tests/guard.rs::test_g1_ok | |
| claude/hooks/test_guard.py::test_g1_brief_needs_deadline_and_cap | tests/guard.rs::test_g1_brief_needs_deadline_and_cap | |
| claude/hooks/test_guard.py::test_g1_brief_accepted_forms | tests/guard.rs::test_g1_brief_accepted_forms | |
| claude/hooks/test_guard.py::test_g1_max_effort_is_clamped_not_denied | tests/guard.rs::test_g1_max_effort_is_clamped_not_denied | |
| claude/hooks/test_guard.py::test_g2_cron | tests/guard.rs::test_g2_cron | |
| claude/hooks/test_guard.py::test_g2_cron_one_shot | tests/guard.rs::test_g2_cron_one_shot | |
| claude/hooks/test_guard.py::test_g2_cron_lists_and_hours | tests/guard.rs::test_g2_cron_lists_and_hours | |
| claude/hooks/test_guard.py::test_cron_interval_function | src/bin/guard.rs::test_cron_interval_function | unit test: called `guard.cron_interval_minutes` directly |
| claude/hooks/test_guard.py::test_g2_wakeup | tests/guard.rs::test_g2_wakeup | |
| claude/hooks/test_guard.py::test_g2_loop_skill | tests/guard.rs::test_g2_loop_skill | |
| claude/hooks/test_guard.py::test_g3_prompt_cadence | tests/guard.rs::test_g3_prompt_cadence | |
| claude/hooks/test_guard.py::test_g3_prompt_below_threshold_and_sessions_independent | tests/guard.rs::test_g3_prompt_below_threshold_and_sessions_independent | |
| claude/hooks/test_guard.py::test_g3_agent_ceiling | tests/guard.rs::test_g3_agent_ceiling | the `guard.SPAWN_CONTEXT == 350000` line is checked by the 351K deny and 340K allow |
| claude/hooks/test_guard.py::test_g4_first_ask_allowed_even_if_written | tests/guard.rs::test_g4_first_ask_allowed_even_if_written | |
| claude/hooks/test_guard.py::test_g4_second_ask_denied | tests/guard.rs::test_g4_second_ask_denied | |
| claude/hooks/test_guard.py::test_g4_parallel_asks_first_allowed | tests/guard.rs::test_g4_parallel_asks_first_allowed | |
| claude/hooks/test_guard.py::test_g4_denied_ask_does_not_count | tests/guard.rs::test_g4_denied_ask_does_not_count | |
| claude/hooks/test_guard.py::test_g4_askme_interview_exempt | tests/guard.rs::test_g4_askme_interview_exempt | |
| claude/hooks/test_guard.py::test_g4_new_human_prompt_resets | tests/guard.rs::test_g4_new_human_prompt_resets | |
| claude/hooks/test_guard.py::test_g4_list_content_human_and_meta_skipped | tests/guard.rs::test_g4_list_content_human_and_meta_skipped | |
| claude/hooks/test_guard.py::test_g5_default_cap_and_message | tests/guard.rs::test_g5_default_cap_and_message | |
| claude/hooks/test_guard.py::test_g5_cap_from_prompt | tests/guard.rs::test_g5_cap_from_prompt | |
| claude/hooks/test_guard.py::test_g5_smallest_tied_cap_wins | tests/guard.rs::test_g5_smallest_tied_cap_wins | |
| claude/hooks/test_guard.py::test_g5_cap_ignored_when_not_tied_to_reply | tests/guard.rs::test_g5_cap_ignored_when_not_tied_to_reply | |
| claude/hooks/test_guard.py::test_g5_cap_phrasings | tests/guard.rs::test_g5_cap_phrasings | |
| claude/hooks/test_guard.py::test_g5_counter_write_fails_allows | tests/guard.rs::test_g5_counter_write_fails_allows | |
| claude/hooks/test_guard.py::test_g5_defaults_by_agent_type | tests/guard.rs::test_g5_defaults_by_agent_type | |
| claude/hooks/test_guard.py::test_g5_two_denies_then_allow | tests/guard.rs::test_g5_two_denies_then_allow | |
| claude/hooks/test_guard.py::test_g5_counter_keyed_by_session_without_agent_id | tests/guard.rs::test_g5_counter_keyed_by_session_without_agent_id | |
| claude/hooks/test_guard.py::test_g5_subagentstop | tests/guard.rs::test_g5_subagentstop | |
| claude/hooks/test_guard.py::test_g5_subagentstop_cap_from_prompt | tests/guard.rs::test_g5_subagentstop_cap_from_prompt | |
| claude/hooks/test_guard.py::test_g5_transcript_path_variants | tests/guard.rs::test_g5_transcript_path_variants | |
| claude/hooks/test_guard.py::test_g5_prompt_as_text_blocks_and_first_user_record_only | tests/guard.rs::test_g5_prompt_as_text_blocks_and_first_user_record_only | |
| claude/hooks/test_guard.py::test_g5_prompt_read_from_start_limited_to_200_lines | tests/guard.rs::test_g5_prompt_read_from_start_limited_to_200_lines | |
| claude/hooks/test_guard.py::test_g5_missing_transcript_falls_back_to_default | tests/guard.rs::test_g5_missing_transcript_falls_back_to_default | |
| claude/hooks/test_guard.py::test_g5_pretool_ignores_other_tools_and_bad_input | tests/guard.rs::test_g5_pretool_ignores_other_tools_and_bad_input | |
| claude/hooks/test_guard.py::test_g6_stop_needs_link_and_block | tests/guard.rs::test_g6_stop_needs_link_and_block | |
| claude/hooks/test_guard.py::test_g6_stop_one_deny_and_scope | tests/guard.rs::test_g6_stop_one_deny_and_scope | |
| claude/hooks/test_guard.py::test_g6_archive_same_turn_denied | tests/guard.rs::test_g6_archive_same_turn_denied | |
| claude/hooks/test_guard.py::test_scratch_allowed_paths | tests/guard.rs::test_scratch_allowed_paths | |
| claude/hooks/test_guard.py::test_scratch_denied_paths | tests/guard.rs::test_scratch_denied_paths | scratch mode runs with a HOME outside the temp dir (`~` must resolve outside the allowed roots) |
| claude/hooks/test_guard.py::test_scratch_tilde_symlink_and_relative | tests/guard.rs::test_scratch_tilde_symlink_and_relative | same HOME note |
| claude/hooks/test_guard.py::test_scratch_fail_open | tests/guard.rs::test_scratch_fail_open | |
| claude/hooks/test_guard.py::test_last_assistant_usage_used_and_large_file | src/bin/guard.rs::test_last_assistant_usage_used_and_large_file | unit test: called `guard.context_size` directly |
| claude/hooks/test_guard.py::test_cap_three_at_once_then_stop_frees_a_slot | tests/guard.rs::test_cap_three_at_once_then_stop_frees_a_slot | |
| claude/hooks/test_guard.py::test_cap_failed_spawn_frees_its_own_slot | tests/guard.rs::test_cap_failed_spawn_frees_its_own_slot | |
| claude/hooks/test_guard.py::test_cap_failure_ignores_other_tools_and_bad_input | tests/guard.rs::test_cap_failure_ignores_other_tools_and_bad_input | |
| claude/hooks/test_guard.py::test_cap_old_format_slots_still_load | tests/guard.rs::test_cap_old_format_slots_still_load | |
| claude/hooks/test_guard.py::test_cap_denied_call_is_not_counted_and_stop_never_goes_negative | tests/guard.rs::test_cap_denied_call_is_not_counted_and_stop_never_goes_negative | |
| claude/hooks/test_guard.py::test_cap_other_denies_do_not_take_a_slot | tests/guard.rs::test_cap_other_denies_do_not_take_a_slot | |
| claude/hooks/test_guard.py::test_cap_stop_with_denied_handback_keeps_the_slot | tests/guard.rs::test_cap_stop_with_denied_handback_keeps_the_slot | |
| claude/hooks/test_guard.py::test_cap_entries_older_than_90_minutes_are_dropped | tests/guard.rs::test_cap_entries_older_than_90_minutes_are_dropped | |
| claude/hooks/test_guard.py::test_cap_by_pace | tests/guard.rs::test_cap_by_pace | fake pace = statusline cache file |
| claude/hooks/test_guard.py::test_cap_without_pace_module_is_unknown | tests/guard.rs::test_cap_without_pace_module_is_unknown | no cache file = unknown |
| claude/hooks/test_guard.py::test_cap_pace_crash_is_unknown | tests/guard.rs::test_cap_pace_crash_is_unknown | a pace function that raises becomes an unreadable cache file |
| claude/hooks/test_guard.py::test_cap_parallel_calls_race_safely | tests/guard.rs::test_cap_parallel_calls_race_safely | |
| claude/hooks/test_guard.py::test_cap_state_dir_unwritable_fails_open | tests/guard.rs::test_cap_state_dir_unwritable_fails_open | |
| claude/hooks/test_guard.py::test_plan_gate_denies_planning_skills_outside_plan_mode | tests/guard.rs::test_plan_gate_denies_planning_skills_outside_plan_mode | |
| claude/hooks/test_guard.py::test_plan_gate_desktop_sonnet_points_to_plan_helper_or_opus | tests/guard.rs::test_plan_gate_desktop_sonnet_points_to_plan_helper_or_opus | |
| claude/hooks/test_guard.py::test_plan_gate_allows_opus_and_fable_mains | tests/guard.rs::test_plan_gate_allows_opus_and_fable_mains | |
| claude/hooks/test_guard.py::test_plan_gate_unknown_model_or_entrypoint_allows | tests/guard.rs::test_plan_gate_unknown_model_or_entrypoint_allows | |
| claude/hooks/test_guard.py::test_plan_gate_uses_the_newest_assistant_model | tests/guard.rs::test_plan_gate_uses_the_newest_assistant_model | |
| claude/hooks/test_guard.py::test_plan_gate_allows | tests/guard.rs::test_plan_gate_allows | |
| claude/hooks/test_guard.py::test_enter_plan_mode_denied_when_ahead | tests/guard.rs::test_enter_plan_mode_denied_when_ahead | |
| claude/hooks/test_guard.py::test_pace_note_once_when_ahead | tests/guard.rs::test_pace_note_once_when_ahead | the `calls == [5, 5]` check (pace.refresh count) cannot be seen from outside the binary: not ported |
| claude/hooks/test_guard.py::test_pace_note_critical_and_return_to_ok | tests/guard.rs::test_pace_note_critical_and_return_to_ok | |
| claude/hooks/test_guard.py::test_pace_note_sessions_independent_and_joined_with_nudge | tests/guard.rs::test_pace_note_sessions_independent_and_joined_with_nudge | |
| claude/hooks/test_guard.py::test_pace_note_fail_open | tests/guard.rs::test_pace_note_fail_open | the raising `status`/`refresh` mocks become an unreadable cache; a failing refresh next to a good status is not observable |
| claude/hooks/test_guard.py::test_check_off | tests/guard.rs::test_check_off | `check_off` is private to the binary: driven through G14 with the list forms |
| claude/hooks/test_guard.py::test_g6_deleted_handoff_is_skipped | tests/guard.rs::test_g6_deleted_handoff_is_skipped | |
| claude/hooks/test_guard.py::test_g10_dirty_file_touched_this_turn | tests/guard.rs::test_g10_dirty_file_touched_this_turn | |
| claude/hooks/test_guard.py::test_g10_untouched_files_stay_quiet | tests/guard.rs::test_g10_untouched_files_stay_quiet | |
| claude/hooks/test_guard.py::test_g10_unpushed_needs_a_commit_this_turn | tests/guard.rs::test_g10_unpushed_needs_a_commit_this_turn | |
| claude/hooks/test_guard.py::test_g10_user_said_not_to_push | tests/guard.rs::test_g10_user_said_not_to_push | |
| claude/hooks/test_guard.py::test_g10_no_upstream_and_git_failures_are_quiet | tests/guard.rs::test_g10_no_upstream_and_git_failures_are_quiet | |
| claude/hooks/test_guard.py::test_g10_git_timeout_or_crash_fails_open | tests/guard.rs::test_g10_git_timeout_or_crash_fails_open | the subprocess mocks (timeout, OSError, ValueError) become a git that cannot start (empty PATH), as in the shim |
| claude/hooks/test_guard.py::test_g10_switches | tests/guard.rs::test_g10_switches | |
| claude/hooks/test_guard.py::test_g11_long_turn_without_a_clock | tests/guard.rs::test_g11_long_turn_without_a_clock | |
| claude/hooks/test_guard.py::test_g11_a_clock_or_duration_in_the_reply_is_enough | tests/guard.rs::test_g11_a_clock_or_duration_in_the_reply_is_enough | |
| claude/hooks/test_guard.py::test_g11_switches_and_missing_transcript | tests/guard.rs::test_g11_switches_and_missing_transcript | |
| claude/hooks/test_guard.py::test_g12_open_steps_fire | tests/guard.rs::test_g12_open_steps_fire | |
| claude/hooks/test_guard.py::test_g12_replays_advance_done_and_start | tests/guard.rs::test_g12_replays_advance_done_and_start | |
| claude/hooks/test_guard.py::test_g12_list_from_an_earlier_turn_still_counts | tests/guard.rs::test_g12_list_from_an_earlier_turn_still_counts | |
| claude/hooks/test_guard.py::test_g12_stays_quiet_when_asked_wrapped_or_questioned | tests/guard.rs::test_g12_stays_quiet_when_asked_wrapped_or_questioned | |
| claude/hooks/test_guard.py::test_g12_quiet_while_a_helper_slot_is_live | tests/guard.rs::test_g12_quiet_while_a_helper_slot_is_live | SPAWN_TTL (5400 s) written as a literal |
| claude/hooks/test_guard.py::test_g12_switches_and_missing_transcript | tests/guard.rs::test_g12_switches_and_missing_transcript | |
| claude/hooks/test_guard.py::test_stop_checks_run_in_order_and_only_one_fires | tests/guard.rs::test_stop_checks_run_in_order_and_only_one_fires | |
| claude/hooks/test_guard.py::test_fail_open | tests/guard.rs::test_fail_open | |
| claude/hooks/test_guard.py::test_missing_transcript_means_zero_context | tests/guard.rs::test_missing_transcript_means_zero_context | |
| claude/hooks/test_guard.py::test_g3_text_no_handoff_push | tests/guard.rs::test_g3_text_no_handoff_push | |
| claude/hooks/test_guard.py::test_g14_denies_in_place_forms | tests/guard.rs::test_g14_denies_in_place_forms | direct `g14_sed` calls become `pretool` runs with GUARD_PLATFORM=darwin |
| claude/hooks/test_guard.py::test_g14_denies_after_separators_and_wrappers | tests/guard.rs::test_g14_denies_after_separators_and_wrappers | same |
| claude/hooks/test_guard.py::test_g14_allows | tests/guard.rs::test_g14_allows | same |
| claude/hooks/test_guard.py::test_g14_macos_only_off_switch_and_fail_open | tests/guard.rs::test_g14_macos_only_off_switch_and_fail_open | same |
| claude/hooks/test_guard.py::test_g14_through_pretool | tests/guard.rs::test_g14_through_pretool | runs on every OS (GUARD_PLATFORM=darwin); the Python test skipped itself off macOS |
| claude/hooks/test_guard.py::test_g15_read_tool | tests/guard.rs::test_g15_read_tool | direct `g15_secrets` calls become `pretool` runs |
| claude/hooks/test_guard.py::test_g15_bash_readers_denied | tests/guard.rs::test_g15_bash_readers_denied | same |
| claude/hooks/test_guard.py::test_g15_bash_allowed | tests/guard.rs::test_g15_bash_allowed | same |
| claude/hooks/test_guard.py::test_g15_process_command_lines_denied | tests/guard.rs::test_g15_process_command_lines_denied | same |
| claude/hooks/test_guard.py::test_g15_process_checks_allowed | tests/guard.rs::test_g15_process_checks_allowed | same |
| claude/hooks/test_guard.py::test_g15_recursive_searches_denied | tests/guard.rs::test_g15_recursive_searches_denied | same |
| claude/hooks/test_guard.py::test_g15_searches_allowed | tests/guard.rs::test_g15_searches_allowed | same |
| claude/hooks/test_guard.py::test_g15_ps_on_macos | tests/guard.rs::test_g15_ps_on_macos | same |
| claude/hooks/test_guard.py::test_g15_grep_tool | tests/guard.rs::test_g15_grep_tool | same |
| claude/hooks/test_guard.py::test_g15_grep_tool_is_in_the_hook_matcher | tests/guard.rs::test_g15_grep_tool_is_in_the_hook_matcher | reads claude/settings.json from the repo |
| claude/hooks/test_guard.py::test_g15_letter_case_on_macos | tests/guard.rs::test_g15_letter_case_on_macos | |
| claude/hooks/test_guard.py::test_g15_ln_install_and_dd | tests/guard.rs::test_g15_ln_install_and_dd | |
| claude/hooks/test_guard.py::test_g15_off_switch_and_fail_open | tests/guard.rs::test_g15_off_switch_and_fail_open | the last check (os.path.realpath mocked to raise) has no counterpart in a binary: not ported |
| claude/hooks/test_guard.py::test_g15_through_pretool | tests/guard.rs::test_g15_through_pretool | |
| claude/hooks/test_guard.py::test_g13_fires_at_exactly_eight | tests/guard.rs::test_g13_fires_at_exactly_eight | |
| claude/hooks/test_guard.py::test_g13_off_in_the_desktop_app | tests/guard.rs::test_g13_off_in_the_desktop_app | |
| claude/hooks/test_guard.py::test_g13_without_tool_use_id_counts_all | tests/guard.rs::test_g13_without_tool_use_id_counts_all | |
| claude/hooks/test_guard.py::test_g13_parallel_calls_each_see_their_own_count | tests/guard.rs::test_g13_parallel_calls_each_see_their_own_count | |
| claude/hooks/test_guard.py::test_g13_text_resets_and_new_streak_starts | tests/guard.rs::test_g13_text_resets_and_new_streak_starts | |
| claude/hooks/test_guard.py::test_g13_empty_text_does_not_reset | tests/guard.rs::test_g13_empty_text_does_not_reset | |
| claude/hooks/test_guard.py::test_g13_stops_at_last_human_prompt | tests/guard.rs::test_g13_stops_at_last_human_prompt | |
| claude/hooks/test_guard.py::test_g13_main_thread_only_and_off_switch | tests/guard.rs::test_g13_main_thread_only_and_off_switch | |
| claude/hooks/test_guard.py::test_g13_fail_open | tests/guard.rs::test_g13_fail_open | the two direct `guard.posttool` calls and the `reverse_lines` mock have no counterpart in a binary; the missing-transcript inputs run through it |
| claude/hooks/test_pace.py::test_budget_per_weekday | src/pace_core.rs::test_budget_per_weekday | unit test with an injected clock (the CLI has no clock flag) |
| claude/hooks/test_pace.py::test_sunday_stays_at_100 | src/pace_core.rs::test_sunday_stays_at_100 | |
| claude/hooks/test_pace.py::test_week_rollover | src/pace_core.rs::test_week_rollover | |
| claude/hooks/test_pace.py::test_monday_before_six_belongs_to_previous_week | src/pace_core.rs::test_monday_before_six_belongs_to_previous_week | |
| claude/hooks/test_pace.py::test_week_start_from_resets_at | src/pace_core.rs::test_week_start_from_resets_at | |
| claude/hooks/test_pace.py::test_unknown_without_data | src/pace_core.rs::test_unknown_without_data | |
| claude/hooks/test_pace.py::test_fresh_c1_wins | src/pace_core.rs::test_fresh_c1_wins | |
| claude/hooks/test_pace.py::test_stale_c1_falls_back_to_estimate | src/pace_core.rs::test_stale_c1_falls_back_to_estimate | |
| claude/hooks/test_pace.py::test_c1_with_past_reset_is_not_fresh | src/pace_core.rs::test_c1_with_past_reset_is_not_fresh | |
| claude/hooks/test_pace.py::test_corrupt_c1_is_ignored | src/pace_core.rs::test_corrupt_c1_is_ignored | |
| claude/hooks/test_pace.py::test_c1_without_seven_day_is_ignored | src/pace_core.rs::test_c1_without_seven_day_is_ignored | |
| claude/hooks/test_pace.py::test_states | src/pace_core.rs::test_states | |
| claude/hooks/test_pace.py::test_critical_even_on_saturday | src/pace_core.rs::test_critical_even_on_saturday | |
| claude/hooks/test_pace.py::test_error_gives_unknown | src/pace_core.rs::test_error_gives_unknown | the `_read_json` mock becomes a cached result whose `files` is not a number (fails inside the status read) |
| claude/hooks/test_pace.py::test_weight_formula_and_percent | src/pace_core.rs::test_weight_formula_and_percent | |
| claude/hooks/test_pace.py::test_default_week_weight | src/pace_core.rs::test_default_week_weight | |
| claude/hooks/test_pace.py::test_cost_per_model_family | src/pace_core.rs::test_cost_per_model_family | |
| claude/hooks/test_pace.py::test_cache_write_split_5m_and_1h | src/pace_core.rs::test_cache_write_split_5m_and_1h | |
| claude/hooks/test_pace.py::test_cache_write_without_split_counts_as_1h | src/pace_core.rs::test_cache_write_without_split_counts_as_1h | |
| claude/hooks/test_pace.py::test_old_cache_formats_are_rebuilt_or_ignored | src/pace_core.rs::test_old_cache_formats_are_rebuilt_or_ignored | |
| claude/hooks/test_pace.py::test_dedupe_streamed_records_last_wins | src/pace_core.rs::test_dedupe_streamed_records_last_wins | |
| claude/hooks/test_pace.py::test_same_id_in_two_files_counts_per_file | src/pace_core.rs::test_same_id_in_two_files_counts_per_file | |
| claude/hooks/test_pace.py::test_free_and_unknown_models_are_skipped | src/pace_core.rs::test_free_and_unknown_models_are_skipped | |
| claude/hooks/test_pace.py::test_old_messages_are_skipped | src/pace_core.rs::test_old_messages_are_skipped | |
| claude/hooks/test_pace.py::test_corrupt_and_odd_lines_are_skipped | src/pace_core.rs::test_corrupt_and_odd_lines_are_skipped | |
| claude/hooks/test_pace.py::test_null_token_fields | src/pace_core.rs::test_null_token_fields | |
| claude/hooks/test_pace.py::test_subagent_files_are_counted | src/pace_core.rs::test_subagent_files_are_counted | |
| claude/hooks/test_pace.py::test_incremental_scan_reads_only_new_bytes | src/pace_core.rs::test_incremental_scan_reads_only_new_bytes | |
| claude/hooks/test_pace.py::test_partial_last_line_is_left_for_next_time | src/pace_core.rs::test_partial_last_line_is_left_for_next_time | |
| claude/hooks/test_pace.py::test_truncated_file_is_rescanned | src/pace_core.rs::test_truncated_file_is_rescanned | |
| claude/hooks/test_pace.py::test_status_uses_cached_result_under_120s | src/pace_core.rs::test_status_uses_cached_result_under_120s | |
| claude/hooks/test_pace.py::test_status_ignores_result_from_another_week | src/pace_core.rs::test_status_ignores_result_from_another_week | |
| claude/hooks/test_pace.py::test_no_transcripts_is_unknown | src/pace_core.rs::test_no_transcripts_is_unknown | |
| claude/hooks/test_pace.py::test_budget_stops_and_next_call_continues | src/pace_core.rs::test_budget_stops_and_next_call_continues | |
| claude/hooks/test_pace.py::test_budget_makes_progress_every_call | src/pace_core.rs::test_budget_makes_progress_every_call | |
| claude/hooks/test_pace.py::test_fresh_c1_stores_week_weight | src/pace_core.rs::test_fresh_c1_stores_week_weight | |
| claude/hooks/test_pace.py::test_low_usage_does_not_calibrate | src/pace_core.rs::test_low_usage_does_not_calibrate | |
| claude/hooks/test_pace.py::test_stale_c1_does_not_calibrate | src/pace_core.rs::test_stale_c1_does_not_calibrate | |
| claude/hooks/test_pace.py::test_incomplete_scan_does_not_calibrate | src/pace_core.rs::test_incomplete_scan_does_not_calibrate | |
| claude/hooks/test_pace.py::test_estimate_uses_calibration | src/pace_core.rs::test_estimate_uses_calibration | |
| claude/hooks/test_pace.py::test_seed | src/pace_core.rs::test_seed | |
| claude/hooks/test_pace.py::test_seed_needs_a_complete_refresh | src/pace_core.rs::test_seed_needs_a_complete_refresh | the mocked incomplete `refresh` has no counterpart; the test covers the empty-week refusal and the stored case. The incomplete-scan refusal is not tested |
| claude/hooks/test_pace.py::test_seed_rejects_bad_values | src/pace_core.rs::test_seed_rejects_bad_values | |
| claude/hooks/test_pace.py::test_corrupt_calibration_uses_default | src/pace_core.rs::test_corrupt_calibration_uses_default | |
| claude/hooks/test_pace.py::test_hit_sets_calibration | src/pace_core.rs::test_hit_sets_calibration | |
| claude/hooks/test_pace.py::test_hit_in_a_second_file_and_subagent_counts | src/pace_core.rs::test_hit_in_a_second_file_and_subagent_counts | |
| claude/hooks/test_pace.py::test_no_hit_keeps_default | src/pace_core.rs::test_no_hit_keeps_default | |
| claude/hooks/test_pace.py::test_a_reply_that_quotes_the_phrase_is_not_a_hit | src/pace_core.rs::test_a_reply_that_quotes_the_phrase_is_not_a_hit | |
| claude/hooks/test_pace.py::test_phrase_is_case_insensitive_and_system_records_count | src/pace_core.rs::test_phrase_is_case_insensitive_and_system_records_count | |
| claude/hooks/test_pace.py::test_first_hit_of_the_week_wins | src/pace_core.rs::test_first_hit_of_the_week_wins | |
| claude/hooks/test_pace.py::test_cheap_hit_stores_nothing | src/pace_core.rs::test_cheap_hit_stores_nothing | |
| claude/hooks/test_pace.py::test_partial_scan_stores_nothing_until_complete | src/pace_core.rs::test_partial_scan_stores_nothing_until_complete | |
| claude/hooks/test_pace.py::test_newer_seed_wins | src/pace_core.rs::test_newer_seed_wins | |
| claude/hooks/test_pace.py::test_newer_hit_wins_over_older_seed | src/pace_core.rs::test_newer_hit_wins_over_older_seed | |
| claude/hooks/test_pace.py::test_hit_is_stored_once | src/pace_core.rs::test_hit_is_stored_once | |
| claude/hooks/test_pace.py::test_no_hit_this_week_keeps_last_week_calibration | src/pace_core.rs::test_no_hit_this_week_keeps_last_week_calibration | |
| claude/hooks/test_pace.py::test_backfill_finds_the_previous_week_hit_once | src/pace_core.rs::test_backfill_finds_the_previous_week_hit_once | |
| claude/hooks/test_pace.py::test_backfill_runs_once_without_a_hit | src/pace_core.rs::test_backfill_runs_once_without_a_hit | |
| claude/hooks/test_pace.py::test_no_backfill_with_a_warm_calibration | src/pace_core.rs::test_no_backfill_with_a_warm_calibration | |
| claude/hooks/test_pace.py::test_status_prints_json | tests/pace.rs::test_status_prints_json | |
| claude/hooks/test_pace.py::test_status_cli_estimates_from_transcripts | tests/pace.rs::test_status_cli_estimates_from_transcripts | transcript mtime is the real time (the Python helper pinned it) |
| claude/hooks/test_pace.py::test_refresh_and_seed | tests/pace.rs::test_refresh_and_seed | same |
| claude/hooks/test_pace.py::test_bad_usage_exits_2 | tests/pace.rs::test_bad_usage_exits_2 | |
| claude/hooks/test_pace.py::test_epoch_forms | src/pace_core.rs::test_epoch_forms | |
| cursor/hooks/test_guard_cursor.py::test_reuses_guard_limits | tests/guard_cursor.rs::test_reuses_guard_limits | the Rust hook keeps its own copies of the limits: the test checks them at 300K and 350K |
| cursor/hooks/test_guard_cursor.py::test_deny_above_spawn_context | tests/guard_cursor.rs::test_deny_above_spawn_context | |
| cursor/hooks/test_guard_cursor.py::test_allow_at_or_below_spawn_context | tests/guard_cursor.rs::test_allow_at_or_below_spawn_context | |
| cursor/hooks/test_guard_cursor.py::test_subagent_uses_parent_conversation | tests/guard_cursor.rs::test_subagent_uses_parent_conversation | |
| cursor/hooks/test_guard_cursor.py::test_compact_clears_record | tests/guard_cursor.rs::test_compact_clears_record | |
| cursor/hooks/test_guard_cursor.py::test_transcript_usage_wins_over_record | tests/guard_cursor.rs::test_transcript_usage_wins_over_record | |
| cursor/hooks/test_guard_cursor.py::test_app_store_size_denies_and_nudges | tests/guard_cursor.rs::test_app_store_size_denies_and_nudges | needs the `sqlite3` tool to build the fixture; returns early without it |
| cursor/hooks/test_guard_cursor.py::test_app_store_small_size_wins_over_stale_record | tests/guard_cursor.rs::test_app_store_small_size_wins_over_stale_record | same |
| cursor/hooks/test_guard_cursor.py::test_app_store_breakdown_fallback_and_parent_conversation | tests/guard_cursor.rs::test_app_store_breakdown_fallback_and_parent_conversation | same |
| cursor/hooks/test_guard_cursor.py::test_app_store_bad_rows_and_missing_conversation_fall_back | tests/guard_cursor.rs::test_app_store_bad_rows_and_missing_conversation_fall_back | same |
| cursor/hooks/test_guard_cursor.py::test_app_store_not_a_database_fails_open | tests/guard_cursor.rs::test_app_store_not_a_database_fails_open | |
| cursor/hooks/test_guard_cursor.py::test_response_usage_recorded | tests/guard_cursor.rs::test_response_usage_recorded | |
| cursor/hooks/test_guard_cursor.py::test_pretooluse_task_gated_other_tools_ignored | tests/guard_cursor.rs::test_pretooluse_task_gated_other_tools_ignored | |
| cursor/hooks/test_guard_cursor.py::test_summed_input_tokens_ignored | tests/guard_cursor.rs::test_summed_input_tokens_ignored | |
| cursor/hooks/test_guard_cursor.py::test_response_without_usage_is_noop | tests/guard_cursor.rs::test_response_without_usage_is_noop | |
| cursor/hooks/test_guard_cursor.py::test_prompt_below_threshold | tests/guard_cursor.rs::test_prompt_below_threshold | |
| cursor/hooks/test_guard_cursor.py::test_nudge_cadence | tests/guard_cursor.rs::test_nudge_cadence | |
| cursor/hooks/test_guard_cursor.py::test_nudge_resets_below_threshold_and_conversations_independent | tests/guard_cursor.rs::test_nudge_resets_below_threshold_and_conversations_independent | |
| cursor/hooks/test_guard_cursor.py::test_fail_open | tests/guard_cursor.rs::test_fail_open | |
| cursor/hooks/test_guard_cursor.py::test_missing_guard_module_fails_open | - | Python only: the hook loaded `guard.py` from a sibling file; the Rust binary has no module to lose |
| cursor/hooks/test_guard_cursor.py::test_state_isolated_in_tmpdir | tests/guard_cursor.rs::test_state_isolated_in_tmpdir | |
| cursor/hooks/test_guard_cursor.py::test_state_pruned | tests/guard_cursor.rs::test_state_pruned | MAX_STATE_FILES and MAX_STATE_AGE written as literals |
| cursor/hooks/test_guard_cursor.py::test_conv_key | tests/guard_cursor.rs::test_conv_key | `conv_key` is private to the binary: checked through state file names and which conversation each mode reads (unit tests in `src/bin/guard_cursor.rs` cover the function) |
| scripts/test_agent_config.py::TranslatePermissionTest::test_bash_prefix_forms | src/agent_config_tests.rs::test_bash_prefix_forms |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_bash_prefix_edge_cases_are_skipped | src/agent_config_tests.rs::test_bash_prefix_edge_cases_are_skipped |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_bash_exact_form | src/agent_config_tests.rs::test_bash_exact_form |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_bash_exact_without_args_or_with_star_is_skipped | src/agent_config_tests.rs::test_bash_exact_without_args_or_with_star_is_skipped |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_read_edit_write_allow_keeps_relative_path | src/agent_config_tests.rs::test_read_edit_write_allow_keeps_relative_path |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_edit_and_write_both_become_write | src/agent_config_tests.rs::test_edit_and_write_both_become_write |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_deny_relative_path_gets_double_star_prefix | src/agent_config_tests.rs::test_deny_relative_path_gets_double_star_prefix |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_deny_absolute_home_and_glob_paths_unchanged | src/agent_config_tests.rs::test_deny_absolute_home_and_glob_paths_unchanged |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_webfetch_domain | src/agent_config_tests.rs::test_webfetch_domain |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_unknown_or_malformed_rules | src/agent_config_tests.rs::test_unknown_or_malformed_rules |  |
| scripts/test_agent_config.py::TranslatePermissionTest::test_surrounding_whitespace_is_ignored | src/agent_config_tests.rs::test_surrounding_whitespace_is_ignored |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_basic | src/agent_config_tests.rs::test_basic |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_no_frontmatter | src/agent_config_tests.rs::test_no_frontmatter |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_value_containing_colon_and_dashes_in_key | src/agent_config_tests.rs::test_value_containing_colon_and_dashes_in_key |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_comments_skipped_and_indented_line_continues_value | src/agent_config_tests.rs::test_comments_skipped_and_indented_line_continues_value |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_quoted_values | src/agent_config_tests.rs::test_quoted_values |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_folded_and_literal_block_scalars | src/agent_config_tests.rs::test_folded_and_literal_block_scalars |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_list_under_empty_key_is_joined | src/agent_config_tests.rs::test_list_under_empty_key_is_joined |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_crlf_frontmatter | src/agent_config_tests.rs::test_crlf_frontmatter |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_closing_fence_without_trailing_newline | src/agent_config_tests.rs::test_closing_fence_without_trailing_newline |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_unterminated_frontmatter_is_plain_text | src/agent_config_tests.rs::test_unterminated_frontmatter_is_plain_text |  |
| scripts/test_agent_config.py::SplitFrontmatterTest::test_empty_value | src/agent_config_tests.rs::test_empty_value |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_tilde | src/agent_config_tests.rs::test_tilde |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_mdc_and_stamp | src/agent_config_tests.rs::test_mdc_and_stamp |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_atomic_write_creates_parents_and_sets_mode | src/agent_config_tests.rs::test_atomic_write_creates_parents_and_sets_mode |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_atomic_write_default_mode_for_new_file | src/agent_config_tests.rs::test_atomic_write_default_mode_for_new_file |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_load_json | src/agent_config_tests.rs::test_load_json |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_setting_reads_like_setup_lib | src/agent_config_tests.rs::test_setting_reads_like_setup_lib |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_default_profile_order | src/agent_config_tests.rs::test_default_profile_order |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_default_profile_owner_when_overlay_exists | src/agent_config_tests.rs::test_default_profile_owner_when_overlay_exists |  |
| scripts/test_agent_config.py::SmallHelpersTest::test_default_profile_garbage_in_setting | src/agent_config_tests.rs::test_default_profile_garbage_in_setting |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_owner_gets_source_then_overlay | src/agent_config_tests.rs::test_owner_gets_source_then_overlay |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_owner_without_overlay_gets_the_source | src/agent_config_tests.rs::test_owner_without_overlay_gets_the_source |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_shared_never_reads_the_overlay | src/agent_config_tests.rs::test_shared_never_reads_the_overlay |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_overlay_that_is_a_directory_is_ignored | src/agent_config_tests.rs::test_overlay_that_is_a_directory_is_ignored |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_user_rules_text_has_cursor_note_first | src/agent_config_tests.rs::test_user_rules_text_has_cursor_note_first |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_cursor_note_routes_named_agents_through_general_purpose | src/agent_config_tests.rs::test_cursor_note_routes_named_agents_through_general_purpose |  |
| scripts/test_agent_config.py::OwnerOverlayTest::test_user_rules_text_appends_output_style_after_the_overlay | src/agent_config_tests.rs::test_user_rules_text_appends_output_style_after_the_overlay |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_creates_file_and_parents | src/agent_config_tests.rs::test_creates_file_and_parents |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_rewrites_our_own_file_when_text_changes | src/agent_config_tests.rs::test_rewrites_our_own_file_when_text_changes |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_unchanged_text_is_skipped | src/agent_config_tests.rs::test_unchanged_text_is_skipped |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_user_file_without_marker_is_left_alone | src/agent_config_tests.rs::test_user_file_without_marker_is_left_alone |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_symlink_is_left_alone_even_if_target_has_marker | src/agent_config_tests.rs::test_symlink_is_left_alone_even_if_target_has_marker |  |
| scripts/test_agent_config.py::WriteGeneratedTest::test_dry_run_writes_nothing | src/agent_config_tests.rs::test_write_generated_dry_run_writes_nothing |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_removes_marked_file | src/agent_config_tests.rs::test_removes_marked_file |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_keeps_user_file | src/agent_config_tests.rs::test_keeps_user_file |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_missing_file_is_fine | src/agent_config_tests.rs::test_missing_file_is_fine |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_symlink_is_kept | src/agent_config_tests.rs::test_symlink_is_kept |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_directory_is_ignored | src/agent_config_tests.rs::test_directory_is_ignored |  |
| scripts/test_agent_config.py::RemoveGeneratedTest::test_dry_run_keeps_file | src/agent_config_tests.rs::test_dry_run_keeps_file |  |
| scripts/test_agent_config.py::CursorRulesTest::test_global_rule_for_each_profile | src/agent_config_tests.rs::test_global_rule_for_each_profile |  |
| scripts/test_agent_config.py::CursorRulesTest::test_rerun_is_idempotent | src/agent_config_tests.rs::test_cursor_rules_rerun_is_idempotent |  |
| scripts/test_agent_config.py::CursorRulesTest::test_output_style_rule_written_then_removed | src/agent_config_tests.rs::test_output_style_rule_written_then_removed |  |
| scripts/test_agent_config.py::CursorRulesTest::test_user_owned_style_rule_is_not_removed | src/agent_config_tests.rs::test_user_owned_style_rule_is_not_removed |  |
| scripts/test_agent_config.py::CursorRulesTest::test_user_owned_global_rule_is_not_overwritten | src/agent_config_tests.rs::test_user_owned_global_rule_is_not_overwritten |  |
| scripts/test_agent_config.py::CursorRulesTest::test_unknown_style_name_means_no_style_file | src/agent_config_tests.rs::test_unknown_style_name_means_no_style_file |  |
| scripts/test_agent_config.py::CursorRulesTest::test_style_matched_by_file_stem | src/agent_config_tests.rs::test_style_matched_by_file_stem |  |
| scripts/test_agent_config.py::CursorRulesTest::test_mark_present_removes_our_files_and_skips_when_hash_matches | src/agent_config_tests.rs::test_mark_present_removes_our_files_and_skips_when_hash_matches |  |
| scripts/test_agent_config.py::CursorRulesTest::test_mark_present_warns_when_rules_changed | src/agent_config_tests.rs::test_mark_present_warns_when_rules_changed |  |
| scripts/test_agent_config.py::CursorRulesTest::test_mark_present_leaves_user_files_alone | src/agent_config_tests.rs::test_mark_present_leaves_user_files_alone |  |
| scripts/test_agent_config.py::CursorRulesTest::test_mark_hash_depends_on_profile | src/agent_config_tests.rs::test_mark_hash_depends_on_profile |  |
| scripts/test_agent_config.py::CursorRulesTest::test_removing_the_mark_brings_the_file_copy_back | src/agent_config_tests.rs::test_removing_the_mark_brings_the_file_copy_back |  |
| scripts/test_agent_config.py::YamlStrTest::test_plain_when_safe | src/agent_config_tests.rs::test_plain_when_safe |  |
| scripts/test_agent_config.py::YamlStrTest::test_quotes_when_needed | src/agent_config_tests.rs::test_quotes_when_needed |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_description_with_colon_is_quoted_and_round_trips | src/agent_config_tests.rs::test_description_with_colon_is_quoted_and_round_trips | `split_frontmatter` anchors the key match at the line start like `re.match`, so the indented `  Use it: after edits` stays a continuation line |
| scripts/test_agent_config.py::CursorAgentsTest::test_converts_frontmatter | src/agent_config_tests.rs::test_cursor_agents_converts_frontmatter |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_write_capable_tools_are_not_readonly | src/agent_config_tests.rs::test_write_capable_tools_are_not_readonly |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_name_falls_back_to_file_stem_and_readme_is_skipped | src/agent_config_tests.rs::test_name_falls_back_to_file_stem_and_readme_is_skipped |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_stale_generated_agent_removed_user_agent_kept | src/agent_config_tests.rs::test_cursor_agents_stale_generated_agent_removed_user_agent_kept |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_user_agent_with_same_name_is_not_overwritten | src/agent_config_tests.rs::test_cursor_agents_user_agent_with_same_name_is_not_overwritten |  |
| scripts/test_agent_config.py::CursorAgentsTest::test_no_agents_dir_is_fine | src/agent_config_tests.rs::test_cursor_agents_no_agents_dir_is_fine |  |
| scripts/test_agent_config.py::SaveJsonTest::test_backup_made_before_change | src/agent_config_tests.rs::test_backup_made_before_change |  |
| scripts/test_agent_config.py::SaveJsonTest::test_no_backup_for_new_file | src/agent_config_tests.rs::test_no_backup_for_new_file |  |
| scripts/test_agent_config.py::SaveJsonTest::test_unchanged_is_skipped | src/agent_config_tests.rs::test_unchanged_is_skipped |  |
| scripts/test_agent_config.py::SaveJsonTest::test_symlink_untouched | src/agent_config_tests.rs::test_symlink_untouched |  |
| scripts/test_agent_config.py::SaveJsonTest::test_private_mode | src/agent_config_tests.rs::test_private_mode |  |
| scripts/test_agent_config.py::SaveJsonTest::test_dry_run | src/agent_config_tests.rs::test_dry_run |  |
| scripts/test_agent_config.py::CursorHooksTest::test_creates_file_with_all_events | src/agent_config_tests.rs::test_creates_file_with_all_events |  |
| scripts/test_agent_config.py::CursorHooksTest::test_user_entries_and_keys_are_kept | src/agent_config_tests.rs::test_user_entries_and_keys_are_kept |  |
| scripts/test_agent_config.py::CursorHooksTest::test_our_entry_keeps_its_place_between_user_entries | src/agent_config_tests.rs::test_our_entry_keeps_its_place_between_user_entries |  |
| scripts/test_agent_config.py::CursorHooksTest::test_rerun_is_idempotent_and_makes_no_backup | src/agent_config_tests.rs::test_rerun_is_idempotent_and_makes_no_backup |  |
| scripts/test_agent_config.py::CursorHooksTest::test_old_entries_for_a_moved_script_are_replaced | src/agent_config_tests.rs::test_old_entries_for_a_moved_script_are_replaced |  |
| scripts/test_agent_config.py::CursorHooksTest::test_missing_guard_removes_our_entries_and_empty_events | src/agent_config_tests.rs::test_missing_guard_removes_our_entries_and_empty_events |  |
| scripts/test_agent_config.py::CursorHooksTest::test_missing_guard_and_no_file_still_writes_skeleton | src/agent_config_tests.rs::test_missing_guard_and_no_file_still_writes_skeleton |  |
| scripts/test_agent_config.py::CursorHooksTest::test_invalid_json_left_untouched | src/agent_config_tests.rs::test_cursor_hooks_invalid_json_left_untouched |  |
| scripts/test_agent_config.py::CursorHooksTest::test_non_object_hooks_left_untouched | src/agent_config_tests.rs::test_non_object_hooks_left_untouched |  |
| scripts/test_agent_config.py::CursorHooksTest::test_path_with_spaces_is_quoted | src/agent_config_tests.rs::test_path_with_spaces_is_quoted | the script path comes from `DOTFILES_REPO`, so the test puts the repo in a folder with a space (Python patched `GUARD`) |
| scripts/test_agent_config.py::CursorHooksTest::test_dry_run_writes_nothing | src/agent_config_tests.rs::test_cursor_hooks_dry_run_writes_nothing |  |
| scripts/test_agent_config.py::CursorMcpTest::test_adds_ours_to_new_file | src/agent_config_tests.rs::test_adds_ours_to_new_file |  |
| scripts/test_agent_config.py::CursorMcpTest::test_user_entry_of_same_name_wins_and_others_kept | src/agent_config_tests.rs::test_user_entry_of_same_name_wins_and_others_kept |  |
| scripts/test_agent_config.py::CursorMcpTest::test_idempotent | src/agent_config_tests.rs::test_idempotent |  |
| scripts/test_agent_config.py::CursorMcpTest::test_no_source_file_keeps_user_file_content | src/agent_config_tests.rs::test_no_source_file_keeps_user_file_content |  |
| scripts/test_agent_config.py::CursorMcpTest::test_invalid_json_left_untouched | src/agent_config_tests.rs::test_cursor_mcp_invalid_json_left_untouched |  |
| scripts/test_agent_config.py::CursorMcpTest::test_skips_without_opt_in_on_every_profile | src/agent_config_tests.rs::test_skips_without_opt_in_on_every_profile |  |
| scripts/test_agent_config.py::CursorMcpTest::test_opt_in_setting | src/agent_config_tests.rs::test_opt_in_setting |  |
| scripts/test_agent_config.py::CursorMcpTest::test_off_setting | src/agent_config_tests.rs::test_off_setting |  |
| scripts/test_agent_config.py::CursorMcpTest::test_env_wins_over_setting | src/agent_config_tests.rs::test_env_wins_over_setting |  |
| scripts/test_agent_config.py::CursorMcpTest::test_change_makes_backup | src/agent_config_tests.rs::test_change_makes_backup |  |
| scripts/test_agent_config.py::CliConfigTest::test_defaults_on_empty_home | src/agent_config_tests.rs::test_defaults_on_empty_home |  |
| scripts/test_agent_config.py::CliConfigTest::test_user_values_win_but_attribution_is_forced_off | src/agent_config_tests.rs::test_user_values_win_but_attribution_is_forced_off |  |
| scripts/test_agent_config.py::CliConfigTest::test_translated_rules_added_and_user_rules_kept | src/agent_config_tests.rs::test_translated_rules_added_and_user_rules_kept |  |
| scripts/test_agent_config.py::CliConfigTest::test_sidecar_records_what_we_added | src/agent_config_tests.rs::test_sidecar_records_what_we_added |  |
| scripts/test_agent_config.py::CliConfigTest::test_rerun_is_idempotent | src/agent_config_tests.rs::test_cli_config_rerun_is_idempotent |  |
| scripts/test_agent_config.py::CliConfigTest::test_no_duplicates_when_user_already_has_the_rule | src/agent_config_tests.rs::test_no_duplicates_when_user_already_has_the_rule |  |
| scripts/test_agent_config.py::CliConfigTest::test_stale_rule_we_owned_is_removed_user_rules_survive | src/agent_config_tests.rs::test_stale_rule_we_owned_is_removed_user_rules_survive |  |
| scripts/test_agent_config.py::CliConfigTest::test_rule_never_owned_is_never_removed | src/agent_config_tests.rs::test_rule_never_owned_is_never_removed |  |
| scripts/test_agent_config.py::CliConfigTest::test_deny_rules_are_tracked_separately_from_allow | src/agent_config_tests.rs::test_deny_rules_are_tracked_separately_from_allow |  |
| scripts/test_agent_config.py::CliConfigTest::test_user_deleting_our_rule_gets_it_back | src/agent_config_tests.rs::test_user_deleting_our_rule_gets_it_back |  |
| scripts/test_agent_config.py::CliConfigTest::test_user_rule_equal_to_ours_is_kept_once | src/agent_config_tests.rs::test_user_rule_equal_to_ours_is_kept_once |  |
| scripts/test_agent_config.py::CliConfigTest::test_corrupt_sidecar_is_survivable | src/agent_config_tests.rs::test_corrupt_sidecar_is_survivable |  |
| scripts/test_agent_config.py::CliConfigTest::test_sidecar_not_written_when_config_is_a_symlink | src/agent_config_tests.rs::test_sidecar_not_written_when_config_is_a_symlink |  |
| scripts/test_agent_config.py::CliConfigTest::test_invalid_config_json_left_untouched_and_no_sidecar | src/agent_config_tests.rs::test_invalid_config_json_left_untouched_and_no_sidecar |  |
| scripts/test_agent_config.py::CliConfigTest::test_dry_run_writes_neither_config_nor_sidecar | src/agent_config_tests.rs::test_dry_run_writes_neither_config_nor_sidecar |  |
| scripts/test_agent_config.py::CliConfigTest::test_statusline_added_only_when_script_exists | src/agent_config_tests.rs::test_statusline_added_only_when_launcher_exists | the launcher `rust-bin` replaces `statusline.sh` |
| scripts/test_agent_config.py::CliConfigTest::test_user_statusline_wins | src/agent_config_tests.rs::test_user_statusline_wins |  |
| scripts/test_agent_config.py::CliConfigTest::test_missing_source_settings_is_fine | src/agent_config_tests.rs::test_missing_source_settings_is_fine |  |
| scripts/test_agent_config.py::CmdCursorTest::test_full_run_creates_everything_and_never_writes_claude_dir | tests/agent_config.rs::test_full_run_creates_everything_and_never_writes_claude_dir |  |
| scripts/test_agent_config.py::CmdCursorTest::test_owner_profile_gets_overlay_but_mcp_stays_opt_in | tests/agent_config.rs::test_owner_profile_gets_overlay_but_mcp_stays_opt_in |  |
| scripts/test_agent_config.py::CmdCursorTest::test_owner_rule_file_is_private_and_names_both_sources | tests/agent_config.rs::test_owner_rule_file_is_private_and_names_both_sources |  |
| scripts/test_agent_config.py::CmdCursorTest::test_owner_rule_file_turns_private_when_the_text_is_unchanged | tests/agent_config.rs::test_owner_rule_file_turns_private_when_the_text_is_unchanged |  |
| scripts/test_agent_config.py::CmdCursorTest::test_owner_without_overlay_warns_and_names_one_source | tests/agent_config.rs::test_owner_without_overlay_warns_and_names_one_source |  |
| scripts/test_agent_config.py::CmdCursorTest::test_second_run_is_all_skips | tests/agent_config.rs::test_second_run_is_all_skips |  |
| scripts/test_agent_config.py::CmdCursorTest::test_dry_run_leaves_home_empty | tests/agent_config.rs::test_dry_run_leaves_home_empty |  |
| scripts/test_agent_config.py::CmdCursorTest::test_one_failing_part_does_not_stop_the_others | tests/agent_config.rs::test_one_failing_part_does_not_stop_the_others | each part runs under `run_part` (panic caught, `skipped a part: <why>`, exit 1); the Rust reason text differs from Python's exception name |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_converts_frontmatter | tests/agent_config.rs::test_opencode_agents_converts_frontmatter |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_readonly_agent_gets_edit_deny | tests/agent_config.rs::test_readonly_agent_gets_edit_deny |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_write_capable_agent_does_not_deny_edit | tests/agent_config.rs::test_write_capable_agent_does_not_deny_edit |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_name_from_file_stem_and_readme_skipped | tests/agent_config.rs::test_name_from_file_stem_and_readme_skipped |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_stale_generated_agent_removed_user_agent_kept | tests/agent_config.rs::test_opencode_agents_stale_generated_agent_removed_user_agent_kept |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_user_agent_with_same_name_is_not_overwritten | tests/agent_config.rs::test_opencode_agents_user_agent_with_same_name_is_not_overwritten |  |
| scripts/test_agent_config.py::OpencodeAgentsTest::test_no_agents_dir_is_fine | tests/agent_config.rs::test_opencode_agents_no_agents_dir_is_fine |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_translated_permissions_added | tests/agent_config.rs::test_translated_permissions_added |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_skill_and_ask_allowed | tests/agent_config.rs::test_skill_and_ask_allowed |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_user_keys_win_ours_added | tests/agent_config.rs::test_user_keys_win_ours_added |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_mcp_added_when_opt_in | tests/agent_config.rs::test_mcp_added_when_opt_in |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_mcp_user_entry_wins | tests/agent_config.rs::test_mcp_user_entry_wins |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_mcp_skipped_when_off | tests/agent_config.rs::test_mcp_skipped_when_off |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_mcp_env_wins_over_setting | tests/agent_config.rs::test_mcp_env_wins_over_setting |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_mcp_merges_into_existing_mcp_block | tests/agent_config.rs::test_mcp_merges_into_existing_mcp_block |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_invalid_json_left_untouched | tests/agent_config.rs::test_opencode_config_invalid_json_left_untouched |  |
| scripts/test_agent_config.py::OpencodeConfigTest::test_dry_run_writes_nothing | tests/agent_config.rs::test_opencode_config_dry_run_writes_nothing |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_sidecar_holds_the_rules_we_added | tests/agent_config.rs::test_sidecar_holds_the_rules_we_added |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_rule_removed_from_the_repo_goes_away_next_run | tests/agent_config.rs::test_rule_removed_from_the_repo_goes_away_next_run |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_a_rule_the_user_edited_is_not_taken_away | tests/agent_config.rs::test_a_rule_the_user_edited_is_not_taken_away |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_a_rule_the_user_added_is_never_touched | tests/agent_config.rs::test_a_rule_the_user_added_is_never_touched |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_dry_run_leaves_the_sidecar_alone | tests/agent_config.rs::test_dry_run_leaves_the_sidecar_alone |  |
| scripts/test_agent_config.py::OpencodeManagedPermissionsTest::test_a_symlinked_config_is_never_claimed | tests/agent_config.rs::test_a_symlinked_config_is_never_claimed |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_theme_file_holds_the_copper_palette | tests/agent_config.rs::test_theme_file_holds_the_copper_palette | the palette keys are compared with the shipped key list (the Python test read `COPPER_THEME`) |
| scripts/test_agent_config.py::OpencodeThemeTest::test_theme_has_every_key_the_opencode_loader_reads | tests/agent_config.rs::test_theme_has_every_key_the_opencode_loader_reads |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_every_colour_is_a_hex_string | tests/agent_config.rs::test_every_colour_is_a_hex_string |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_tui_names_the_theme_because_the_schema_takes_a_string | tests/agent_config.rs::test_tui_names_the_theme_because_the_schema_takes_a_string |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_user_theme_name_wins_and_is_reported | tests/agent_config.rs::test_user_theme_name_wins_and_is_reported |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_user_chosen_theme_is_restored_next_run | tests/agent_config.rs::test_user_chosen_theme_is_restored_next_run |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_other_tui_keys_survive | tests/agent_config.rs::test_other_tui_keys_survive |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_rerun_skips_an_unchanged_theme | tests/agent_config.rs::test_rerun_skips_an_unchanged_theme |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_invalid_tui_json_is_left_alone | tests/agent_config.rs::test_invalid_tui_json_is_left_alone |  |
| scripts/test_agent_config.py::OpencodeThemeTest::test_an_older_generated_tui_is_replaced | tests/agent_config.rs::test_an_older_generated_tui_is_replaced | |
| scripts/test_agent_config.py::OpencodeThemeTest::test_a_broken_inline_theme_without_our_marker_is_the_users | tests/agent_config.rs::test_a_broken_inline_theme_without_our_marker_is_the_users | also checks the object is printed the way Python prints it |
| scripts/test_agent_config.py::OpencodeRulesTest::test_shared_profile_carries_the_rules_and_the_note | tests/agent_config.rs::test_shared_profile_carries_the_rules_and_the_note |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_owner_profile_gets_the_private_overlay_at_0600 | tests/agent_config.rs::test_owner_profile_gets_the_private_overlay_at_0600 |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_owner_profile_without_the_file_warns | tests/agent_config.rs::test_owner_profile_without_the_file_warns |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_output_style_is_appended | tests/agent_config.rs::test_output_style_is_appended |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_user_own_agents_md_is_never_overwritten | tests/agent_config.rs::test_user_own_agents_md_is_never_overwritten |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_symlink_is_left_alone | tests/agent_config.rs::test_symlink_is_left_alone |  |
| scripts/test_agent_config.py::OpencodeRulesTest::test_rerun_is_idempotent | tests/agent_config.rs::test_opencode_rules_rerun_is_idempotent |  |
| scripts/test_agent_config.py::OpencodeProjectsTest::test_creates_project_opencode_and_agent_links | tests/agent_config.rs::test_creates_project_opencode_and_agent_links |  |
| scripts/test_agent_config.py::OpencodeProjectsTest::test_does_not_clobber_existing_project_config_or_agent | tests/agent_config.rs::test_does_not_clobber_existing_project_config_or_agent |  |
| scripts/test_agent_config.py::OpencodeProjectsTest::test_a_leading_tilde_means_the_home_folder | tests/agent_config.rs::test_a_leading_tilde_means_the_home_folder | |
| scripts/test_agent_config.py::OpencodeProjectsTest::test_a_relative_path_is_skipped_with_a_warning | tests/agent_config.rs::test_a_relative_path_is_skipped_with_a_warning |  |
| scripts/test_agent_config.py::CmdOpencodeTest::test_full_run_creates_agents_and_config_and_never_writes_claude_dir | tests/agent_config.rs::test_full_run_creates_agents_and_config_and_never_writes_claude_dir |  |
| scripts/test_agent_config.py::CmdUserRulesTest::test_print_mode_writes_text_to_stdout_only | tests/agent_config.rs::test_print_mode_writes_text_to_stdout_only | checks the Cursor note first and the rules last instead of calling `user_rules_text` |
| scripts/test_agent_config.py::CmdUserRulesTest::test_default_profile_comes_from_env | tests/agent_config.rs::test_default_profile_comes_from_env |  |
| scripts/test_agent_config.py::CmdUserRulesTest::test_copy_sends_text_to_clipboard_writes_mark_and_drops_file_copy | tests/agent_config.rs::test_copy_sends_text_to_clipboard_writes_mark_and_drops_file_copy | fake `pbcopy`, `wl-copy`, `xclip` and `xsel` on PATH write stdin to a file (Python mocked `subprocess.run`) |
| scripts/test_agent_config.py::CmdUserRulesTest::test_clipboard_command_picks_the_tool_for_the_platform | src/agent_config_tests.rs::test_clipboard_command_picks_the_tool_for_the_platform | unit test: Rust picks the tool with `cfg!` at compile time, so it checks the arm of the platform it runs on (macOS `pbcopy`; Linux `xclip` and none via PATH) |
| scripts/test_agent_config.py::CmdUserRulesTest::test_copy_without_a_clipboard_tool_prints_the_rules_and_fails | tests/agent_config.rs::test_copy_without_a_clipboard_tool_prints_the_rules_and_fails | an empty PATH stands in for the missing tool (Python mocked `clipboard_command`) |
| scripts/test_agent_config.py::CmdUserRulesTest::test_after_copy_cursor_rules_skips_then_warns_on_change | tests/agent_config.rs::test_after_copy_cursor_rules_skips_then_warns_on_change | runs `agent-config cursor` (Python called `cursor_rules` directly) |
| scripts/test_agent_config.py::CmdUserRulesTest::test_full_cursor_run_after_copy_creates_no_rule_files | tests/agent_config.rs::test_full_cursor_run_after_copy_creates_no_rule_files |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_merge_rules | src/agent_config_tests.rs::test_merge_rules | unit test |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_first_build_writes_a_real_file | tests/agent_config.rs::test_first_build_writes_a_real_file |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_layer_goes_on_top | tests/agent_config.rs::test_layer_goes_on_top |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_second_run_changes_nothing | tests/agent_config.rs::test_second_run_changes_nothing |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_claude_change_is_kept_in_the_layer | tests/agent_config.rs::test_claude_change_is_kept_in_the_layer |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_layout_only_rewrite_is_no_drift | tests/agent_config.rs::test_layout_only_rewrite_is_no_drift |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_upstream_change_flows_through | tests/agent_config.rs::test_upstream_change_flows_through |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_link_is_replaced_without_drift | tests/agent_config.rs::test_link_is_replaced_without_drift |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_bad_json_touches_nothing | tests/agent_config.rs::test_bad_json_touches_nothing |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_removed_union_item_warns | tests/agent_config.rs::test_removed_union_item_warns |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_promote_moves_a_key_to_the_repo | tests/agent_config.rs::test_promote_moves_a_key_to_the_repo |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_dry_run_writes_nothing | tests/agent_config.rs::test_claude_settings_dry_run_writes_nothing |  |
| scripts/test_agent_config.py::ClaudeSettingsTest::test_cli | tests/agent_config.rs::test_cli |  |
| scripts/test_agent_config.py::MainTest::test_requires_a_subcommand | tests/agent_config.rs::test_requires_a_subcommand | checks a non-zero exit (Python: `SystemExit`) |
| scripts/test_agent_config.py::MainTest::test_rejects_unknown_profile | tests/agent_config.rs::test_rejects_unknown_profile | clap `value_parser` on `--profile` of cursor, opencode and user-rules: exit 2 like argparse, clap's message text (`invalid value 'root' for '--profile <PROFILE>'`) |
| scripts/test_agent_config.py::MainTest::test_dry_flag_is_set_per_invocation | tests/agent_config.rs::test_dry_flag_is_set_per_invocation | runs two processes: the dry one writes nothing, the next one writes (Python read the module global `DRY`) |
| scripts/test_agent_config.py::QuietOutputTest::test_skip_counts_and_summary_prints_one_line | tests/agent_quiet_output.rs::test_skip_counts_and_summary_prints_one_line | `skip` and `summary` called directly, output taken with `out::capture`; a test file of its own, because the counters are process-wide |
| scripts/test_agent_config.py::QuietOutputTest::test_written_files_keep_their_line_and_join_the_count | tests/agent_quiet_output.rs::test_written_files_keep_their_line_and_join_the_count | as above |
| scripts/test_agent_config.py::QuietOutputTest::test_nothing_skipped_prints_no_count_line | tests/agent_quiet_output.rs::test_nothing_skipped_prints_no_count_line | as above |
| scripts/test_agent_config.py::QuietOutputTest::test_header_is_verbose_only | tests/agent_quiet_output.rs::test_header_is_verbose_only | as above |
| scripts/test_agent_config.py::QuietOutputTest::test_verbose_prints_every_skip_line | tests/agent_quiet_output.rs::test_verbose_prints_every_skip_line | as above |
| scripts/test_agent_config.py::QuietOutputTest::test_a_second_opencode_run_is_one_line | tests/agent_config.rs::test_a_second_opencode_run_is_one_line | runs the binary twice in a quiet HOME |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_all_good | src/bin/spawn.rs::test_all_good | PreflightTest; unit test of the bin |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_skip_setting_counts_as_accepted | src/bin/spawn.rs::test_skip_setting_counts_as_accepted | PreflightTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_failures_name_the_fix | src/bin/spawn.rs::test_failures_name_the_fix | PreflightTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_other_modes_skip_bypass_check | src/bin/spawn.rs::test_other_modes_skip_bypass_check | PreflightTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_clean_env | src/bin/spawn.rs::test_clean_env | CommandTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_names | src/bin/spawn.rs::test_names | CommandTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_command_quotes_everything | src/bin/spawn.rs::test_command_quotes_everything | CommandTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_ok | src/bin/spawn.rs::test_classify_ok | ClassifyTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_wait | src/bin/spawn.rs::test_classify_wait | ClassifyTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_dialogs | src/bin/spawn.rs::test_classify_dialogs | ClassifyTest |
| claude/skills/ps_spawn/scripts/test_spawn.py::test_prompt_words_are_not_a_dialog | src/bin/spawn.rs::test_classify_prompt_words_are_not_a_dialog | ClassifyTest |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_scenario | src/bin/route.rs::test_scenario | RouteTest; unit test of the bin |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_cap_defaults_the_cheapest | src/bin/route.rs::test_cap_defaults_the_cheapest | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_kill_makes_stop_tasks_do | src/bin/route.rs::test_kill_makes_stop_tasks_do | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_nothing_to_ask_means_mic_drop | src/bin/route.rs::test_nothing_to_ask_means_mic_drop | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_wait_on_do_reruns | src/bin/route.rs::test_wait_on_do_reruns | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_answers_unlock | src/bin/route.rs::test_answers_unlock | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_dependent_question_waits | src/bin/route.rs::test_dependent_question_waits | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_wait_chain_is_order_free | src/bin/route.rs::test_wait_chain_is_order_free | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_cycle_is_bad_input | src/bin/route.rs::test_cycle_is_bad_input | also checks the error messages |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_semicolon_in_text_keeps_the_handoff_line | src/bin/route.rs::test_semicolon_in_text_keeps_the_handoff_line | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_skip_closes_a_row | src/bin/route.rs::test_skip_closes_a_row | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_bad_input | src/bin/route.rs::test_bad_input | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_row_without_id_gets_a_default_id | src/bin/route.rs::test_row_without_id_gets_a_default_id | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_row_without_text_is_dropped_and_named | src/bin/route.rs::test_row_without_text_is_dropped_and_named | `--json` also lists the dropped rows (`test_cli_json_lists_the_dropped_rows`) |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_wrong_types_stay_bad_input | tests/skill_bins.rs::test_wrong_types_stay_bad_input | runs the route binary with the rows on stdin; checks exit 2 and the bad-input line, not the message (the Rust message for `id: 5` is `id missing`, Python's is `id must be a string`) |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_cli | src/bin/route.rs::test_cli | |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_whatsleft_links | src/bin/route.rs::test_whatsleft_links | SkillLinkTest; reads SKILL.md |
| claude/skills/ps_whatsleft/scripts/test_route.py::test_askme_links | src/bin/route.rs::test_askme_links | SkillLinkTest; reads SKILL.md |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_foreground_unmasked_check_after_edit_passes | tests/skill_bins.rs::test_foreground_unmasked_check_after_edit_passes | FreshnessTest; runs `whatsleft_scan --session` on a transcript in a temp git repo (Python called `check_freshness`) |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_foreground_pipe_with_pipefail_passes | tests/skill_bins.rs::test_foreground_pipe_with_pipefail_passes | passes because `masked_exit` honors `pipefail` |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_check_chained_with_and_is_not_masked | tests/skill_bins.rs::test_check_chained_with_and_is_not_masked | passes because `masked_exit` stops at the first `&&` |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_failed_foreground_check_still_fails | tests/skill_bins.rs::test_failed_foreground_check_still_fails | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_background_check_without_notice_is_still_running | tests/skill_bins.rs::test_background_check_without_notice_is_still_running | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_background_check_completed_exit_0_passed | tests/skill_bins.rs::test_background_check_completed_exit_0_passed | the notice is read: completed with exit code 0 and no hidden exit code is passed |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_background_check_failed_or_non_zero_exit_failed | tests/skill_bins.rs::test_background_check_failed_or_non_zero_exit_failed | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_masked_exit_code_is_named | tests/skill_bins.rs::test_masked_exit_code_is_named | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_masked_background_check_with_exit_0_notice_is_hidden | tests/skill_bins.rs::test_masked_background_check_with_exit_0_notice_is_hidden | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_bash_in_place_edit_counts | tests/skill_bins.rs::test_bash_in_place_edit_counts | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_bash_reads_are_not_edits | tests/skill_bins.rs::test_bash_reads_are_not_edits | `IN_PLACE_EDIT` matches `sed -i` and `perl -pi` only; reads, `grep -i` and `perl -M` are not edits |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_in_place_edit_outside_the_repo_is_not_an_edit | tests/skill_bins.rs::test_in_place_edit_outside_the_repo_is_not_an_edit | an in-place edit counts only when the Bash call ran in the repo or names its path |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_in_place_edit_without_any_check | tests/skill_bins.rs::test_in_place_edit_without_any_check | |
| claude/skills/ps_whatsleft/scripts/test_whatsleft_scan.py::test_replay_2026_10_14 | tests/skill_bins.rs::test_replay_2026_10_14 | |
| claude/skills/ps_harness-optimize/scripts/test_harness_stats.py::test_spawns_and_agents_table | tests/skill_bins.rs::test_spawns_and_agents_table | runs `collect`, `agents` and `report` on a temp HOME (Python set the module paths) |
| claude/skills/ps_harness-optimize/scripts/test_harness_stats.py::test_scan_survives_null_token_fields | tests/skill_bins.rs::test_scan_survives_null_token_fields | `scan` is private, so the numbers are read from the `collect` row |
