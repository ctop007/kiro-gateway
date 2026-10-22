# -*- coding: utf-8 -*-

"""
Unit tests for kiro/tool_names.py - shortening tool names over Kiro's 64-character limit.
"""

from unittest.mock import patch

import pytest

from kiro.tool_names import (
    KIRO_MAX_TOOL_NAME_LENGTH,
    ToolNameMap,
    shorten_tool_name,
)

# Real tool names from Claude Code's chrome-devtools plugin (65-76 characters)
CHROME_DEVTOOLS_PREFIX = "mcp__plugin_chrome-devtools-mcp_chrome-devtools__"
CHROME_DEVTOOLS_TOOLS = [
    "get_console_message",
    "get_network_request",
    "lighthouse_audit",
    "list_console_messages",
    "list_network_requests",
    "performance_analyze_insight",
    "performance_start_trace",
    "performance_stop_trace",
    "screencast_start",
    "take_heapsnapshot",
]
CHROME_DEVTOOLS_NAMES = [CHROME_DEVTOOLS_PREFIX + tool for tool in CHROME_DEVTOOLS_TOOLS]


class TestShortenToolName:
    """Tests for shorten_tool_name()."""

    @pytest.mark.parametrize("name", ["read_file", "", "a" * 64, CHROME_DEVTOOLS_PREFIX + "click"])
    def test_names_within_limit_are_unchanged(self, name):
        """
        What it does: Names of 64 characters or fewer.
        Purpose: Shortening never touches valid names (including the exact boundary).
        """
        assert shorten_tool_name(name) == name

    def test_65_characters_is_shortened_to_exactly_64(self):
        """
        What it does: Name one character over the limit.
        Purpose: Boundary condition just above the limit.
        """
        result = shorten_tool_name("a" * 65)
        assert len(result) == KIRO_MAX_TOOL_NAME_LENGTH

    @pytest.mark.parametrize("name", CHROME_DEVTOOLS_NAMES)
    def test_real_mcp_names_keep_the_tool_name(self, name):
        """
        What it does: Shortens every real over-long chrome-devtools tool name.
        Purpose: Result fits the limit and still ends with the tool's own name,
                 which is what the model uses to pick a tool.
        """
        result = shorten_tool_name(name)
        tool = name.rsplit("__", 1)[1]

        assert len(result) == KIRO_MAX_TOOL_NAME_LENGTH
        assert result.endswith("__" + tool)
        assert result.startswith("mcp__plugin_chrome")

    def test_real_mcp_names_are_all_distinct(self):
        """
        What it does: Shortens all chrome-devtools names together.
        Purpose: Similar names (performance_start_trace / performance_stop_trace) stay distinct.
        """
        shortened = [shorten_tool_name(name) for name in CHROME_DEVTOOLS_NAMES]
        assert len(set(shortened)) == len(shortened)

    def test_same_tool_under_different_servers_is_distinct(self):
        """
        What it does: Same tool name with prefixes that only differ after the kept part.
        Purpose: The hash disambiguates prefixes that truncate identically.
        """
        a = shorten_tool_name("mcp__" + "x" * 40 + "_server_one__performance_analyze_insight")
        b = shorten_tool_name("mcp__" + "x" * 40 + "_server_two__performance_analyze_insight")
        assert a != b

    def test_is_deterministic(self):
        """
        What it does: Shortens the same name twice.
        Purpose: Request and response sides derive the same name independently.
        """
        name = CHROME_DEVTOOLS_NAMES[5]
        assert shorten_tool_name(name) == shorten_tool_name(name)

    def test_non_mcp_name_is_truncated_with_hash(self):
        """
        What it does: Long name without the "__" separator.
        Purpose: Falls back to truncation plus hash.
        """
        name = "get_authenticated_user_profile_data_with_extended_information_about_it"
        result = shorten_tool_name(name)

        assert len(result) == KIRO_MAX_TOOL_NAME_LENGTH
        assert result.startswith(name[:55] + "_")

    def test_tool_part_too_long_falls_back_to_truncation(self):
        """
        What it does: MCP-style name whose own tool part is nearly 64 characters.
        Purpose: No room for a prefix, so plain truncation is used.
        """
        name = "mcp__server__" + "t" * 60
        result = shorten_tool_name(name)

        assert len(result) == KIRO_MAX_TOOL_NAME_LENGTH
        assert not result.endswith("__" + "t" * 60)

    @pytest.mark.parametrize("name", ["__" + "t" * 70, "p" * 70 + "__"])
    def test_separator_at_edges_falls_back_to_truncation(self, name):
        """
        What it does: "__" at the very start or end.
        Purpose: Empty prefix or tool part never produces a malformed name.
        """
        assert len(shorten_tool_name(name)) == KIRO_MAX_TOOL_NAME_LENGTH

    def test_custom_max_length(self):
        """
        What it does: Uses a smaller limit.
        Purpose: Limit is a parameter, not hardcoded in the algorithm.
        """
        assert len(shorten_tool_name("mcp__server_name__do_thing", max_length=20)) == 20

    def test_non_ascii_names(self):
        """
        What it does: Long name with non-ASCII characters.
        Purpose: Hashing and slicing work on any Unicode input.
        """
        result = shorten_tool_name("инструмент_" * 10)
        assert len(result) == KIRO_MAX_TOOL_NAME_LENGTH


