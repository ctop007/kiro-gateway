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
Exception handlers for Kiro Gateway.

Contains functions for handling validation errors and other exceptions
in a JSON-serialization compatible format.
"""

import json
from typing import Any, Dict, List, Optional

from fastapi import Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from loguru import logger

from kiro.request_log import finish_request_record, start_request_record


# Model endpoints whose 422 responses go to the request log, by API surface.
# Kept in sync with LOGGED_ENDPOINTS in kiro/debug_middleware.py.
_REQUEST_LOG_API_BY_PATH: Dict[str, str] = {
    "/v1/chat/completions": "openai",
    "/v1/messages": "anthropic",
}


def sanitize_validation_errors(errors: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """
    Converts validation errors to JSON-serializable format.
    
    Pydantic may include bytes objects in the 'input' field, which
    are not JSON-serializable. This function converts them to strings.
    
    Args:
        errors: List of validation errors from Pydantic
    
    Returns:
        List of errors with bytes converted to strings
    """
    sanitized = []
    for error in errors:
        sanitized_error = {}
        for key, value in error.items():
            if isinstance(value, bytes):
                # Convert bytes to string
                sanitized_error[key] = value.decode("utf-8", errors="replace")
            elif isinstance(value, (list, tuple)):
                # Recursively process lists
                sanitized_error[key] = [
                    v.decode("utf-8", errors="replace") if isinstance(v, bytes) else v
                    for v in value
                ]
            else:
                sanitized_error[key] = value
        sanitized.append(sanitized_error)
    return sanitized


async def validation_exception_handler(request: Request, exc: RequestValidationError) -> JSONResponse:
    """
    Pydantic validation error handler.
    
    Logs error details and returns an informative response.
    Correctly handles bytes objects in errors by converting them to strings.
    Also flushes debug logs for validation errors when DEBUG_MODE is enabled.
    
    Args:
        request: FastAPI Request object
        exc: Validation exception from Pydantic
    
    Returns:
        JSONResponse with error details and status 422
    """
    body = await request.body()
    body_str = body.decode("utf-8", errors="replace")
    
    # Sanitize errors for JSON serialization
    sanitized_errors = sanitize_validation_errors(exc.errors())
    
    logger.error(f"Validation error (422): {sanitized_errors}")
    # Log body at DEBUG level to avoid cluttering console with potentially large payloads
    # logger.debug(f"Request body: {body_str[:500]}...")
    
    # Flush debug logs for validation errors
    # This is called AFTER middleware has initialized debug logging,
    # so all app logs during request processing will be captured
    try:
        from kiro.debug_logger import debug_logger
        if debug_logger:
            error_message = f"Validation error: {sanitized_errors}"
            debug_logger.flush_on_error(422, error_message)
    except ImportError:
        pass  # debug_logger not available
    
    _log_validation_failure(request.url.path, body, sanitized_errors)
    
    return JSONResponse(
        status_code=422,
        content={"detail": sanitized_errors, "body": body_str[:500]},
    )


def _log_validation_failure(path: str, body: bytes, errors: List[Dict[str, Any]]) -> None:
    """
    Write a request log record for a 422 on a model endpoint.

    The route never runs for these requests, so the model and stream flag are
    read from the raw body on a best-effort basis. Only the validation error
    locations and messages are kept, never the body content.

    Args:
        path: Request path
        body: Raw request body
        errors: Sanitized Pydantic validation errors
    """
    api = _REQUEST_LOG_API_BY_PATH.get(path)
    if api is None:
        return

    model: Optional[str] = None
    stream = False
    try:
        payload = json.loads(body) if body else None
    except (json.JSONDecodeError, UnicodeDecodeError):
        payload = None
    if isinstance(payload, dict):
        if isinstance(payload.get("model"), str):
            model = payload["model"]
        stream = payload.get("stream") is True

    summary = "; ".join(
        f"{'.'.join(str(part) for part in error.get('loc', ()))}: {error.get('msg', '')}"
        for error in errors
    )
    record = start_request_record(api=api, model=model, stream=stream)
    record.status = 422
    record.error = f"Validation error: {summary}" if summary else "Validation error"
    finish_request_record(record)
