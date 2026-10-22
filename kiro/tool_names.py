# -*- coding: utf-8 -*-

# Kiro Gateway
# https://github.com/jwadow/kiro-gateway
# Copyright (C) 2025 Jwadow
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
# GNU Affero General Public License for more details.
#
# You should have received a copy of the GNU Affero General Public License
# along with this program. If not, see <https://www.gnu.org/licenses/>.

"""
Tool name shortening for Kiro's 64-character tool name limit.

Kiro rejects tool names longer than 64 characters, but MCP clients routinely
produce longer ones (Claude Code: "mcp__plugin_<plugin>_<server>__<tool>").
Instead of rejecting the whole request, long names are shortened on the way to
Kiro and restored on the way back, so clients never see the short names.

Shortening is a pure, deterministic function of the original name. That lets
the converters (request side) and the routes/streaming (response side) derive
the same mapping independently from the request's tool list, with no shared
or global state between requests.

Shape of a shortened name (always exactly 64 characters):
- MCP-style names keep the real tool name and compress the server prefix:
  "mcp__plugin_chrome-devtools-mcp_chrome-devtools__performance_analyze_insight"
  -> "mcp__plugin_chrome-devtool_<hash8>__performance_analyze_insight"
- Other names are truncated: "<first 55 chars>_<hash8>"
"""

import hashlib
from typing import Any, Dict, Iterable, Optional

from loguru import logger

# Kiro API limit for toolSpecification.name
KIRO_MAX_TOOL_NAME_LENGTH = 64

# Separator between MCP server prefix and the tool's own name
MCP_NAME_SEPARATOR = "__"

# Hex digits of the hash that keeps shortened names unique
TOOL_NAME_HASH_LENGTH = 8

# Minimum characters of the server prefix worth keeping; below this the
# tool's own name is too long and plain truncation is used instead
MIN_KEPT_PREFIX_LENGTH = 3


def _name_hash(name: str) -> str:
    """
    Short stable hash of a full tool name.

    Args:
        name: Original tool name

    Returns:
        TOOL_NAME_HASH_LENGTH lowercase hex characters
    """
    return hashlib.sha256(name.encode("utf-8")).hexdigest()[:TOOL_NAME_HASH_LENGTH]


def shorten_tool_name(name: str, max_length: int = KIRO_MAX_TOOL_NAME_LENGTH) -> str:
    """
    Shorten a tool name to fit Kiro's limit, keeping it readable and unique.

    Names within the limit are returned unchanged. For MCP-style names the part
    after the last "__" (the tool's own name, which models rely on to pick a
    tool) is kept intact and the server prefix is compressed. The hash of the
    full original name keeps different long names from colliding.

    Args:
        name: Original tool name
        max_length: Maximum allowed length

    Returns:
        The original name, or a shortened name of exactly max_length characters

    Examples:
        >>> shorten_tool_name("read_file")
        'read_file'
        >>> len(shorten_tool_name("mcp__" + "x" * 40 + "__performance_analyze_insight"))
        64
    """
    if len(name) <= max_length:
        return name

    digest = _name_hash(name)
    prefix, separator, tool = name.rpartition(MCP_NAME_SEPARATOR)
    if separator and prefix and tool:
        # prefix[:kept] + "_" + digest + "__" + tool
        kept = max_length - len(tool) - len(MCP_NAME_SEPARATOR) - TOOL_NAME_HASH_LENGTH - 1
        if kept >= MIN_KEPT_PREFIX_LENGTH:
            return f"{prefix[:kept]}_{digest}{MCP_NAME_SEPARATOR}{tool}"

    return f"{name[:max_length - TOOL_NAME_HASH_LENGTH - 1]}_{digest}"


class ToolNameMap:
    """
    Per-request mapping between client tool names and names sent to Kiro.

    Build one per request from the request's tool names. Names that fit the
    limit map to themselves and are not stored, so an empty map means nothing
    was shortened.

    Example:
        >>> name_map = ToolNameMap.from_names(["read_file", "mcp__" + "x" * 70 + "__click"])
        >>> short = name_map.to_kiro("mcp__" + "x" * 70 + "__click")
        >>> name_map.to_client(short) == "mcp__" + "x" * 70 + "__click"
        True
    """

    def __init__(self) -> None:
        """Initializes an empty mapping."""
        self._to_kiro: Dict[str, str] = {}
        self._to_client: Dict[str, str] = {}
        self._known_names: set = set()

    @classmethod
    def from_names(cls, names: Iterable[Optional[str]]) -> "ToolNameMap":
        """
        Build a mapping for a request's tool names.

        Args:
            names: Tool names as sent by the client (None/empty entries are ignored)

        Returns:
            ToolNameMap with every over-long name registered

        Raises:
            ValueError: If two different names shorten to the same name
        """
        name_map = cls()
        valid_names = [name for name in names if name]
        name_map._known_names.update(valid_names)
        for name in valid_names:
            name_map.to_kiro(name)
        return name_map

    def __bool__(self) -> bool:
        """Whether any name was shortened."""
        return bool(self._to_kiro)

    def __len__(self) -> int:
        """Number of shortened names."""
        return len(self._to_kiro)

    def to_kiro(self, name: str) -> str:
        """
        Name to send to Kiro for a client tool name, registering it if shortened.

        Args:
            name: Client tool name

        Returns:
            Shortened name if over the limit, otherwise the name itself

        Raises:
            ValueError: If the shortened name collides with a different tool name
        """
        if not name or len(name) <= KIRO_MAX_TOOL_NAME_LENGTH:
            return name

        cached = self._to_kiro.get(name)
        if cached is not None:
            return cached

        short = shorten_tool_name(name)
        owner = self._to_client.get(short)
        if (owner is not None and owner != name) or short in self._known_names:
            raise ValueError(
                f"Tool name '{name}' ({len(name)} characters) exceeds the Kiro API limit of "
                f"{KIRO_MAX_TOOL_NAME_LENGTH} characters and could not be shortened without "
                f"clashing with another tool name. Rename one of the tools."
            )

        self._to_kiro[name] = short
        self._to_client[short] = name
        logger.debug(f"Tool name shortened for Kiro: '{name}' -> '{short}'")
        return short

    def to_client(self, name: str) -> str:
        """
        Original client name for a name returned by Kiro.

        Args:
            name: Tool name from a Kiro response

        Returns:
            The original long name if it was shortened, otherwise the name itself
        """
        return self._to_client.get(name, name)

    def restore_tool_call(self, tool_call: Dict[str, Any]) -> Dict[str, Any]:
        """
        Restore the client name inside a parsed tool call, in place.

        Handles both shapes used by the parsers: OpenAI-style
        {"function": {"name": ...}} and flat {"name": ...}.

        Args:
            tool_call: Tool call dict from the stream parser

        Returns:
            The same dict, for convenient chaining
        """
        if not self._to_client or not isinstance(tool_call, dict):
            return tool_call

        function = tool_call.get("function")
        if isinstance(function, dict) and isinstance(function.get("name"), str):
            function["name"] = self.to_client(function["name"])
        if isinstance(tool_call.get("name"), str):
            tool_call["name"] = self.to_client(tool_call["name"])
        return tool_call