class TestToolNameMap:
    """Tests for ToolNameMap."""

    def test_empty_when_nothing_is_shortened(self):
        """
        What it does: Builds a map from short names only.
        Purpose: Empty map signals that no restoring is needed.
        """
        name_map = ToolNameMap.from_names(["read_file", "write_file"])

        assert not name_map
        assert len(name_map) == 0
        assert name_map.to_kiro("read_file") == "read_file"

    def test_round_trip_for_long_names(self):
        """
        What it does: Shortens and restores every chrome-devtools name.
        Purpose: Clients always get back the exact original name.
        """
        name_map = ToolNameMap.from_names(CHROME_DEVTOOLS_NAMES)

        assert len(name_map) == len(CHROME_DEVTOOLS_NAMES)
        for name in CHROME_DEVTOOLS_NAMES:
            short = name_map.to_kiro(name)
            assert len(short) <= KIRO_MAX_TOOL_NAME_LENGTH
            assert name_map.to_client(short) == name

    def test_unknown_names_pass_through(self):
        """
        What it does: Restores a name that was never shortened.
        Purpose: Short names and hallucinated names are returned unchanged.
        """
        name_map = ToolNameMap.from_names(CHROME_DEVTOOLS_NAMES)
        assert name_map.to_client("read_file") == "read_file"
        assert name_map.to_client("not_a_tool") == "not_a_tool"

    def test_none_and_empty_names_are_ignored(self):
        """
        What it does: Tool list containing None and "".
        Purpose: Malformed tool entries do not crash map building.
        """
        name_map = ToolNameMap.from_names([None, "", CHROME_DEVTOOLS_NAMES[0]])
        assert len(name_map) == 1

    def test_maps_are_independent(self):
        """
        What it does: Two maps for two requests.
        Purpose: No shared state between requests.
        """
        first = ToolNameMap.from_names(CHROME_DEVTOOLS_NAMES[:1])
        second = ToolNameMap.from_names([])

        assert first
        assert not second
        assert second.to_client(first.to_kiro(CHROME_DEVTOOLS_NAMES[0])) != CHROME_DEVTOOLS_NAMES[0]

    def test_to_kiro_registers_names_not_in_the_tool_list(self):
        """
        What it does: Shortens a long name seen only in history.
        Purpose: Old tool uses for removed tools still fit the limit and can be restored.
        """
        name_map = ToolNameMap.from_names([])
        short = name_map.to_kiro(CHROME_DEVTOOLS_NAMES[0])

        assert len(short) == KIRO_MAX_TOOL_NAME_LENGTH
        assert name_map.to_client(short) == CHROME_DEVTOOLS_NAMES[0]

    def test_collision_between_long_names_raises(self):
        """
        What it does: Forces two different names to shorten identically.
        Purpose: Ambiguous mappings are rejected with an actionable message.
        """
        with patch("kiro.tool_names.shorten_tool_name", return_value="same_short_name"):
            with pytest.raises(ValueError, match="Rename one of the tools"):
                ToolNameMap.from_names(["a" * 70, "b" * 70])

    def test_collision_with_existing_short_name_raises(self):
        """
        What it does: A shortened name equals another tool's real name.
        Purpose: Responses could not be restored unambiguously.
        """
        long_name = "a" * 70
        clash = shorten_tool_name(long_name)
        with pytest.raises(ValueError, match="clashing"):
            ToolNameMap.from_names([clash, long_name])

    def test_duplicate_long_names_are_fine(self):
        """
        What it does: The same long name listed twice.
        Purpose: Duplicates map to one short name, not a collision.
        """
        name_map = ToolNameMap.from_names([CHROME_DEVTOOLS_NAMES[0]] * 2)
        assert len(name_map) == 1


class TestRestoreToolCall:
    """Tests for ToolNameMap.restore_tool_call()."""

    @pytest.fixture
    def name_map(self):
        return ToolNameMap.from_names(CHROME_DEVTOOLS_NAMES)

    def test_restores_openai_shape(self, name_map):
        """
        What it does: Tool call with a nested "function" dict.
        Purpose: Shape produced by the stream parser for both APIs.
        """
        short = name_map.to_kiro(CHROME_DEVTOOLS_NAMES[0])
        call = {"id": "c1", "type": "function", "function": {"name": short, "arguments": "{}"}}

        result = name_map.restore_tool_call(call)

        assert result is call
        assert call["function"]["name"] == CHROME_DEVTOOLS_NAMES[0]
        assert call["function"]["arguments"] == "{}"

    def test_restores_flat_shape(self, name_map):
        """
        What it does: Tool call with a top-level "name".
        Purpose: Anthropic-style tool blocks are handled too.
        """
        short = name_map.to_kiro(CHROME_DEVTOOLS_NAMES[1])
        call = {"id": "t1", "name": short, "input": {}}

        name_map.restore_tool_call(call)

        assert call["name"] == CHROME_DEVTOOLS_NAMES[1]

    @pytest.mark.parametrize("call", [
        {"function": None},
        {"function": {"name": None}},
        {"name": 123},
        {},
        "not-a-dict",
        None,
    ])
    def test_malformed_calls_are_left_alone(self, name_map, call):
        """
        What it does: Malformed tool calls.
        Purpose: Restoring never raises on unexpected parser output.
        """
        assert name_map.restore_tool_call(call) == call

    def test_empty_map_is_a_no_op(self):
        """
        What it does: Restores with an empty map.
        Purpose: Requests without long names are unaffected.
        """
        call = {"function": {"name": "read_file"}}
        assert ToolNameMap().restore_tool_call(call) == {"function": {"name": "read_file"}}
