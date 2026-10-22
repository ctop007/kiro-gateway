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
Kiro API error enhancement and user-friendly message formatting.

This module provides a centralized system for enhancing cryptic Kiro API errors
with clear, actionable, user-friendly messages.

Architecture:
- KiroErrorReason: Enum of known error reasons from Kiro API
- KiroErrorInfo: Structured information about an enhanced error
- enhance_kiro_error(): Analyzes error JSON and returns enhanced message
- build_openai_error_body() / build_anthropic_error_body(): Client-facing error
  bodies whose error types match what each native API returns for the same
  condition, so clients can react (e.g. compact the conversation on overflow)

Example:
    >>> error_json = {"message": "Input is too long.", "reason": "CONTENT_LENGTH_EXCEEDS_THRESHOLD"}
    >>> error_info = enhance_kiro_error(error_json)
    >>> print(error_info.user_message)
    "Model context limit reached. Conversation size exceeds model capacity."
"""

from dataclasses import dataclass
from enum import Enum
from typing import Any, Dict, Optional

from loguru import logger


@dataclass
class KiroErrorInfo:
    """
    Structured information about a Kiro API error.
    
    Contains both the enhanced user-friendly message and the original
    error details for logging and debugging.
    
    Attributes:
        reason: Error reason code from Kiro API (as string, e.g. "CONTENT_LENGTH_EXCEEDS_THRESHOLD")
        user_message: Enhanced, user-friendly message for end users
        original_message: Original message from Kiro API (for logging)
    """
    reason: str
    user_message: str
    original_message: str


def enhance_kiro_error(error_json: Dict[str, Any]) -> KiroErrorInfo:
    """
    Enhances Kiro API error with user-friendly message.
    
    Takes raw error JSON from Kiro API and returns structured information
    with enhanced, user-friendly messages that help users understand what
    went wrong without technical jargon.
    
    Args:
        error_json: Parsed JSON from Kiro API error response
                   Expected format: {"message": "...", "reason": "..."}
                   The "reason" field is optional.
    
    Returns:
        KiroErrorInfo with enhanced message and original details
    
    Example:
        >>> error_json = {"message": "Input is too long.", "reason": "CONTENT_LENGTH_EXCEEDS_THRESHOLD"}
        >>> error_info = enhance_kiro_error(error_json)
        >>> print(error_info.user_message)
        "Model context limit reached. Conversation size exceeds model capacity."
        >>> print(error_info.original_message)
        "Input is too long."
    
    Example (unknown error):
        >>> error_json = {"message": "Something went wrong.", "reason": "UNKNOWN_REASON"}
        >>> error_info = enhance_kiro_error(error_json)
        >>> print(error_info.user_message)
        "Something went wrong. (reason: UNKNOWN_REASON)"
    """
    # Extract original message and reason from Kiro API response
    # Handle None values explicitly (preserve empty strings)
    original_message = error_json.get("message")
    if original_message is None:
        original_message = "Unknown error"
    
    reason = error_json.get("reason")
    if reason is None:
        reason = "UNKNOWN"
    
    # Map known reasons to user-friendly messages
    if reason == "CONTENT_LENGTH_EXCEEDS_THRESHOLD":
        # Context limit exceeded - conversation is too long
        user_message = "Model context limit reached. Conversation size exceeds model capacity."
    
    elif reason == "MONTHLY_REQUEST_COUNT":
        # Monthly request limit exceeded - account quota exhausted
        user_message = "Monthly request limit exceeded. Account has reached its monthly quota."
    
    elif reason == "INVALID_MODEL_ID":
        # Invalid model name or subscription tier insufficient
        user_message = "Invalid model ID or insufficient subscription level to use it."

    elif original_message == "Improperly formed request." and reason in (None, "UNKNOWN", "null"):
        # Generic 400 error
        user_message = (
            "Kiro API rejected the request. If problem persists, open issue with info and attached debug logs at:"
            "https://github.com/jwadow/kiro-gateway/issues"
        )

    # Future error enhancements can be added here:
    # elif reason == "RATE_LIMIT_EXCEEDED":
    #     user_message = "Rate limit exceeded. Too many requests in a short time."
    # elif reason == "INVALID_MODEL":
    #     user_message = "Invalid model specified. The requested model is not available."
    
    else:
        # Unknown error or no enhancement available
        # Keep original message and append reason if present
        if "reason" in error_json and reason != "UNKNOWN":
            user_message = f"{original_message} (reason: {reason})"
        else:
            user_message = original_message
    
    return KiroErrorInfo(
        reason=reason,
        user_message=user_message,
        original_message=original_message
    )


# ==================================================================================================
# Client-facing error bodies
# ==================================================================================================

# Kiro reason for a request that exceeds the model's context window
CONTEXT_OVERFLOW_REASON = "CONTENT_LENGTH_EXCEEDS_THRESHOLD"

# Default error types for Kiro errors without a native equivalent
OPENAI_DEFAULT_ERROR_TYPE = "kiro_api_error"
ANTHROPIC_DEFAULT_ERROR_TYPE = "api_error"

# Native error shapes for context overflow. Clients key off these to trigger
# their own recovery (Claude Code compacts on "prompt is too long",
# OpenAI SDK users check code == "context_length_exceeded").
OPENAI_CONTEXT_OVERFLOW_TYPE = "invalid_request_error"
OPENAI_CONTEXT_OVERFLOW_CODE = "context_length_exceeded"
ANTHROPIC_CONTEXT_OVERFLOW_TYPE = "invalid_request_error"
ANTHROPIC_CONTEXT_OVERFLOW_PREFIX = "prompt is too long"


def build_openai_error_body(message: str, status_code: int, reason: Optional[str]) -> Dict[str, Any]:
    """
    Build an OpenAI-format error body for a failed Kiro request.

    Args:
        message: User-facing error message (usually KiroErrorInfo.user_message)
        status_code: HTTP status code returned by Kiro
        reason: Kiro error reason, or None if it could not be extracted

    Returns:
        {"error": {...}} body. Context overflow uses OpenAI's native
        invalid_request_error / context_length_exceeded; anything else keeps
        the gateway's kiro_api_error type with the HTTP status as code.

    Examples:
        >>> build_openai_error_body("x", 400, "CONTENT_LENGTH_EXCEEDS_THRESHOLD")["error"]["code"]
        'context_length_exceeded'
    """
    if reason == CONTEXT_OVERFLOW_REASON:
        return {
            "error": {
                "message": message,
                "type": OPENAI_CONTEXT_OVERFLOW_TYPE,
                "code": OPENAI_CONTEXT_OVERFLOW_CODE,
            }
        }
    return {
        "error": {
            "message": message,
            "type": OPENAI_DEFAULT_ERROR_TYPE,
            "code": status_code,
        }
    }


def build_anthropic_error_body(message: str, reason: Optional[str]) -> Dict[str, Any]:
    """
    Build an Anthropic-format error body for a failed Kiro request.

    Args:
        message: User-facing error message (usually KiroErrorInfo.user_message)
        reason: Kiro error reason, or None if it could not be extracted

    Returns:
        {"type": "error", "error": {...}} body. Context overflow uses the native
        invalid_request_error type and a message starting with "prompt is too
        long", the wording the real Anthropic API uses for this condition.

    Examples:
        >>> build_anthropic_error_body("x", "CONTENT_LENGTH_EXCEEDS_THRESHOLD")["error"]["type"]
        'invalid_request_error'
    """
    if reason == CONTEXT_OVERFLOW_REASON:
        if not message.lower().startswith(ANTHROPIC_CONTEXT_OVERFLOW_PREFIX):
            message = f"{ANTHROPIC_CONTEXT_OVERFLOW_PREFIX}: {message}"
        return {
            "type": "error",
            "error": {"type": ANTHROPIC_CONTEXT_OVERFLOW_TYPE, "message": message},
        }
    return {
        "type": "error",
        "error": {"type": ANTHROPIC_DEFAULT_ERROR_TYPE, "message": message},
    }
