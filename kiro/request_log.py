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
Structured per-request log.

When REQUEST_LOG_DIR is set, every /v1/chat/completions and /v1/messages
request appends exactly one JSON line to <dir>/requests-YYYY-MM.jsonl once the
response is over (returned, streamed to the end, failed, or the client left).

The log is metadata only: time, API surface, model, status, outcome, duration
and billed credits. Message content, headers and credentials are never written,
and the error summary is capped at ERROR_SUMMARY_MAX_CHARS.

Lifecycle:
    record = start_request_record(api="anthropic", model=..., stream=...)
    return await track_request(record, lambda: handle(..., record))

track_request() owns every exit of the route: it records the status and error
summary of returned responses and raised HTTPExceptions, and writes a 500 row
for anything unexpected. Streaming routes set record.stream_owned before
returning their StreamingResponse; their generator then fills status/outcome in
its own finally block (it alone knows about mid-stream errors and client
disconnects) and calls finish_request_record(). The streaming layer fills
record.credits, record.response_id and the refusal outcome along the way.

Writes open the month file, append one line and close it again under a
process-wide lock. Holding no file handle lets other programs (the desktop
app) delete old month files safely, and the lock keeps concurrent lines from
interleaving.
"""

import asyncio
import json
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, AsyncIterator, Awaitable, Callable, Dict, Optional

from fastapi import HTTPException
from loguru import logger
from starlette.responses import Response, StreamingResponse

from kiro.config import REQUEST_LOG_DIR


# Maximum length of the error summary stored per record. Upstream error
# messages can echo parts of the request, so keep only a short prefix.
ERROR_SUMMARY_MAX_CHARS = 200

# Allowed outcome values (see the gateway/request-log spec).
OUTCOME_OK = "ok"
OUTCOME_ERROR = "error"
OUTCOME_REFUSED = "refused"
OUTCOME_CLIENT_DISCONNECTED = "client_disconnected"

# Status written when an exit path forgot to set one. Counts as a failure.
_FALLBACK_STATUS = 500

_write_lock = threading.Lock()

# Warn once per process about write failures instead of once per request.
_write_failure_warned = False


def _now() -> datetime:
    """
    Return the current local time as a timezone-aware datetime.

    Kept as a separate function so tests can pin the clock.

    Returns:
        Current local time with its UTC offset
    """
    return datetime.now().astimezone()


@dataclass
class RequestRecord:
    """
    Metadata for one model request, filled in as the request progresses.

    Attributes:
        api: API surface, "openai" or "anthropic"
        model: Model name as requested by the client (None if unknown)
        stream: Whether the client asked for a streaming response
        started_at: Local, timezone-aware start time (decides the month file)
        status: HTTP status returned to the client
        outcome: Explicit outcome override (refused / client_disconnected / error)
        credits: Credits billed upstream for this request
        response_id: Response ID, same as req= in the [Credit] log line
        error: Error summary for failed requests
        finished: True once the record has been written (or deliberately dropped)
        stream_owned: True when a streaming generator took over finishing the record
    """

    api: str
    model: Optional[str]
    stream: bool
    started_at: datetime
    started_monotonic: float
    status: Optional[int] = None
    outcome: Optional[str] = None
    credits: float = 0.0
    response_id: Optional[str] = None
    error: Optional[str] = None
    finished: bool = field(default=False)
    stream_owned: bool = field(default=False)

    def mark_error(self, message: str) -> None:
        """
        Mark the request as failed with an error summary.

        Used when a stream fails after the 200 status line was already sent,
        so the HTTP status alone would make the request look successful.

        Args:
            message: Human-readable error description
        """
        self.outcome = OUTCOME_ERROR
        self.error = message


def start_request_record(api: str, model: Optional[str], stream: bool) -> RequestRecord:
    """
    Create a record at the start of a request.

    Always returns a record, even when logging is disabled, so call sites
    never need their own "is logging enabled" branches.

    Args:
        api: API surface, "openai" or "anthropic"
        model: Model name from the client request (None if unknown)
        stream: Whether the client asked for streaming

    Returns:
        A new, unfinished RequestRecord
    """
    return RequestRecord(
        api=api,
        model=model,
        stream=stream,
        started_at=_now(),
        started_monotonic=time.monotonic(),
    )


def request_log_file_for(directory: str, started_at: datetime) -> Path:
    """
    Build the month file path for a request start time.

    Args:
        directory: Request log directory
        started_at: Local start time of the request

    Returns:
        Path to requests-YYYY-MM.jsonl inside the directory
    """
    return Path(directory) / f"requests-{started_at:%Y-%m}.jsonl"


def _summarize_error(error: Optional[str]) -> Optional[str]:
    """
    Normalize an error message for storage.

    Collapses whitespace (so one record stays on one line) and truncates to
    ERROR_SUMMARY_MAX_CHARS.

    Args:
        error: Raw error message, or None

    Returns:
        Single-line, truncated summary, or None when there is nothing to say
    """
    if error is None:
        return None
    collapsed = " ".join(str(error).split())
    if not collapsed:
        return None
    return collapsed[:ERROR_SUMMARY_MAX_CHARS]


def _resolve_outcome(record: RequestRecord, status: int) -> str:
    """
    Decide the outcome value written for a record.

    An explicit outcome (refusal, disconnect, mid-stream error) wins; otherwise
    2xx means ok and anything else means error.

    Args:
        record: The record being finished
        status: Final HTTP status

    Returns:
        One of the OUTCOME_* values
    """
    if record.outcome:
        return record.outcome
    return OUTCOME_OK if 200 <= status < 300 else OUTCOME_ERROR


def _serialize(record: RequestRecord) -> Dict[str, Any]:
    """
    Convert a record into the JSON object written to disk.

    Args:
        record: Finished request record

    Returns:
        Dictionary with exactly the fields defined in the spec
    """
    status = record.status if record.status is not None else _FALLBACK_STATUS
    duration_ms = int((time.monotonic() - record.started_monotonic) * 1000)
    return {
        "ts": record.started_at.isoformat(timespec="milliseconds"),
        "id": record.response_id,
        "api": record.api,
        "model": record.model,
        "stream": record.stream,
        "status": status,
        "outcome": _resolve_outcome(record, status),
        "duration_ms": max(duration_ms, 0),
        "credits": record.credits,
        "error": _summarize_error(record.error),
    }


def finish_request_record(record: Optional[RequestRecord]) -> None:
    """
    Write the record as one JSON line, at most once. Never raises.

    Safe to call from several exit paths: only the first call writes.
    Write failures are logged once per process and otherwise ignored, so a
    broken log destination never affects the response.

    Args:
        record: Record to write, or None (no-op)
    """
    global _write_failure_warned

    if record is None or record.finished:
        return
    record.finished = True

    if not REQUEST_LOG_DIR:
        return

    line = json.dumps(_serialize(record), ensure_ascii=False) + "\n"
    path = request_log_file_for(REQUEST_LOG_DIR, record.started_at)

    try:
        with _write_lock:
            path.parent.mkdir(parents=True, exist_ok=True)
            with open(path, "a", encoding="utf-8") as handle:
                handle.write(line)
    except OSError as e:
        if not _write_failure_warned:
            _write_failure_warned = True
            logger.warning(
                f"Request log disabled for this run: cannot write to {path.parent} ({e}). "
                f"Check REQUEST_LOG_DIR points to a writable directory."
            )


def finish_streaming_request_record(
    record: RequestRecord,
    streaming_error: Optional[BaseException],
    client_disconnected: bool,
) -> None:
    """
    Fill the outcome of a streaming request and write its log record.

    Called from a route's stream generator in its finally block. The HTTP
    status is already 200 there, so mid-stream failures and client disconnects
    must be recorded explicitly.

    Args:
        record: Request log record owned by the stream
        streaming_error: Exception raised while streaming, if any
        client_disconnected: Whether the client went away mid-stream
    """
    record.status = 200
    if streaming_error is not None:
        record.mark_error(str(streaming_error) or type(streaming_error).__name__)
    elif client_disconnected:
        record.outcome = OUTCOME_CLIENT_DISCONNECTED
    finish_request_record(record)


def _error_from_body(body: bytes) -> Optional[str]:
    """
    Extract the human-readable error message from an error response body.

    Understands the OpenAI / Anthropic shape ({"error": {"message": ...}}) and
    FastAPI's ({"detail": ...}). Falls back to the raw text.

    Args:
        body: Raw response body

    Returns:
        Error message, or None for an empty body
    """
    text = body.decode("utf-8", errors="replace") if body else ""
    if not text.strip():
        return None
    try:
        payload = json.loads(text)
    except json.JSONDecodeError:
        return text
    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, dict) and error.get("message"):
            return str(error["message"])
        if isinstance(error, str):
            return error
        if "detail" in payload:
            return str(payload["detail"])
    return text


def _finish_after_stream(record: RequestRecord, iterator: AsyncIterator[Any]) -> AsyncIterator[Any]:
    """
    Wrap a streaming body that does not fill the record itself.

    Used for streaming responses produced outside the main pipeline (native
    web_search), so they still leave one row when the stream ends.

    Args:
        record: Record to finish when the stream ends
        iterator: Original body iterator

    Returns:
        Async iterator yielding the same chunks
    """
    async def wrapper() -> AsyncIterator[Any]:
        completed = False
        try:
            async for chunk in iterator:
                yield chunk
            completed = True
        except (GeneratorExit, asyncio.CancelledError):
            record.outcome = OUTCOME_CLIENT_DISCONNECTED
            raise
        finally:
            if not completed and record.outcome != OUTCOME_CLIENT_DISCONNECTED:
                record.mark_error("Stream ended with an error")
            finish_request_record(record)

    return wrapper()


def _record_response(record: RequestRecord, response: Response) -> Response:
    """
    Record status and error summary of a response returned by a route.

    Args:
        record: Request record
        response: Response the route is about to return

    Returns:
        The same response (streaming bodies may be wrapped to finish the record)
    """
    record.status = response.status_code
    if isinstance(response, StreamingResponse):
        if not record.stream_owned:
            record.stream_owned = True
            response.body_iterator = _finish_after_stream(record, response.body_iterator)
        return response
    if response.status_code >= 400:
        record.error = _error_from_body(response.body)
    return response


async def track_request(
    record: RequestRecord,
    handler: Callable[[], Awaitable[Response]],
) -> Response:
    """
    Run a route handler and make sure its request leaves exactly one record.

    Returned responses and raised HTTPExceptions are recorded with their real
    status and error summary. Any other exception propagates unchanged and is
    recorded as a 500 (the status FastAPI returns for it). Streaming responses
    are finished by their generator instead.

    Args:
        record: Record created at the start of the route
        handler: Coroutine factory running the actual route logic

    Returns:
        The handler's response

    Raises:
        HTTPException: Re-raised from the handler after recording it
    """
    try:
        return _record_response(record, await handler())
    except HTTPException as e:
        record.status = e.status_code
        record.error = str(e.detail)
        raise
    finally:
        if record.status is None:
            # Unexpected exception: FastAPI turns it into a plain 500
            record.status = _FALLBACK_STATUS
            record.error = record.error or "Internal server error"
        if not record.stream_owned:
            finish_request_record(record)
